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

fn create_token_for_workspace(pool: &PgPool, workspace_id: &str, role: &str) -> String {
    let rt = tokio::runtime::Runtime::new().expect("rt");
    rt.block_on(async move {
        let token = format!("ob_test_{}", uuid::Uuid::new_v4());
        let token_hash = openbrain_store::hash_token(&token);
        let identity_id = uuid::Uuid::new_v4().to_string();

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
        .bind(workspace_id)
        .bind(role)
        .bind("test")
        .execute(pool)
        .await
        .expect("insert token");

        token
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

#[test]
fn mcp_stdio_lifecycle_filters_read() {
    let Some(pool) = setup_pool() else {
        return;
    };
    let (token, workspace_id) = create_workspace_token(&pool, "writer");

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

    let obj_id = "obj-lifecycle-1";

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
            "params": {"name":"openbrain.write","arguments":{
                "objects":[{"type":"claim","id":obj_id,"scope":workspace_id,"status":"draft","spec_version":"0.1","tags":[],"data":{"subject":"a","predicate":"b","object":"c","polarity":"pos"},"provenance":{"actor":"tester"},"lifecycle_state":"scratch"}]
            }}
        })
    )
    .expect("write tools/call");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params": {"name":"openbrain.read","arguments":{"scope":workspace_id,"refs":[obj_id]}}
        })
    )
    .expect("write read");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":4,
            "method":"tools/call",
            "params": {"name":"openbrain.read","arguments":{"scope":workspace_id,"refs":[obj_id],"include_states":["scratch"]}}
        })
    )
    .expect("write read override");

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
    let mut saw_write = false;
    let mut saw_read_denied = false;
    let mut saw_read_override = false;

    while start.elapsed() < deadline {
        let remaining = deadline.saturating_sub(start.elapsed());
        let Ok(line) = rx.recv_timeout(remaining) else {
            break;
        };
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        let id = v.get("id").and_then(|id| id.as_i64());
        if id == Some(2) {
            let result = v.get("result").expect("result");
            assert_eq!(result.get("ok").and_then(|b| b.as_bool()), Some(true));
            saw_write = true;
        }
        if id == Some(3) {
            let result = v.get("result").expect("result");
            let code = result
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(|s| s.as_str());
            if code == Some("OB_NOT_FOUND") {
                saw_read_denied = true;
            }
        }
        if id == Some(4) {
            let result = v.get("result").expect("result");
            assert_eq!(result.get("ok").and_then(|b| b.as_bool()), Some(true));
            saw_read_override = true;
        }

        if saw_write && saw_read_denied && saw_read_override {
            break;
        }
    }

    assert!(saw_write);
    assert!(saw_read_denied);
    assert!(saw_read_override);

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn mcp_policy_denies_reader_decision_read() {
    let Some(pool) = setup_pool() else {
        return;
    };
    let (owner_token, workspace_id) = create_workspace_token(&pool, "owner");
    let reader_token = create_token_for_workspace(&pool, &workspace_id, "reader");

    let mut owner_child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
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
        .expect("spawn owner mcp");

    let mut owner_stdin = owner_child.stdin.take().expect("stdin");
    let owner_stdout = owner_child.stdout.take().expect("stdout");
    let decision_id = format!("decision-{}", uuid::Uuid::new_v4());

    writeln!(
        owner_stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"protocolVersion":"0","auth_token": owner_token}
        })
    )
    .expect("init owner");
    writeln!(
        owner_stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"openbrain.write","arguments":{"objects":[{
                "type":"policy.rule",
                "id":format!("policy-{}", uuid::Uuid::new_v4()),
                "scope":workspace_id,
                "status":"canonical",
                "spec_version":"0.1",
                "tags":[],
                "data":{
                    "id":"deny-reader-decision",
                    "effect":"deny",
                    "operations":["read","search_structured","search_semantic"],
                    "roles":["reader"],
                    "object_kinds":["decision"],
                    "reason":"OB_POLICY_DENY_DECISION_READ"
                },
                "provenance":{"actor":"owner"}
            }]}}
        })
    )
    .expect("policy write");
    writeln!(
        owner_stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"openbrain.write","arguments":{"objects":[{
                "type":"decision",
                "id":decision_id,
                "scope":workspace_id,
                "status":"draft",
                "spec_version":"0.1",
                "tags":[],
                "data":{"k":"v"},
                "provenance":{"actor":"owner"}
            }]}}
        })
    )
    .expect("decision write");
    drop(owner_stdin);

    let owner_reader = BufReader::new(owner_stdout);
    let mut policy_ok = false;
    let mut write_ok = false;
    for line in owner_reader.lines().map_while(Result::ok) {
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            policy_ok = v
                .get("result")
                .and_then(|r| r.get("ok"))
                .and_then(|b| b.as_bool())
                == Some(true);
        }
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            write_ok = v
                .get("result")
                .and_then(|r| r.get("ok"))
                .and_then(|b| b.as_bool())
                == Some(true);
            break;
        }
    }
    assert!(policy_ok);
    assert!(write_ok);
    let _ = owner_child.kill();
    let _ = owner_child.wait();

    let mut reader_child = Command::new(env!("CARGO_BIN_EXE_openbrain"))
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
        .expect("spawn reader mcp");
    let mut reader_stdin = reader_child.stdin.take().expect("stdin");
    let reader_stdout = reader_child.stdout.take().expect("stdout");

    writeln!(
        reader_stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params":{"protocolVersion":"0","auth_token": reader_token}
        })
    )
    .expect("init reader");
    writeln!(
        reader_stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"tools/call",
            "params":{"name":"openbrain.read","arguments":{"scope":workspace_id,"refs":[decision_id]}}
        })
    )
    .expect("reader read");
    drop(reader_stdin);

    let mut saw_forbidden = false;
    let reader = BufReader::new(reader_stdout);
    for line in reader.lines().map_while(Result::ok) {
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(2) {
            let code = v
                .get("result")
                .and_then(|r| r.get("error"))
                .and_then(|e| e.get("code"))
                .and_then(|s| s.as_str());
            if code == Some("OB_FORBIDDEN") {
                saw_forbidden = true;
            }
            break;
        }
    }
    assert!(saw_forbidden);
    let _ = reader_child.kill();
    let _ = reader_child.wait();
}

