use openbrain_embed::NoopEmbeddingProvider;
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::PgStore;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

async fn spawn_http_server(database_url: &str) -> (String, tokio::sync::oneshot::Sender<()>) {
    let store = PgStore::connect(database_url)
        .await
        .expect("connect postgres");
    let store =
        PgStore::from_pool_with_embedder(store.pool().clone(), Arc::new(NoopEmbeddingProvider));

    let llm = AnthropicClient::from_env();
    let app = build_router(AppState { store, llm });

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .expect("server run");
    });

    (format!("http://{}", addr), tx)
}

fn mcp_ping(database_url: &str) -> Value {
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
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"tools/call",
            "params": {"name":"openbrain.ping","arguments":{}}
        })
    )
    .expect("write tools/call");

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

    let line = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("mcp response line");

    let v: Value = serde_json::from_str(&line).expect("stdout JSON");
    let result = v.get("result").cloned().expect("result");

    let _ = child.kill();
    let _ = child.wait();

    result
}

#[tokio::test]
async fn parity_http_ping_and_mcp_ping_shape() {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "skipping postgres-backed tests: set DATABASE_URL (example: postgres://postgres:postgres@localhost:5432/postgres)"
            );
            return;
        }
    };

    // HTTP ping
    let (base, shutdown_tx) = spawn_http_server(&database_url).await;
    let client = reqwest::Client::new();
    let http_ping = client
        .post(format!("{}/v1/ping", base))
        .send()
        .await
        .expect("http ping")
        .json::<Value>()
        .await
        .expect("http ping json");

    let _ = shutdown_tx.send(());

    assert_eq!(http_ping.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        http_ping.get("version").and_then(|v| v.as_str()),
        Some("0.1")
    );

    // MCP ping
    let mcp_ping = mcp_ping(&database_url);
    assert_eq!(mcp_ping.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        mcp_ping.get("version").and_then(|v| v.as_str()),
        Some("0.1")
    );
}
