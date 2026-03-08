use openbrain_core::{LifecycleState, MemoryObject};
use openbrain_embed::{FakeEmbeddingProvider, NoopEmbeddingProvider};
use openbrain_store::{
    EmbedGenerateRequest, EmbedTarget, EmbeddingCoverageRequest, EmbeddingReembedRequest, PgStore,
    PutObjectsRequest, Store,
};
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
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

fn obj_claim(id: &str, scope: &str, state: Option<LifecycleState>) -> MemoryObject {
    MemoryObject {
        object_type: Some("claim".to_string()),
        id: Some(id.to_string()),
        scope: Some(scope.to_string()),
        status: Some("draft".to_string()),
        spec_version: Some("0.1".to_string()),
        tags: Some(vec!["t1".to_string()]),
        data: Some(serde_json::json!({
            "subject": "a",
            "predicate": "b",
            "object": id,
            "polarity": "pos"
        })),
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: state,
        expires_at: None,
        memory_key: Some(format!("decision:{id}")),
        conflict_status: None,
        resolved_by_object_id: None,
        resolved_at: None,
        resolution_note: None,
    }
}

#[tokio::test]
async fn embedding_coverage_counts_missing_and_present() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let scope = format!("scope-{}", Uuid::new_v4());
    let id_a = format!("obj-{}", Uuid::new_v4());
    let id_b = format!("obj-{}", Uuid::new_v4());

    let store = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake",
    );
    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj_claim(&id_a, &scope, Some(LifecycleState::Accepted)),
                obj_claim(&id_b, &scope, Some(LifecycleState::Accepted)),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let _ = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref {
                r#ref: id_a.clone(),
            },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    let coverage = store
        .embedding_coverage(EmbeddingCoverageRequest {
            scope: scope.clone(),
            provider: "fake".to_string(),
            model: "fake-v1".to_string(),
            kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            missing_sample_limit: Some(10),
        })
        .await
        .expect("coverage");

    assert_eq!(coverage.total_eligible, 2);
    assert_eq!(coverage.with_embeddings, 1);
    assert_eq!(coverage.missing, 1);
    assert_eq!(coverage.missing_refs, vec![id_b]);
}

#[tokio::test]
async fn reembed_fills_missing_and_is_idempotent() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let scope = format!("scope-{}", Uuid::new_v4());
    let ids = [
        format!("obj-{}", Uuid::new_v4()),
        format!("obj-{}", Uuid::new_v4()),
    ];

    let store = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake",
    );
    let _ = store
        .put_objects(PutObjectsRequest {
            objects: ids
                .iter()
                .map(|id| obj_claim(id, &scope, Some(LifecycleState::Accepted)))
                .collect(),
            actor: None,
            idempotency_key: None,
        })
        .await;

    let first = store
        .reembed_missing(EmbeddingReembedRequest {
            scope: scope.clone(),
            to_provider: "fake".to_string(),
            to_model: "fake-v1".to_string(),
            to_kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            limit: Some(100),
            after: None,
            dry_run: false,
            max_bytes: None,
            max_objects: None,
            actor: Some("tester".to_string()),
        })
        .await
        .expect("first reembed");
    assert_eq!(first.processed, 2);

    let second = store
        .reembed_missing(EmbeddingReembedRequest {
            scope: scope.clone(),
            to_provider: "fake".to_string(),
            to_model: "fake-v1".to_string(),
            to_kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            limit: Some(100),
            after: None,
            dry_run: false,
            max_bytes: None,
            max_objects: None,
            actor: Some("tester".to_string()),
        })
        .await
        .expect("second reembed");
    assert_eq!(second.processed, 0);

    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM ob_embeddings
           WHERE scope = $1 AND provider = $2 AND model = $3 AND kind = $4"#,
    )
    .bind(&scope)
    .bind("fake")
    .bind("fake-v1")
    .bind("semantic")
    .fetch_one(&pool)
    .await
    .expect("count embeddings");
    assert_eq!(count, 2);
}

#[tokio::test]
async fn reembed_respects_lifecycle_state_filter() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let scope = format!("scope-{}", Uuid::new_v4());
    let accepted = format!("obj-{}", Uuid::new_v4());
    let candidate = format!("obj-{}", Uuid::new_v4());

    let store = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake",
    );
    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj_claim(&accepted, &scope, Some(LifecycleState::Accepted)),
                obj_claim(&candidate, &scope, Some(LifecycleState::Candidate)),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let out = store
        .reembed_missing(EmbeddingReembedRequest {
            scope: scope.clone(),
            to_provider: "fake".to_string(),
            to_model: "fake-v1".to_string(),
            to_kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            limit: Some(100),
            after: None,
            dry_run: false,
            max_bytes: None,
            max_objects: None,
            actor: None,
        })
        .await
        .expect("reembed");
    assert_eq!(out.processed, 1);
}

#[tokio::test]
async fn reembed_returns_clear_error_when_provider_unavailable() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let store = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(NoopEmbeddingProvider),
        "noop",
    );
    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_claim(&id, &scope, Some(LifecycleState::Accepted))],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let err = store
        .reembed_missing(EmbeddingReembedRequest {
            scope,
            to_provider: "noop".to_string(),
            to_model: "default".to_string(),
            to_kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            limit: Some(10),
            after: None,
            dry_run: false,
            max_bytes: None,
            max_objects: None,
            actor: None,
        })
        .await
        .expect_err("provider unavailable");

    assert_eq!(err.code, "OB_EMBEDDING_FAILED");
    assert_eq!(err.message, "embedding provider unavailable");
    assert_eq!(
        err.details
            .as_ref()
            .and_then(|d| d.get("reason"))
            .and_then(|v| v.as_str()),
        Some("provider_unavailable")
    );
}

#[tokio::test]
async fn reembed_dry_run_does_not_write_embeddings() {
    let Some(pool) = setup_pool().await else {
        return;
    };
    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());
    let store = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake",
    );

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_claim(&id, &scope, Some(LifecycleState::Accepted))],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let out = store
        .reembed_missing(EmbeddingReembedRequest {
            scope: scope.clone(),
            to_provider: "fake".to_string(),
            to_model: "fake-v1".to_string(),
            to_kind: "semantic".to_string(),
            state: LifecycleState::Accepted,
            limit: Some(10),
            after: None,
            dry_run: true,
            max_bytes: None,
            max_objects: None,
            actor: None,
        })
        .await
        .expect("dry run");

    assert_eq!(out.processed, 1);
    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM ob_embeddings
           WHERE scope = $1 AND provider = $2 AND model = $3 AND kind = $4"#,
    )
    .bind(&scope)
    .bind("fake")
    .bind("fake-v1")
    .bind("semantic")
    .fetch_one(&pool)
    .await
    .expect("count embeddings");
    assert_eq!(count, 0);
}
