use openbrain_core::{Envelope, MemoryObject};
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
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_REQUEST"),
    }
}
