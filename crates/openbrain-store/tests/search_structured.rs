use openbrain_core::{Envelope, LifecycleState, MemoryObject};
use openbrain_store::{
    OrderBySpec, OrderDirection, PgStore, PutObjectsRequest, SearchStructuredRequest, Store,
};
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

fn obj(
    id: &str,
    scope: &str,
    object_type: &str,
    status: &str,
    data: serde_json::Value,
) -> MemoryObject {
    MemoryObject {
        object_type: Some(object_type.to_string()),
        id: Some(id.to_string()),
        scope: Some(scope.to_string()),
        status: Some(status.to_string()),
        spec_version: Some("0.1".to_string()),
        tags: Some(vec!["t1".to_string()]),
        data: Some(data),
        provenance: Some(serde_json::json!({"actor":"tester","ts":"2026-01-01T00:00:00Z"})),
        lifecycle_state: None,
        expires_at: None,
        memory_key: None,
    }
}

#[allow(clippy::too_many_arguments)]
fn obj_with_lifecycle(
    id: &str,
    scope: &str,
    object_type: &str,
    status: &str,
    lifecycle_state: LifecycleState,
    expires_at: Option<&str>,
    memory_key: Option<&str>,
    data: serde_json::Value,
) -> MemoryObject {
    MemoryObject {
        object_type: Some(object_type.to_string()),
        id: Some(id.to_string()),
        scope: Some(scope.to_string()),
        status: Some(status.to_string()),
        spec_version: Some("0.1".to_string()),
        tags: Some(vec![]),
        data: Some(data),
        provenance: Some(serde_json::json!({"actor":"tester","ts":"2026-01-01T00:00:00Z"})),
        lifecycle_state: Some(lifecycle_state),
        expires_at: expires_at.map(|s| s.to_string()),
        memory_key: memory_key.map(|s| s.to_string()),
    }
}

