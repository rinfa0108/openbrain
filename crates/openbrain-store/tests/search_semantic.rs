use async_trait::async_trait;
use openbrain_core::textnorm::normalize_object_text;
use openbrain_core::{Envelope, LifecycleState, MemoryObject};
use openbrain_embed::{EmbedError, EmbeddingProvider, FakeEmbeddingProvider};
use openbrain_store::{
    EmbedGenerateRequest, EmbedTarget, PgStore, PutObjectsRequest, SearchSemanticRequest, Store,
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
        tags: Some(vec![]),
        data: Some(data),
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: None,
        expires_at: None,
        memory_key: None,
    }
}

fn obj_with_lifecycle(
    id: &str,
    scope: &str,
    object_type: &str,
    status: &str,
    lifecycle_state: LifecycleState,
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
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: Some(lifecycle_state),
        expires_at: None,
        memory_key: memory_key.map(|s| s.to_string()),
    }
}

async fn seed_object_and_embedding(store: &PgStore, scope: &str, id: &str, object: MemoryObject) {
    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![object],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let _ = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.to_string(),
            target: EmbedTarget::Ref {
                r#ref: id.to_string(),
            },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;
}

#[tokio::test]
async fn semantic_search_orders_by_score_and_is_scope_isolated() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let scope_a = format!("scope-a-{}", Uuid::new_v4());
    let scope_b = format!("scope-b-{}", Uuid::new_v4());

    let dec_id = format!("obj-{}", Uuid::new_v4());
    let claim_id = format!("obj-{}", Uuid::new_v4());
    let other_scope_id = format!("obj-{}", Uuid::new_v4());

    let dec_data = serde_json::json!({"title":"DecA","outcome":"yes","rationale":"because"});
    let claim_data =
        serde_json::json!({"subject":"a","predicate":"b","object":"c","polarity":"pos"});

    seed_object_and_embedding(
        &store,
        &scope_a,
        &dec_id,
        obj(&dec_id, &scope_a, "decision", "draft", dec_data.clone()),
    )
    .await;
    seed_object_and_embedding(
        &store,
        &scope_a,
        &claim_id,
        obj(&claim_id, &scope_a, "claim", "draft", claim_data.clone()),
    )
    .await;
    seed_object_and_embedding(
        &store,
        &scope_b,
        &other_scope_id,
        obj(
            &other_scope_id,
            &scope_b,
            "decision",
            "draft",
            dec_data.clone(),
        ),
    )
    .await;

    let query = normalize_object_text("decision", &dec_data).unwrap();

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: scope_a.clone(),
            query,
            top_k: Some(10),
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert_eq!(data.matches[0].r#ref, dec_id);
            assert_eq!(data.matches[0].kind, "decision");
            assert!(data.matches.iter().all(|m| m.r#ref != other_scope_id));
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn semantic_search_applies_filters() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let scope = format!("scope-{}", Uuid::new_v4());
    let dec_id = format!("obj-{}", Uuid::new_v4());
    let claim_id = format!("obj-{}", Uuid::new_v4());

    let dec_data = serde_json::json!({"title":"DecB","outcome":"no","rationale":"why"});
    let claim_data =
        serde_json::json!({"subject":"x","predicate":"y","object":"z","polarity":"pos"});

    seed_object_and_embedding(
        &store,
        &scope,
        &dec_id,
        obj(&dec_id, &scope, "decision", "draft", dec_data.clone()),
    )
    .await;
    seed_object_and_embedding(
        &store,
        &scope,
        &claim_id,
        obj(&claim_id, &scope, "claim", "draft", claim_data.clone()),
    )
    .await;

    let query = normalize_object_text("claim", &claim_data).unwrap();

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query,
            top_k: Some(10),
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: Some(r#"type == "decision""#.to_string()),
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert!(data.matches.iter().all(|m| m.kind == "decision"));
            assert_eq!(data.matches[0].r#ref, dec_id);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn semantic_search_top_k_is_capped_to_50() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));
    let scope = format!("scope-{}", Uuid::new_v4());

    let mut objects = Vec::new();
    let mut ids = Vec::new();

    for _ in 0..55 {
        let id = format!("obj-{}", Uuid::new_v4());
        ids.push(id.clone());
        objects.push(obj(
            &id,
            &scope,
            "claim",
            "draft",
            serde_json::json!({"subject": id, "predicate":"p", "object":"o", "polarity":"pos"}),
        ));
    }

    let _ = store
        .put_objects(PutObjectsRequest {
            objects,
            actor: None,
            idempotency_key: None,
        })
        .await;

    for id in &ids {
        let _ = store
            .embed_generate(EmbedGenerateRequest {
                scope: scope.clone(),
                target: EmbedTarget::Ref { r#ref: id.clone() },
                model: "fake-v1".to_string(),
                dims: None,
            })
            .await;
    }

    let query = "hello world".to_string();

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query,
            top_k: Some(500),
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => assert!(data.matches.len() <= 50),
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn semantic_search_rejects_invalid_filter() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: format!("scope-{}", Uuid::new_v4()),
            query: "hello".to_string(),
            top_k: None,
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: Some("type === \"claim\"".to_string()),
            types: None,
            status: None,
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
async fn semantic_search_respects_embedding_provider_selection() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store_a = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "alpha",
    );
    let store_b = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "beta",
    );

    let scope = format!("scope-{}", Uuid::new_v4());
    let obj_a = format!("obj-{}", Uuid::new_v4());
    let obj_b = format!("obj-{}", Uuid::new_v4());

    let data_a = serde_json::json!({"title":"Alpha","outcome":"yes","rationale":"a"});
    let data_b = serde_json::json!({"title":"Beta","outcome":"no","rationale":"b"});

    let _ = store_a
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj(&obj_a, &scope, "decision", "draft", data_a.clone()),
                obj(&obj_b, &scope, "decision", "draft", data_b.clone()),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let _ = store_a
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref {
                r#ref: obj_a.clone(),
            },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    let _ = store_b
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref {
                r#ref: obj_b.clone(),
            },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    let query = normalize_object_text("decision", &data_a).unwrap();

    let res_alpha = store_a
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query: query.clone(),
            top_k: Some(5),
            model: None,
            embedding_provider: Some("alpha".to_string()),
            embedding_model: Some("fake-v1".to_string()),
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res_alpha {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert_eq!(data.matches[0].r#ref, obj_a);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }

    let res_beta = store_a
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query,
            top_k: Some(5),
            model: None,
            embedding_provider: Some("beta".to_string()),
            embedding_model: Some("fake-v1".to_string()),
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res_beta {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert_eq!(data.matches[0].r#ref, obj_b);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn semantic_search_respects_embedding_model_selection() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let scope = format!("scope-{}", Uuid::new_v4());
    let model_a_id = format!("obj-{}", Uuid::new_v4());
    let model_b_id = format!("obj-{}", Uuid::new_v4());

    let model_a_data =
        serde_json::json!({"title":"Model A","outcome":"yes","rationale":"embedding model one"});
    let model_b_data =
        serde_json::json!({"title":"Model B","outcome":"no","rationale":"embedding model two"});

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![
                obj(
                    &model_a_id,
                    &scope,
                    "decision",
                    "draft",
                    model_a_data.clone(),
                ),
                obj(
                    &model_b_id,
                    &scope,
                    "decision",
                    "draft",
                    model_b_data.clone(),
                ),
            ],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let _ = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref {
                r#ref: model_a_id.clone(),
            },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    let _ = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref {
                r#ref: model_b_id.clone(),
            },
            model: "fake-v2".to_string(),
            dims: None,
        })
        .await;

    let query_a = normalize_object_text("model a", &model_a_data).unwrap();
    let query_b = normalize_object_text("model b", &model_b_data).unwrap();

    let res_a = store
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query: query_a,
            top_k: Some(5),
            model: None,
            embedding_provider: None,
            embedding_model: Some("fake-v1".to_string()),
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res_a {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert_eq!(data.matches[0].r#ref, model_a_id);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }

    let res_b = store
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query: query_b,
            top_k: Some(5),
            model: None,
            embedding_provider: None,
            embedding_model: Some("fake-v2".to_string()),
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res_b {
        Envelope::Ok { data, .. } => {
            assert!(!data.matches.is_empty());
            assert_eq!(data.matches[0].r#ref, model_b_id);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[derive(Debug)]
struct BadDimsProvider;

#[async_trait]
impl EmbeddingProvider for BadDimsProvider {
    async fn embed(&self, _model: &str, _text: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(vec![0.0; 10])
    }
}

#[tokio::test]
async fn semantic_search_dims_mismatch_fails() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(BadDimsProvider));

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: format!("scope-{}", Uuid::new_v4()),
            query: "hello".to_string(),
            top_k: None,
            model: Some("bad".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_EMBEDDING_FAILED"),
    }
}

#[tokio::test]
async fn semantic_search_large_query_rejected() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));
    let big = "a".repeat(33_000);

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: format!("scope-{}", Uuid::new_v4()),
            query: big,
            top_k: None,
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
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
async fn semantic_search_surfaces_conflicts() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));
    let scope = format!("scope-{}", Uuid::new_v4());
    let key = "fact:db_provider";

    let id_a = format!("obj-{}", Uuid::new_v4());
    let id_b = format!("obj-{}", Uuid::new_v4());

    let data_a = serde_json::json!({"title":"Decision A","outcome":"yes","rationale":"a"});
    let data_b = serde_json::json!({"title":"Decision B","outcome":"no","rationale":"b"});

    seed_object_and_embedding(
        &store,
        &scope,
        &id_a,
        obj_with_lifecycle(
            &id_a,
            &scope,
            "decision",
            "draft",
            LifecycleState::Accepted,
            Some(key),
            data_a.clone(),
        ),
    )
    .await;
    seed_object_and_embedding(
        &store,
        &scope,
        &id_b,
        obj_with_lifecycle(
            &id_b,
            &scope,
            "decision",
            "draft",
            LifecycleState::Accepted,
            Some(key),
            data_b.clone(),
        ),
    )
    .await;

    let query = normalize_object_text("decision", &data_a).unwrap();

    let res = store
        .search_semantic(SearchSemanticRequest {
            scope: scope.clone(),
            query,
            top_k: Some(10),
            model: Some("fake-v1".to_string()),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            filters: None,
            types: None,
            status: None,
            include_states: None,
            include_expired: None,
            now: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => {
            let mut saw_a = false;
            let mut saw_b = false;
            for m in data.matches {
                if m.r#ref == id_a {
                    saw_a = true;
                    assert!(m.conflict);
                }
                if m.r#ref == id_b {
                    saw_b = true;
                    assert!(m.conflict);
                }
            }
            assert!(saw_a && saw_b);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}
