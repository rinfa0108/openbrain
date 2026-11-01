//! Upgrade path for an existing database.
//!
//! Builds a database exactly as it was after migration 0006 (the state of every
//! deployment created before the append-only history guard), shows history could
//! still be rewritten there, then applies 0007 and shows it no longer can while the
//! existing rows survive. Needs `DATABASE_URL` for a role that can CREATE DATABASE;
//! it skips otherwise unless `OPENBRAIN_REQUIRE_DATABASE_TESTS=1`.

use sqlx::{Connection, Executor, PgConnection};
use std::path::PathBuf;
use uuid::Uuid;

const THROUGH_0006: &[&str] = &[
    "0001_init.sql",
    "0002_embeddings_provider_kind.sql",
    "0003_workspace_access.sql",
    "0004_memory_lifecycle.sql",
    "0005_idempotency_ledger.sql",
    "0006_object_version_history.sql",
];
const GUARD: &str = "0007_object_version_history_append_only.sql";
const INSERT_VERSION: &str = "INSERT INTO ob_object_versions \
    (object_id, scope, version, type, status, spec_version, data, provenance) \
    VALUES ('o1', 's', $1, 'claim', 'draft', '1', '{}', '{}')";

fn migration(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../migrations")
        .join(name);
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {name}: {error}"))
}

fn database_url() -> Option<String> {
    match std::env::var("DATABASE_URL") {
        Ok(url) => Some(url),
        Err(_) => {
            if std::env::var("OPENBRAIN_REQUIRE_DATABASE_TESTS").as_deref() == Ok("1") {
                panic!("DATABASE_URL is required when OPENBRAIN_REQUIRE_DATABASE_TESTS=1");
            }
            eprintln!("skipping postgres-backed tests: set DATABASE_URL");
            None
        }
    }
}

/// The same server and credentials, a different database name.
fn with_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, format!("?{query}")),
        None => (url, String::new()),
    };
    let (server, _) = base
        .rsplit_once('/')
        .expect("DATABASE_URL has a database name");
    format!("{server}/{database}{query}")
}

async fn rejected(connection: &mut PgConnection, statement: &str) -> bool {
    match sqlx::query(statement).execute(&mut *connection).await {
        Ok(_) => false,
        Err(sqlx::Error::Database(error)) => error.message().contains("append-only"),
        Err(other) => panic!("unexpected error for {statement}: {other}"),
    }
}

async fn scenario(url: &str, name: &str) -> Result<(), sqlx::Error> {
    let mut db = PgConnection::connect(&with_database(url, name)).await?;
    for file in THROUGH_0006 {
        db.execute(migration(file).as_str()).await?;
    }
    sqlx::query(INSERT_VERSION).bind(1_i64).execute(&mut db).await?;

    // The pre-0007 state: history is still rewritable. If this stops being true the
    // test no longer proves that 0007 is what changed the behaviour.
    let rewritten = sqlx::query("UPDATE ob_object_versions SET status = 'edited'")
        .execute(&mut db)
        .await?
        .rows_affected();
    assert_eq!(rewritten, 1, "history should be rewritable before 0007");
    sqlx::query("UPDATE ob_object_versions SET status = 'draft'")
        .execute(&mut db)
        .await?;

    db.execute(migration(GUARD).as_str()).await?;
    // 0007 must be safe to apply a second time.
    db.execute(migration(GUARD).as_str()).await?;

    assert!(rejected(&mut db, "UPDATE ob_object_versions SET status = 'x'").await);
    assert!(rejected(&mut db, "DELETE FROM ob_object_versions").await);
    assert!(rejected(&mut db, "TRUNCATE ob_object_versions").await);

    let kept: Vec<(String, String)> =
        sqlx::query_as("SELECT object_id, status FROM ob_object_versions")
            .fetch_all(&mut db)
            .await?;
    assert_eq!(kept, vec![("o1".to_string(), "draft".to_string())]);

    // Appending is still allowed.
    sqlx::query(INSERT_VERSION).bind(2_i64).execute(&mut db).await?;
    db.close().await
}

#[tokio::test]
async fn database_at_0006_gains_append_only_history_from_0007() {
    let Some(url) = database_url() else {
        return;
    };
    let name = format!("ob_upgrade_{}", Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&url).await.expect("connect admin");
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .expect("create scratch database");

    let outcome = scenario(&url, &name).await;

    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .expect("drop scratch database");
    outcome.expect("upgrade scenario");
}
