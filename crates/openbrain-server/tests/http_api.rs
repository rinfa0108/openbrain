use openbrain_embed::NoopEmbeddingProvider;
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::{hash_token, PgStore};
use serde_json::json;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
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
async fn http_ping_write_read_and_structured_search() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    // ping
    let ping = client
        .post(format!("{}/v1/ping", base))
        .send()
        .await
        .expect("ping")
        .json::<serde_json::Value>()
        .await
        .expect("ping json");
    assert_eq!(ping.get("ok").and_then(|v| v.as_bool()), Some(true));

    // write
    let scope = workspace_id;
    let id1 = format!("obj-{}", uuid::Uuid::new_v4());
    let id2 = format!("obj-{}", uuid::Uuid::new_v4());

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": id1,
                "scope": scope,
                "status": "draft",
                "spec_version": "0.1",
                "tags": ["t1"],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            },
            {
                "type": "claim",
                "id": id2,
                "scope": scope,
                "status": "candidate",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"x","predicate":"y","object":"z","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");
    assert_eq!(write.get("ok").and_then(|v| v.as_bool()), Some(true));

    // read (scoped)
    let read = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({"scope": write_body["objects"][0]["scope"].clone(), "refs": [write_body["objects"][0]["id"].clone()]}))
        .send()
        .await
        .expect("read")
        .json::<serde_json::Value>()
        .await
        .expect("read json");
    assert_eq!(read.get("ok").and_then(|v| v.as_bool()), Some(true));

    // structured search
    let structured = client
        .post(format!("{}/v1/search/structured", base))
        .bearer_auth(&token)
        .json(&json!({
            "scope": write_body["objects"][0]["scope"].clone(),
            "where_expr": "type == \"claim\" AND status == \"draft\"",
            "limit": 50,
            "offset": 0
        }))
        .send()
        .await
        .expect("structured")
        .json::<serde_json::Value>()
        .await
        .expect("structured json");

    assert_eq!(structured.get("ok").and_then(|v| v.as_bool()), Some(true));
    let results = structured
        .get("results")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(results.len(), 1);

    server.shutdown().await;
}

#[tokio::test]
async fn http_requires_auth() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write = client
        .post(format!("{}/v1/write", base))
        .json(&json!({"objects": []}))
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");

    assert_eq!(
        write
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_UNAUTHENTICATED")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_reader_cannot_write() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "reader").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": format!("obj-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");

    assert_eq!(
        write
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_cross_workspace_denied() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;
    let other_workspace = format!("ws-{}", uuid::Uuid::new_v4());

    sqlx::query("INSERT INTO ob_workspaces (id, name) VALUES ($1, $2)")
        .bind(&other_workspace)
        .bind(&other_workspace)
        .execute(&pool)
        .await
        .expect("insert other workspace");

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": format!("obj-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");
    assert_eq!(write.get("ok").and_then(|v| v.as_bool()), Some(true));

    let read = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({"scope": other_workspace, "refs": [write_body["objects"][0]["id"].clone()]}))
        .send()
        .await
        .expect("read")
        .json::<serde_json::Value>()
        .await
        .expect("read json");

    assert_eq!(
        read.get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    server.shutdown().await;
}
