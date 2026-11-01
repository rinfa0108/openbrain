use openbrain_core::{Envelope, MemoryObject};
use openbrain_store::{GetObjectsRequest, PgStore, PutObjectsRequest, Store};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

async fn setup_pool() -> Option<PgPool> {
    let database_url = match std::env::var("DATABASE_URL") {
        Ok(v) => v,
        Err(_) => {
            if std::env::var("OPENBRAIN_REQUIRE_DATABASE_TESTS").as_deref() == Ok("1") {
                panic!("DATABASE_URL is required when OPENBRAIN_REQUIRE_DATABASE_TESTS=1");
            }
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
        conflict_status: None,
        resolved_by_object_id: None,
        resolved_at: None,
        resolution_note: None,
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
            expected_versions: std::collections::BTreeMap::new(),
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
            include_conflicts: None,
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
            expected_versions: std::collections::BTreeMap::new(),
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
            expected_versions: std::collections::BTreeMap::new(),
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
            expected_versions: std::collections::BTreeMap::from([(id.clone(), 1)]),
        })
        .await;

    match second {
        Envelope::Ok { data, .. } => assert_eq!(data.results[0].version, 2),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn cas_rejects_stale_and_cross_scope_writes_and_preserves_history() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool.clone());
    let scope = format!("test-{}", Uuid::new_v4());
    let other_scope = format!("test-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let first = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &scope, "0.1")],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::new(),
        })
        .await;
    assert!(matches!(first, Envelope::Ok { .. }));

    let mut current = valid_object(&id, &scope, "0.1");
    current.data = Some(serde_json::json!({"k":"current"}));
    let current_write = store
        .put_objects(PutObjectsRequest {
            objects: vec![current],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::from([(id.clone(), 1)]),
        })
        .await;
    assert!(matches!(current_write, Envelope::Ok { .. }));

    let mut stale = valid_object(&id, &scope, "0.1");
    stale.data = Some(serde_json::json!({"k":"stale"}));
    match store
        .put_objects(PutObjectsRequest {
            objects: vec![stale],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::from([(id.clone(), 1)]),
        })
        .await
    {
        Envelope::Err { error, .. } => {
            assert_eq!(error.code, "OB_CONFLICT");
            assert_eq!(
                error.details.as_ref().and_then(|v| v.get("reason_code")),
                Some(&serde_json::json!("OB_EXPECTED_VERSION_MISMATCH"))
            );
        }
        Envelope::Ok { .. } => panic!("stale write was accepted"),
    }

    let cross_scope = store
        .put_objects(PutObjectsRequest {
            objects: vec![valid_object(&id, &other_scope, "0.1")],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::from([(id.clone(), 2)]),
        })
        .await;
    match cross_scope {
        Envelope::Err { error, .. } => {
            assert_eq!(error.code, "OB_CONFLICT");
            assert!(error
                .details
                .as_ref()
                .and_then(|details| details.get("existing_scope"))
                .is_none());
            assert_eq!(
                error.details.as_ref().and_then(|v| v.get("reason_code")),
                Some(&serde_json::json!("OB_SCOPE_ID_COLLISION"))
            );
        }
        Envelope::Ok { .. } => panic!("cross-scope collision was accepted"),
    }

    let mut omitted_expected = valid_object(&id, &scope, "0.1");
    omitted_expected.data = Some(serde_json::json!({"k":"unconditional"}));
    match store
        .put_objects(PutObjectsRequest {
            objects: vec![omitted_expected],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::new(),
        })
        .await
    {
        Envelope::Err { error, .. } => {
            assert_eq!(error.code, "OB_CONFLICT");
            assert_eq!(
                error.details.as_ref().and_then(|v| v.get("reason_code")),
                Some(&serde_json::json!("OB_EXPECTED_VERSION_MISMATCH"))
            );
        }
        Envelope::Ok { .. } => panic!("unconditional existing-object update was accepted"),
    }

    let history_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ob_object_versions WHERE object_id = $1 AND scope = $2",
    )
    .bind(&id)
    .bind(&scope)
    .fetch_one(&pool)
    .await
    .expect("count object history");
    assert_eq!(history_count, 1);

    let direct_scope_update = sqlx::query("UPDATE ob_objects SET scope = $2 WHERE id = $1")
        .bind(&id)
        .bind(&other_scope)
        .execute(&pool)
        .await;
    assert!(
        direct_scope_update.is_err(),
        "direct scope mutation was accepted"
    );

    let direct_history_update = sqlx::query(
        "UPDATE ob_object_versions SET status = 'tampered' WHERE object_id = $1 AND version = 1",
    )
    .bind(&id)
    .execute(&pool)
    .await;
    assert!(
        direct_history_update.is_err(),
        "direct history update was accepted"
    );

    let direct_history_delete =
        sqlx::query("DELETE FROM ob_object_versions WHERE object_id = $1 AND version = 1")
            .bind(&id)
            .execute(&pool)
            .await;
    assert!(
        direct_history_delete.is_err(),
        "direct history delete was accepted"
    );
}

#[tokio::test]
async fn failed_batch_rolls_back_prior_object_updates() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool.clone());
    let scope = format!("test-{}", Uuid::new_v4());
    let first_id = format!("obj-{}", Uuid::new_v4());
    let second_id = format!("obj-{}", Uuid::new_v4());

    let initial = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                valid_object(&first_id, &scope, "0.1"),
                valid_object(&second_id, &scope, "0.1"),
            ],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::new(),
        })
        .await;
    assert!(matches!(initial, Envelope::Ok { .. }));

    let mut first_update = valid_object(&first_id, &scope, "0.1");
    first_update.data = Some(serde_json::json!({"k":"should-rollback"}));
    let mut second_update = valid_object(&second_id, &scope, "0.1");
    second_update.data = Some(serde_json::json!({"k":"should-fail"}));
    let failed = store
        .put_objects(PutObjectsRequest {
            objects: vec![first_update, second_update],
            actor: None,
            idempotency_key: None,
            expected_versions: std::collections::BTreeMap::from([
                (first_id.clone(), 1),
                (second_id.clone(), 0),
            ]),
        })
        .await;
    assert!(matches!(failed, Envelope::Err { .. }));

    let current = store
        .get_objects(GetObjectsRequest {
            scope: scope.clone(),
            refs: vec![first_id.clone()],
            include_states: None,
            include_expired: None,
            include_conflicts: None,
            now: None,
        })
        .await;
    match current {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.objects[0].version, 1);
            assert_eq!(data.objects[0].data, serde_json::json!({"k":"v"}));
        }
        Envelope::Err { error, .. } => panic!("unexpected read error: {}", error.code),
    }

    let history_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ob_object_versions WHERE object_id = $1")
            .bind(&first_id)
            .fetch_one(&pool)
            .await
            .expect("count rolled-back history");
    assert_eq!(history_count, 0);
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
            expected_versions: std::collections::BTreeMap::new(),
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
            expected_versions: std::collections::BTreeMap::new(),
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
            expected_versions: std::collections::BTreeMap::new(),
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_UNSUPPORTED_VERSION"),
    }
}
