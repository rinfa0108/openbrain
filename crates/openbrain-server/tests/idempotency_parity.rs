use openbrain_embed::NoopEmbeddingProvider;
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::{hash_token, PgStore};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

async fn setup_pool() -> Option<PgPool> {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "skipping postgres-backed tests: set DATABASE_URL (example: postgres://postgres:postgres@localhost:5432/postgres)"
            );
            return None;
        }
    };

    let store = PgStore::connect(&database_url)
        .await
        .expect("connect postgres");
    let pool = store.pool().clone();

    let migrations_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let migrator = sqlx::migrate::Migrator::new(migrations_dir)
        .await
        .expect("load migrations");
    migrator.run(&pool).await.expect("run migrations");

    Some(pool)
}

struct TestServer {
    base: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl TestServer {
    async fn spawn() -> Option<Self> {
        let pool = setup_pool().await?;

        let store = PgStore::from_pool_with_embedder(pool, Arc::new(NoopEmbeddingProvider));
        let llm = AnthropicClient::from_env();
        let app = build_router(AppState { store, llm });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");

        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("server run");
        });

        Some(Self {
            base: format!("http://{}", addr),
            shutdown_tx: Some(tx),
            task: Some(task),
        })
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

async fn create_workspace_token(pool: &PgPool, role: &str) -> (String, String) {
    let workspace_id = format!("ws-{}", uuid::Uuid::new_v4());
    let token = format!("ob_test_{}", uuid::Uuid::new_v4());
    let token_hash = hash_token(&token);
    let identity_id = uuid::Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO ob_workspaces (id, name) VALUES ($1, $2)")
        .bind(&workspace_id)
        .bind(&workspace_id)
        .execute(pool)
        .await
        .expect("insert workspace");

    sqlx::query("INSERT INTO ob_identities (id, display_name) VALUES ($1, $2)")
        .bind(&identity_id)
        .bind("test-identity")
        .execute(pool)
        .await
        .expect("insert identity");

    sqlx::query(
        "INSERT INTO ob_tokens (token_hash, identity_id, workspace_id, role, label) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&token_hash)
    .bind(&identity_id)
    .bind(&workspace_id)
    .bind(role)
    .bind("test")
    .execute(pool)
    .await
    .expect("insert token");

    (token, workspace_id)
}

#[tokio::test]
async fn parity_http_and_mcp_write_receipts() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };
    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;
    let base = server.base.clone();
    let client = reqwest::Client::new();
    let key = format!("idem-{}", uuid::Uuid::new_v4());

    let body = json!({
        "idempotency_key": key,
        "objects": [{
            "type":"claim",
            "id": format!("obj-{}", uuid::Uuid::new_v4()),
            "scope": workspace_id,
            "status":"draft",
            "spec_version":"0.1",
            "tags":[],
            "data":{"subject":"a","predicate":"b","object":"c","polarity":"pos"},
            "provenance":{"actor":"tester"}
        }]
    });

    let first_http = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .expect("http write 1")
        .json::<Value>()
        .await
        .expect("http write 1 json");

    let second_http = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .expect("http write 2")
        .json::<Value>()
        .await
        .expect("http write 2 json");

    assert!(!first_http
        .get("replayed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false));
    assert!(second_http
        .get("replayed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false));
    assert_eq!(
        first_http.get("receipt_hash"),
        second_http.get("receipt_hash"),
        "HTTP replay must return same receipt hash"
    );

    let database_url = std::env::var("DATABASE_URL").expect("db url");
    let mut child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
        .arg("mcp")
        .env("DATABASE_URL", database_url)
        .env("OPENBRAIN_EMBED_PROVIDER", "noop")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openbrain mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");

    writeln!(
        stdin,
        "{}",
        json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"protocolVersion":"0","auth_token": token}
        })
    )
    .expect("write initialize");

    let mcp_key = format!("idem-{}", uuid::Uuid::new_v4());
    let mcp_object_id = format!("obj-{}", uuid::Uuid::new_v4());
    let mcp_write = json!({
        "jsonrpc":"2.0",
        "id":2,
        "method":"tools/call",
        "params":{
            "name":"openbrain.write",
            "arguments":{
                "idempotency_key": mcp_key,
                "objects":[{
                    "type":"claim",
                    "id": mcp_object_id,
                    "scope": workspace_id,
                    "status":"draft",
                    "spec_version":"0.1",
                    "tags":[],
                    "data":{"subject":"m","predicate":"n","object":"o","polarity":"pos"},
                    "provenance":{"actor":"tester"}
                }]
            }
        }
    });
    writeln!(stdin, "{mcp_write}").expect("write mcp 1");
    writeln!(stdin, "{mcp_write}").expect("write mcp 2");
    drop(stdin);

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    let deadline = Duration::from_secs(8);
    let start = std::time::Instant::now();
    let mut tool_results = Vec::new();

    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        let Ok(line) = rx.recv_timeout(remaining) else {
            break;
        };
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            tool_results.push(v);
            if tool_results.len() == 2 {
                break;
            }
        }
    }

    assert_eq!(tool_results.len(), 2, "expected two MCP write responses");
    let first = tool_results[0].get("result").expect("result 1");
    let second = tool_results[1].get("result").expect("result 2");
    assert!(!first
        .get("replayed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false));
    assert!(second
        .get("replayed")
        .and_then(|v| v.as_bool())
        .unwrap_or(false));
    assert_eq!(first.get("receipt_hash"), second.get("receipt_hash"));

    let _ = child.kill();
    let _ = child.wait();
    server.shutdown().await;
}
