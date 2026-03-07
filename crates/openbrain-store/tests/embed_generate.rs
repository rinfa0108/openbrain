use openbrain_core::{Envelope, MemoryObject};
use openbrain_embed::FakeEmbeddingProvider;
use openbrain_store::{EmbedGenerateRequest, EmbedTarget, PgStore, PutObjectsRequest, Store};
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

fn obj_claim(id: &str, scope: &str) -> MemoryObject {
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
            "object": "c",
            "polarity": "pos"
        })),
        provenance: Some(serde_json::json!({"actor":"tester"})),
        lifecycle_state: None,
        expires_at: None,
        memory_key: None,
    }
}

#[tokio::test]
async fn embed_generate_text_dedupes_by_scope_model_checksum() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let scope = format!("scope-{}", Uuid::new_v4());
    let model = "fake-v1".to_string();

    let r1 = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text {
                text: "hello\r\n  world".to_string(),
            },
            model: model.clone(),
            dims: None,
        })
        .await;

    let (id1, checksum1) = match r1 {
        Envelope::Ok { data, .. } => {
            assert!(!data.reused);
            (data.embedding_id, data.checksum)
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    let r2 = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text {
                text: "hello world".to_string(),
            },
            model: model.clone(),
            dims: Some(1536),
        })
        .await;

    match r2 {
        Envelope::Ok { data, .. } => {
            assert!(data.reused);
            assert_eq!(data.embedding_id, id1);
            assert_eq!(data.checksum, checksum1);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }
}

#[tokio::test]
async fn embed_generate_is_provider_aware_for_dedupe() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store_a = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake-a",
    );
    let store_b = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake-b",
    );

    let scope = format!("scope-{}", Uuid::new_v4());
    let model = "fake-v1".to_string();
    let text = "same text across providers".to_string();

    let first = store_a
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text { text: text.clone() },
            model: model.clone(),
            dims: None,
        })
        .await;

    let first_id = match first {
        Envelope::Ok { data, .. } => data.embedding_id,
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    let second_same_provider = store_a
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text { text: text.clone() },
            model: model.clone(),
            dims: None,
        })
        .await;

    match second_same_provider {
        Envelope::Ok { data, .. } => {
            assert!(data.reused);
            assert_eq!(data.embedding_id, first_id);
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    }

    let third_other_provider = store_b
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text { text },
            model: model.clone(),
            dims: None,
        })
        .await;

    let third_id = match third_other_provider {
        Envelope::Ok { data, .. } => {
            assert!(!data.reused);
            data.embedding_id
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    assert_ne!(first_id, third_id);

    let count: i64 =
        sqlx::query_scalar(r#"SELECT COUNT(*) FROM ob_embeddings WHERE scope = $1 AND model = $2"#)
            .bind(&scope)
            .bind(&model)
            .fetch_one(&pool)
            .await
            .expect("count embeddings");

    assert_eq!(count, 2);
}

#[tokio::test]
async fn embed_generate_allows_multiple_providers_for_same_object() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store_a = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake-a",
    );
    let store_b = PgStore::from_pool_with_embedder_and_provider(
        pool.clone(),
        Arc::new(FakeEmbeddingProvider),
        "fake-b",
    );

    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let _ = store_a
        .put_objects(PutObjectsRequest {
            objects: vec![obj_claim(&id, &scope)],
            actor: None,
            idempotency_key: None,
        })
        .await;

    for store in [&store_a, &store_b] {
        let res = store
            .embed_generate(EmbedGenerateRequest {
                scope: scope.clone(),
                target: EmbedTarget::Ref { r#ref: id.clone() },
                model: "fake-v1".to_string(),
                dims: None,
            })
            .await;
        match res {
            Envelope::Ok { data, .. } => assert_eq!(data.object_id.as_deref(), Some(id.as_str())),
            Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
        }
    }

    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM ob_embeddings WHERE scope = $1 AND object_id = $2"#,
    )
    .bind(&scope)
    .bind(&id)
    .fetch_one(&pool)
    .await
    .expect("count embeddings");

    assert_eq!(count, 2);
}

#[tokio::test]
async fn embed_generate_allows_multiple_models_for_same_object() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());
    let model_a = "fake-v1".to_string();
    let model_b = "fake-v2".to_string();
    let text = "shared text for model-aware embeddings".to_string();

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_claim(&id, &scope)],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let first = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text { text: text.clone() },
            model: model_a,
            dims: None,
        })
        .await;

    let first_id = match first {
        Envelope::Ok { data, .. } => {
            assert!(!data.reused);
            data.embedding_id
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    let second = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Text { text: text.clone() },
            model: model_b,
            dims: None,
        })
        .await;

    let second_id = match second {
        Envelope::Ok { data, .. } => {
            assert!(!data.reused);
            data.embedding_id
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    assert_ne!(first_id, second_id);

    let count: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*) FROM ob_embeddings WHERE scope = $1 AND object_id = $2"#,
    )
    .bind(&scope)
    .bind(&id)
    .fetch_one(store.pool())
    .await
    .expect("count embeddings");

    assert_eq!(count, 2);
}

#[tokio::test]
async fn embed_generate_ref_stores_object_id() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool.clone(), Arc::new(FakeEmbeddingProvider));

    let scope = format!("scope-{}", Uuid::new_v4());
    let id = format!("obj-{}", Uuid::new_v4());

    let _ = store
        .put_objects(PutObjectsRequest {
            objects: vec![obj_claim(&id, &scope)],
            actor: None,
            idempotency_key: None,
        })
        .await;

    let res = store
        .embed_generate(EmbedGenerateRequest {
            scope: scope.clone(),
            target: EmbedTarget::Ref { r#ref: id.clone() },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    let embedding_id = match res {
        Envelope::Ok { data, .. } => {
            assert_eq!(data.object_id.as_deref(), Some(id.as_str()));
            data.embedding_id
        }
        Envelope::Err { error, .. } => panic!("unexpected error: {}", error.code),
    };

    let stored_object_id: Option<String> =
        sqlx::query_scalar(r#"SELECT object_id FROM ob_embeddings WHERE id = $1"#)
            .bind(&embedding_id)
            .fetch_one(&pool)
            .await
            .expect("read embedding row");

    assert_eq!(stored_object_id.as_deref(), Some(id.as_str()));
}

#[tokio::test]
async fn embed_generate_dims_mismatch_rejected() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let res = store
        .embed_generate(EmbedGenerateRequest {
            scope: format!("scope-{}", Uuid::new_v4()),
            target: EmbedTarget::Text {
                text: "hello".to_string(),
            },
            model: "fake-v1".to_string(),
            dims: Some(10),
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_REQUEST"),
    }
}

#[tokio::test]
async fn embed_generate_large_text_rejected() {
    let Some(pool) = setup_pool().await else {
        return;
    };

    let store = PgStore::from_pool_with_embedder(pool, Arc::new(FakeEmbeddingProvider));

    let big = "a".repeat(33_000);
    let res = store
        .embed_generate(EmbedGenerateRequest {
            scope: format!("scope-{}", Uuid::new_v4()),
            target: EmbedTarget::Text { text: big },
            model: "fake-v1".to_string(),
            dims: None,
        })
        .await;

    match res {
        Envelope::Ok { .. } => panic!("expected error"),
        Envelope::Err { error, .. } => assert_eq!(error.code, "OB_INVALID_REQUEST"),
    }
}
