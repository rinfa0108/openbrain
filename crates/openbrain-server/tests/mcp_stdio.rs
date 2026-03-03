use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn mcp_stdio_ping_smoke() {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            eprintln!(
                "skipping postgres-backed tests: set DATABASE_URL (example: postgres://postgres:postgres@localhost:5432/postgres)"
            );
            return;
        }
    };

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

    // Send initialize (best-effort; server tolerates missing init)
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":1,
            "method":"initialize",
            "params": {"protocolVersion":"0"}
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
