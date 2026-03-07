use openbrain_core::{Envelope, MemoryObject};
use openbrain_store::{GetObjectsRequest, PgStore, PutObjectsRequest, Store};
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

fn valid_object(id: &str, scope: &str, spec_version: &str) -> MemoryObject {
    MemoryObject {
        object_type: Some("claim".to_string()),
        id: Some(id.to_string()),
        scope: Some(scope.to_string()),
        status: Some("draft".to_string()),
        spec_version: Some(spec_version.to_string()),
        tags: Some(vec!["t1".to_string()]),
        data: Some(serde_json::json!({"k":"v"})),
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: None,
        expires_at: None,
        memory_key: None,
    }
}

#[tokio::test]
async fn roundtrip_write_read() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let put = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &scope, "0.1")],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let version = match put {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.results.len(), 1);
            assert_eq!(data.results[0].version, 1);
            data.results[0].version
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };
    assert_eq!(version, 1);

    let got = store
        .get_objects(GetObjectsRequest {
            scope: scope.clone(),
            refs: vec![id.clone()],
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match got {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.objects.len(), 1);
            let o = &data.objects[0];
            assert_eq!(o.id, id);
            assert_eq!(o.scope, scope);
            assert_eq!(o.object_type, "claim");
            assert_eq!(o.status, "draft");
            assert_eq!(o.spec_version, "0.1");
            assert_eq!(o.tags, vec!["t1".to_string()]);
            assert_eq!(o.data, serde_json::json!({"k":"v"}));
            assert_eq!(o.provenance, serde_json::json!({"actor":"tester"}));
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn insert_sets_version_1() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let res = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &scope, "0.1")],
            actor: None,
            idempotency_key: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => assert_eq!(data.results[0].version, 1),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn update_increments_version() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let first = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &scope, "0.1")],
            actor: None,
            idempotency_key: None,
        })
        .await;

    match first {
        Envelope::Ok { data, .. } => assert_eq!(data.results[0].version, 1),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }

    let mut updated = valid_object(&id, &scope, "0.1");
    updated.status = Some("candidate".to_string());
    updated.data = Some(serde_json::json!({"k":"v2"}));

    let second = store
        .put_objects(PutObjectsRequest {
            objects: vec![updated],
            actor: None,
            idempotency_key: None,
        })
        .await;

    match second {
        Envelope::Ok { data, .. } => assert_eq!(data.results[0].version, 2),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn write_appends_event() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool.clone());

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &scope, "0.1")],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)
           FROM ob_events
           WHERE scope = $1
             AND event_type = 'object_written'
             AND payload->>'ref' = $2"#,
    )
    .bind(&scope)
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("count events");

    assert!(count >= 1);
}

#[tokio::test]
async fn invalid_object_missing_required_field_fails_with_invalid_schema() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let mut obj = valid_object(&id, &scope, "0.1");
    obj.data = None;

    let res = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj],
            actor: None,
            idempotency_key: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_SCHEMA"),
    }
}

#[tokio::test]
async fn wrong_spec_version_fails_with_unsupported_version() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let obj = valid_object(&id, &scope, "0.0");

    let res = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj],
            actor: None,
            idempotency_key: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_UNSUPPORTED_VERSION"),
    }
}