#[test]
fn mcp_audit_object_timeline_parity() {
    let Some(pool) = setup_pool() else {
        return;
    };
    let (token, workspace_id) = create_workspace_token(&pool, "owner");
    let obj_id = format!("obj-{}", uuid::Uuid::new_v4());

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
            "params":{"protocolVersion":"0","auth_token": token}
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
            "params":{"name":"openbrain.write","arguments":{"objects":[
                {"type":"claim","id":obj_id,"scope":workspace_id,"status":"draft","spec_version":"0.1","tags":[],"data":{"k":"v"},"provenance":{"actor":"owner"}}
            ]}}
        })
    )
    .expect("write object");

    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"tools/call",
            "params":{"name":"openbrain.audit.object_timeline","arguments":{"scope":workspace_id,"object_id":obj_id,"limit":20}}
        })
    )
    .expect("write audit call");

    drop(stdin);

    let reader = BufReader::new(stdout);
    let mut saw_audit_ok = false;
    for line in reader.lines().map_while(Result::ok) {
        let v: Value = serde_json::from_str(&line).expect("stdout JSON");
        if v.get("id").and_then(|id| id.as_i64()) == Some(3) {
            let ok = v
                .get("result")
                .and_then(|r| r.get("ok"))
                .and_then(|b| b.as_bool());
            let events = v
                .get("result")
                .and_then(|r| r.get("events"))
                .and_then(|e| e.as_array())
                .cloned()
                .unwrap_or_default();
            assert_eq!(ok, Some(true));
            assert!(
                !events.is_empty(),
                "audit timeline should include object events"
            );
            saw_audit_ok = true;
            break;
        }
    }

    assert!(saw_audit_ok);
    let _ = child.kill();
    let _ = child.wait();
}