#[tokio::test]
async fn structured_search_filters_and_restricts_scope() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope_a = format!("scope-a-{}", Uuid::new_v4());
    let scope_b = format!("scope-b-{}", Uuid::new_v4());

    let a1 = format!("obj-{}", Uuid::new_v4());
    let a2 = format!("obj-{}", Uuid::new_v4());
    let b1 = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj(
                    &a1,
                    &scope_a,
                    "claim",
                    "draft",
                    serde_json::json!({"priority": 5}),
                ),
                obj(
                    &a2,
                    &scope_a,
                    "claim",
                    "candidate",
                    serde_json::json!({"priority": 7}),
                ),
                obj(
                    &b1,
                    &scope_b,
                    "claim",
                    "draft",
                    serde_json::json!({"priority": 5}),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope_a.clone(),
            where_expr: Some(r#"type == "claim" AND status == "draft""#.to_string()),
            limit: Some(200),
            offset: Some(0),
            order_by: Some(OrderBySpec {
                field: "updated_at".to_string(),
                direction: OrderDirection::Desc,
            }),
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            let refs: Vec<String> = data.results.into_iter().map(|r| r.r#ref).collect();
            assert_eq!(refs, vec![a1]);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn structured_search_filters_by_nested_data() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());
    let a = format!("obj-{}", Uuid::new_v4());
    let b = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj(
                    &a,
                    &scope,
                    "task",
                    "draft",
                    serde_json::json!({"meta": {"kind": "a"}, "priority": 5}),
                ),
                obj(
                    &b,
                    &scope,
                    "task",
                    "draft",
                    serde_json::json!({"meta": {"kind": "b"}, "priority": 3}),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"data.meta.kind == "a" AND data.priority >= 5"#.to_string()),
            limit: Some(200),
            offset: Some(0),
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.results.len(), 1);
            assert_eq!(data.results[0].r#ref, a);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn structured_search_rejects_unknown_field() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());

    let res = store
        .search_structured(SearchStructuredRequest {
            scope,
            where_expr: Some(r#"evil == "x""#.to_string()),
            limit: None,
            offset: None,
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_REQUEST"),
    }
}

#[tokio::test]
async fn structured_search_rejects_injection_like_input() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());

    let res = store
        .search_structured(SearchStructuredRequest {
            scope,
            where_expr: Some(r#"type == "claim"; DROP TABLE ob_objects"#.to_string()),
            limit: None,
            offset: None,
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_REQUEST"),
    }
}

#[tokio::test]
async fn structured_search_defaults_to_accepted_and_not_expired() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());
    let now = "2026-01-02T00:00:00Z";

    let id_scratch = format!("obj-{}", Uuid::new_v4());
    let id_candidate = format!("obj-{}", Uuid::new_v4());
    let id_accepted = format!("obj-{}", Uuid::new_v4());
    let id_deprecated = format!("obj-{}", Uuid::new_v4());
    let id_expired = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj_with_lifecycle(
                    &id_scratch,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Scratch,
                    None,
                    None,
                    serde_json::json!({"priority": 1}),
                ),
                obj_with_lifecycle(
                    &id_candidate,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Candidate,
                    None,
                    None,
                    serde_json::json!({"priority": 2}),
                ),
                obj_with_lifecycle(
                    &id_accepted,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    None,
                    None,
                    serde_json::json!({"priority": 3}),
                ),
                obj_with_lifecycle(
                    &id_deprecated,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Deprecated,
                    None,
                    None,
                    serde_json::json!({"priority": 4}),
                ),
                obj_with_lifecycle(
                    &id_expired,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    Some("2026-01-01T00:00:00Z"),
                    None,
                    serde_json::json!({"priority": 5}),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"type == "claim""#.to_string()),
            limit: Some(50),
            offset: Some(0),
            order_by: None,
            include_states: None,
            include_expired: None,
            now: Some(now.to_string()),
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            let refs: Vec<String> = data.results.into_iter().map(|r| r.r#ref).collect();
            assert_eq!(refs, vec![id_accepted]);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn structured_search_include_states_and_expired_override() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());
    let now = "2026-01-02T00:00:00Z";

    let id_scratch = format!("obj-{}", Uuid::new_v4());
    let id_accepted = format!("obj-{}", Uuid::new_v4());
    let id_expired = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj_with_lifecycle(
                    &id_scratch,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Scratch,
                    None,
                    None,
                    serde_json::json!({"priority": 1}),
                ),
                obj_with_lifecycle(
                    &id_accepted,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    None,
                    None,
                    serde_json::json!({"priority": 2}),
                ),
                obj_with_lifecycle(
                    &id_expired,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    Some("2026-01-01T00:00:00Z"),
                    None,
                    serde_json::json!({"priority": 3}),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"type == "claim""#.to_string()),
            limit: Some(50),
            offset: Some(0),
            order_by: None,
            include_states: Some(vec![LifecycleState::Scratch, LifecycleState::Accepted]),
            include_expired: Some(true),
            now: Some(now.to_string()),
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            let mut refs: Vec<String> = data.results.into_iter().map(|r| r.r#ref).collect();
            refs.sort();
            let mut expected = vec![id_scratch, id_accepted, id_expired];
            expected.sort();
            assert_eq!(refs, expected);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn structured_search_promotion_changes_visibility() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_with_lifecycle(
                &id,
                &scope,
                "claim",
                "draft",
                LifecycleState::Scratch,
                None,
                None,
                serde_json::json!({"priority": 1}),
            )],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"type == "claim""#.to_string()),
            limit: Some(50),
            offset: Some(0),
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => assert!(data.results.is_empty()),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_with_lifecycle(
                &id,
                &scope,
                "claim",
                "draft",
                LifecycleState::Accepted,
                None,
                None,
                serde_json::json!({"priority": 2}),
            )],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"type == "claim""#.to_string()),
            limit: Some(50),
            offset: Some(0),
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.results.len(), 1);
            assert_eq!(data.results[0].r#ref, id);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn structured_search_conflict_detection() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let store = PgStore::from_pool(pool);

    let scope = format!("scope-{}", Uuid::new_v4());
    let key = "decision:db_provider";

    let id_a = format!("obj-{}", Uuid::new_v4());
    let id_b = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj_with_lifecycle(
                    &id_a,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    None,
                    Some(key),
                    serde_json::json!({"value": "postgres"}),
                ),
                obj_with_lifecycle(
                    &id_b,
                    &scope,
                    "claim",
                    "draft",
                    LifecycleState::Accepted,
                    None,
                    Some(key),
                    serde_json::json!({"value": "sqlite"}),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.clone(),
            where_expr: Some(r#"type == "claim""#.to_string()),
            limit: Some(50),
            offset: Some(0),
            order_by: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.results.len(), 2);
            for item in data.results {
                assert!(item.conflict);
                assert!(item.conflict_count.unwrap_or(0) >= 2);
                assert!(item
                    .conflicting_object_ids
                    .as_ref()
                    .map(|ids| !ids.is_empty())
                    .unwrap_or(false));
            }
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}
