use openbrain_core::{Envelope, MemoryObject};
use openbrain_store::{PgStore, PutObjectsRequest, Store};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

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

fn mk_object(id: &str, scope: &str, k: &str) -> MemoryObject {
    MemoryObject {
        object_type: Some("claim".to_string()),
        id: Some(id.to_string()),
        scope: Some(scope.to_string()),
        status: Some("draft".to_string()),
        spec_version: Some("0.1".to_string()),
        tags: Some(vec!["idempotency".to_string()]),
        data: Some(serde_json::json!({"key": k, "value": "v"})),
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: Some(openbrain_core::LifecycleState::Accepted),
        expires_at: None,
        memory_key: None,
        conflict_status: None,
        resolved_by_object_id: None,
        resolved_at: None,
        resolution_note: None,
    }
}

#[tokio::test]
async fn write_with_idempotency_key_is_replayed() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool.clone());
    let scope = format!("ws-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());
    let key = format!("idem-{}", Uuid::new_v4());

    let req = PutObjectsRequest {
        objects: vec![mk_object(&id, &scope, "a")],
        actor: Some("writer".to_string()),
        idempotency_key: Some(key),
    };

    let first = store.put_objects(req.clone()).await;
    let second = store.put_objects(req).await;

    let first_data = match first {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => panic!("unexpected first error: {}", error.code),
    };
    let second_data = match second {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => panic!("unexpected second error: {}", error.code),
    };

    assert!(!first_data.replayed);
    assert!(second_data.replayed);
    assert_eq!(first_data.results, second_data.results);
    assert_eq!(first_data.accepted_count, second_data.accepted_count);
    assert_eq!(first_data.object_ids, second_data.object_ids);
    assert_eq!(first_data.receipt_hash, second_data.receipt_hash);

    let object_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ob_objects WHERE scope = $1 AND id = $2")
            .bind(&scope)
            .bind(&id)
            .fetch_one(&pool)
            .await
            .expect("count objects");
    assert_eq!(object_count, 1);

    let event_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ob_events WHERE scope = $1 AND event_type = 'object_written' AND payload->>'ref' = $2",
    )
    .bind(&scope)
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("count events");
    assert_eq!(event_count, 1);
}

#[tokio::test]
async fn idempotency_key_mismatch_rejected() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);
    let scope = format!("ws-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());
    let key = format!("idem-{}", Uuid::new_v4());

    let first = PutObjectsRequest {
        objects: vec![mk_object(&id, &scope, "a")],
        actor: Some("writer".to_string()),
        idempotency_key: Some(key.clone()),
    };
    let second = PutObjectsRequest {
        objects: vec![mk_object(&id, &scope, "b")],
        actor: Some("writer".to_string()),
        idempotency_key: Some(key),
    };

    match store.put_objects(first).await {
        Envelope::Ok { .. } => {}
        Envelope::Err { error, .. } => panic!("unexpected first error: {}", error.code),
    }

    match store.put_objects(second).await {
        Envelope::Ok { .. } => panic!("expected mismatch error"),
        Envelope::Err { error, .. } => {
            assert_eq!(error.code, "OB_INVALID_REQUEST");
            let reason = error
                .details
                .as_ref()
                .and_then(|d| d.get("reason_code"))
                .and_then(|v| v.as_str());
            assert_eq!(reason, Some("OB_IDEMPOTENCY_KEY_REUSE_MISMATCH"));
        }
    }
}

#[tokio::test]
async fn idempotency_is_workspace_scoped() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);
    let key = format!("idem-{}", Uuid::new_v4());

    let scope_a = format!("ws-a-{}", Uuid::new_v4());
    let scope_b = format!("ws-b-{}", Uuid::new_v4());
    let id_a = format!("obj-a-{}", Uuid::new_v4());
    let id_b = format!("obj-b-{}", Uuid::new_v4());

    let a = store
        .put_objects(PutObjectsRequest {
            objects: vec![mk_object(&id_a, &scope_a, "a")],
            actor: Some("writer".to_string()),
            idempotency_key: Some(key.clone()),
        })
        .await;
    let b = store
        .put_objects(PutObjectsRequest {
            objects: vec![mk_object(&id_b, &scope_b, "a")],
            actor: Some("writer".to_string()),
            idempotency_key: Some(key),
        })
        .await;

    let a = match a {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => panic!("unexpected a error: {}", error.code),
    };
    let b = match b {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => panic!("unexpected b error: {}", error.code),
    };
    assert!(!a.replayed);
    assert!(!b.replayed);
}

#[tokio::test]
async fn bounded_object_ids_receipt_cap() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);
    let scope = format!("ws-{}", Uuid::new_v4());
    let key = format!("idem-{}", Uuid::new_v4());

    let mut objects = Vec::new();
    for i in 0..60 {
        objects.push(mk_object(
            &format!("obj-{}-{}", i, Uuid::new_v4()),
            &scope,
            "x",
        ));
    }

    let resp = store
        .put_objects(PutObjectsRequest {
            objects,
            actor: Some("writer".to_string()),
            idempotency_key: Some(key),
        })
        .await;

    match resp {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.accepted_count, 60);
            assert_eq!(data.object_ids.len(), 50);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn ledger_persists_across_new_connection() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let database_url = std::env::var("DATABASE_URL").expect("db url");

    let scope = format!("ws-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());
    let key = format!("idem-{}", Uuid::new_v4());
    let req = PutObjectsRequest {
        objects: vec![mk_object(&id, &scope, "p")],
        actor: Some("writer".to_string()),
        idempotency_key: Some(key),
    };

    let store_a = PgStore::from_pool(pool);
    let first = store_a.put_objects(req.clone()).await;
    match first {
        Envelope::Ok { data, .. } => assert!(!data.replayed),
        Envelope::Err { error, .. } => panic!("unexpected first error: {}", error.code),
    }

    let store_b = PgStore::connect(&database_url)
        .await
        .expect("new connection");
    let second = store_b.put_objects(req).await;
    match second {
        Envelope::Ok { data, .. } => assert!(data.replayed),
        Envelope::Err { error, .. } => panic!("unexpected second error: {}", error.code),
    }
}
