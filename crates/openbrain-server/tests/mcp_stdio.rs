use serde_json::Value;
use sqlx::PgPool;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn setup_pool() -> Option<PgPool> {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "skipping postgres-backed tests: set DATABASE_URL (example: postgres://postgres:postgres@localhost:5432/postgres)"
            );
            return None;
        }
    };

    let rt = tokio::runtime::Runtime::new().expect("rt");
    rt.block_on(async move {
        let pool = PgPool::connect(&database_url).await.expect("connect");
        let migrations_dir =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let migrator = sqlx::migrate::Migrator::new(migrations_dir)
            .await
            .expect("load migrations");
        migrator.run(&pool).await.expect("run migrations");
        Some(pool)
    })
}

fn create_workspace_token(pool: &PgPool, role: &str) -> (String, String) {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    rt.block_on(async move {
        let workspace_id = format!("ws-{}", uuid::Uuid::new_v4());
        let token = format!("ob_test_{}", uuid::Uuid::new_v4());
        let token_hash = openbrain_store::hash_token(&token);
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
    })
}

#[test]
fn mcp_stdio_ping_smoke() {
    let Some(pool) = setup_pool() else {
        return;
    };
    let (token, _workspace_id) = create_workspace_token(&pool, "writer");

    let mut child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
        .arg("mcp")
        .env(
            "DATABASE_URL",
            std::env::var("DATABASE_URL").expect("db url"),
        )
        .env("OPENBRAIN_EMBED_PROVIDER", "noop")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn openbrain mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");

    // Send initialize (best-effort; server tolerates missing init)
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params": {"protocolVersion":"0","auth_token": token}
        })
    )
    .expect("write initialize");

    // Call openbrain.ping
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
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

    let deadline = Duration::from_secs(5);
    let start = std::time::Instant::now();

    let mut saw_ping = false;

    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        let Ok(line) = rx.recv_timeout(remaining) else {
            break;
        };

        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let result = v.get("result").expect("result");
            assert_eq!(result.get("ok").and_then(|b| b.as_bool()), Some(true));
            assert_eq!(result.get("version").and_then(|s| s.as_str()), Some("0.1"));
            saw_ping = true;
            break;
        }
    }

    assert!(saw_ping, "did not receive ping response");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn mcp_stdio_requires_auth_for_write() {
    let Some(_pool) = setup_pool() else {
        return;
    };

    let mut child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
        .arg("mcp")
        .env(
            "DATABASE_URL",
            std::env::var("DATABASE_URL").expect("db url"),
        )
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
            "params": {"name":"openbrain.write","arguments":{"objects":[]}}
        })
    )
    .expect("write tools/call");

    drop(stdin);

    let reader = BufReader::new(stdout);
    let mut saw_unauth = false;
    for line in reader.lines().map_while(Result::ok) {
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(1) {
            let result = v.get("result").expect("result");
            let code = result
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(|s| s.as_str());
            if code == Some("OB_UNAUTHENTICATED") {
                saw_unauth = true;
            }
            break;
        }
    }

    assert!(saw_unauth, "did not receive unauthenticated error");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn mcp_stdio_reader_forbidden_write() {
    let Some(pool) = setup_pool() else {
        return;
    };
    let (token, workspace_id) = create_workspace_token(&pool, "reader");

    let mut child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
        .arg("mcp")
        .env(
            "DATABASE_URL",
            std::env::var("DATABASE_URL").expect("db url"),
        )
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
            "method":"initialize",
            "params": {"protocolVersion":"0","auth_token": token}
        })
    )
    .expect("write initialize");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params": {"name":"openbrain.write","arguments":{"objects":[{"type":"claim","id":"obj-1","scope":workspace_id,"status":"draft","spec_version":"0.1","tags":[],"data":{"subject":"a","predicate":"b","object":"c","polarity":"pos"},"provenance":{"actor":"tester"}}]}}
        })
    )
    .expect("write tools/call");

    drop(stdin);

    let reader = BufReader::new(stdout);
    let mut saw_forbidden = false;
    for line in reader.lines().map_while(Result::ok) {
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let result = v.get("result").expect("result");
            let code = result
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(|s| s.as_str());
            if code == Some("OB_FORBIDDEN") {
                saw_forbidden = true;
            }
            break;
        }
    }

    assert!(saw_forbidden, "did not receive forbidden error");

    let _ = child.kill();
    let _ = child.wait();
}
