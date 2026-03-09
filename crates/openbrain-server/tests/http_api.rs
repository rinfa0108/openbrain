use openbrain_embed::NoopEmbeddingProvider;
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::{hash_token, PgStore};
use serde_json::json;
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

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

struct TestServer {
    base: String,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl TestServer {
    async fn spawn() -> Option<Self> {
        let pool = setup_pool().await?;

        let store = PgStore::from_pool_with_embedder(pool, Arc::new(NoopEmbeddingProvider));
        let llm = AnthropicClient::from_env();
        let app = build_router(AppState { store, llm });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("local addr");

        let (tx, rx) = oneshot::channel::<()>();

        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await
                .expect("server run");
        });

        Some(Self {
            base: format!("http://{}", addr),
            shutdown_tx: Some(tx),
            task: Some(task),
        })
    }

    async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

async fn create_workspace_token(pool: &PgPool, role: &str) -> (String, String) {
    let workspace_id = format!("ws-{}", uuid::Uuid::new_v4());
    let token = format!("ob_test_{}", uuid::Uuid::new_v4());
    let token_hash = hash_token(&token);
    let identity_id = uuid::Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO ob_workspaces (id, name) VALUES ($1, $2)")
        .bind(&workspace_id)
        .bind(&workspace_id)
        .execute(pool)
        .await
        .expect("insert workspace");

    sqlx::query("INSERT INTO ob_identities (id, display_name) VALUES ($1, $2)")
        .bind(&identity_id)
        .bind("test-identity")
        .execute(pool)
        .await
        .expect("insert identity");

    sqlx::query(
        "INSERT INTO ob_tokens (token_hash, identity_id, workspace_id, role, label) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&token_hash)
    .bind(&identity_id)
    .bind(&workspace_id)
    .bind(role)
    .bind("test")
    .execute(pool)
    .await
    .expect("insert token");

    (token, workspace_id)
}

async fn create_token_for_workspace(pool: &PgPool, workspace_id: &str, role: &str) -> String {
    let token = format!("ob_test_{}", uuid::Uuid::new_v4());
    let token_hash = hash_token(&token);
    let identity_id = uuid::Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO ob_identities (id, display_name) VALUES ($1, $2)")
        .bind(&identity_id)
        .bind("test-identity")
        .execute(pool)
        .await
        .expect("insert identity");

    sqlx::query(
        "INSERT INTO ob_tokens (token_hash, identity_id, workspace_id, role, label) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&token_hash)
    .bind(&identity_id)
    .bind(workspace_id)
    .bind(role)
    .bind("test")
    .execute(pool)
    .await
    .expect("insert token");

    token
}

#[tokio::test]
async fn http_ping_write_read_and_structured_search() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    // ping
    let ping = client
        .post(format!("{}/v1/ping", base))
        .send()
        .await
        .expect("ping")
        .json::<serde_json::Value>()
        .await
        .expect("ping json");
    assert_eq!(ping.get("ok").and_then(|v| v.as_bool()), Some(true));

    // write
    let scope = workspace_id;
    let id1 = format!("obj-{}", uuid::Uuid::new_v4());
    let id2 = format!("obj-{}", uuid::Uuid::new_v4());

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": id1,
                "scope": scope,
                "status": "draft",
                "spec_version": "0.1",
                "tags": ["t1"],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            },
            {
                "type": "claim",
                "id": id2,
                "scope": scope,
                "status": "candidate",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"x","predicate":"y","object":"z","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");
    assert_eq!(write.get("ok").and_then(|v| v.as_bool()), Some(true));

    // read (scoped)
    let read = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({"scope": write_body["objects"][0]["scope"].clone(), "refs": [write_body["objects"][0]["id"].clone()]}))
        .send()
        .await
        .expect("read")
        .json::<serde_json::Value>()
        .await
        .expect("read json");
    assert_eq!(read.get("ok").and_then(|v| v.as_bool()), Some(true));

    // structured search
    let structured = client
        .post(format!("{}/v1/search/structured", base))
        .bearer_auth(&token)
        .json(&json!({
            "scope": write_body["objects"][0]["scope"].clone(),
            "where_expr": "type == \"claim\" AND status == \"draft\"",
            "limit": 50,
            "offset": 0
        }))
        .send()
        .await
        .expect("structured")
        .json::<serde_json::Value>()
        .await
        .expect("structured json");

    assert_eq!(structured.get("ok").and_then(|v| v.as_bool()), Some(true));
    let results = structured
        .get("results")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(results.len(), 1);

    server.shutdown().await;
}

#[tokio::test]
async fn http_requires_auth() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write = client
        .post(format!("{}/v1/write", base))
        .json(&json!({"objects": []}))
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");

    assert_eq!(
        write
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_UNAUTHENTICATED")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_reader_cannot_write() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "reader").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": format!("obj-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");

    assert_eq!(
        write
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_cross_workspace_denied() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;
    let other_workspace = format!("ws-{}", uuid::Uuid::new_v4());

    sqlx::query("INSERT INTO ob_workspaces (id, name) VALUES ($1, $2)")
        .bind(&other_workspace)
        .bind(&other_workspace)
        .execute(&pool)
        .await
        .expect("insert other workspace");

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": format!("obj-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"}
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");
    assert_eq!(write.get("ok").and_then(|v| v.as_bool()), Some(true));

    let read = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({"scope": other_workspace, "refs": [write_body["objects"][0]["id"].clone()]}))
        .send()
        .await
        .expect("read")
        .json::<serde_json::Value>()
        .await
        .expect("read json");

    assert_eq!(
        read.get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_lifecycle_filters_are_enforced() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (token, workspace_id) = create_workspace_token(&pool, "writer").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let id_scratch = format!("obj-{}", uuid::Uuid::new_v4());
    let id_accepted = format!("obj-{}", uuid::Uuid::new_v4());

    let write_body = json!({
        "objects": [
            {
                "type": "claim",
                "id": id_scratch,
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"a","predicate":"b","object":"c","polarity":"pos"},
                "provenance": {"actor":"tester"},
                "lifecycle_state": "scratch"
            },
            {
                "type": "claim",
                "id": id_accepted,
                "scope": workspace_id,
                "status": "draft",
                "spec_version": "0.1",
                "tags": [],
                "data": {"subject":"x","predicate":"y","object":"z","polarity":"pos"},
                "provenance": {"actor":"tester"},
                "lifecycle_state": "accepted"
            }
        ]
    });

    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&token)
        .json(&write_body)
        .send()
        .await
        .expect("write")
        .json::<serde_json::Value>()
        .await
        .expect("write json");
    assert_eq!(write.get("ok").and_then(|v| v.as_bool()), Some(true));

    let read_scratch = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({"scope": workspace_id, "refs": [id_scratch]}))
        .send()
        .await
        .expect("read scratch")
        .json::<serde_json::Value>()
        .await
        .expect("read scratch json");
    assert_eq!(
        read_scratch
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_NOT_FOUND")
    );

    let read_override = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&token)
        .json(&json!({
            "scope": workspace_id,
            "refs": [id_scratch],
            "include_states": ["scratch"]
        }))
        .send()
        .await
        .expect("read override")
        .json::<serde_json::Value>()
        .await
        .expect("read override json");
    assert_eq!(
        read_override.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let structured = client
        .post(format!("{}/v1/search/structured", base))
        .bearer_auth(&token)
        .json(&json!({
            "scope": workspace_id,
            "where_expr": "type == \"claim\"",
            "limit": 50,
            "offset": 0
        }))
        .send()
        .await
        .expect("structured")
        .json::<serde_json::Value>()
        .await
        .expect("structured json");
    let results = structured
        .get("results")
        .and_then(|v| v.as_array())
        .unwrap();
    assert_eq!(results.len(), 1);

    server.shutdown().await;
}

#[tokio::test]
async fn http_policy_denies_reader_decision_and_writer_promotion() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };

    let pool = setup_pool().await.expect("pool");
    let (owner_token, workspace_id) = create_workspace_token(&pool, "owner").await;
    let (reader_token, _) = create_workspace_token(&pool, "reader").await;
    let (writer_token, _) = create_workspace_token(&pool, "writer").await;

    let base = server.base.clone();
    let client = reqwest::Client::new();

    let policy_reader_deny = json!({
        "objects": [{
            "type": "policy.rule",
            "id": format!("policy-{}", uuid::Uuid::new_v4()),
            "scope": workspace_id,
            "status": "canonical",
            "spec_version": "0.1",
            "tags": [],
            "data": {
                "id": "deny-reader-decision",
                "effect": "deny",
                "operations": ["read", "search_structured", "search_semantic"],
                "roles": ["reader"],
                "object_kinds": ["decision"],
                "reason": "OB_POLICY_DENY_DECISION_READ"
            },
            "provenance": {"actor":"owner"}
        }]
    });
    let policy_write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&policy_reader_deny)
        .send()
        .await
        .expect("policy write")
        .json::<serde_json::Value>()
        .await
        .expect("policy json");
    assert_eq!(policy_write.get("ok").and_then(|v| v.as_bool()), Some(true));

    let decision_id = format!("decision-{}", uuid::Uuid::new_v4());
    let write_decision = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&writer_token)
        .json(&json!({
            "objects": [{
                "type":"decision",
                "id": decision_id,
                "scope": workspace_id,
                "status":"draft",
                "spec_version":"0.1",
                "tags": [],
                "data":{"k":"v"},
                "provenance":{"actor":"writer"}
            }]
        }))
        .send()
        .await
        .expect("write decision")
        .json::<serde_json::Value>()
        .await
        .expect("write decision json");
    assert_eq!(
        write_decision.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let denied_read = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&reader_token)
        .json(&json!({"scope": workspace_id, "refs": [decision_id]}))
        .send()
        .await
        .expect("read decision")
        .json::<serde_json::Value>()
        .await
        .expect("read decision json");
    assert_eq!(
        denied_read
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    let policy_promote_deny = json!({
        "objects": [{
            "type": "policy.rule",
            "id": format!("policy-{}", uuid::Uuid::new_v4()),
            "scope": workspace_id,
            "status": "canonical",
            "spec_version": "0.1",
            "tags": [],
            "data": {
                "id": "deny-writer-promote",
                "effect": "deny",
                "operations": ["write"],
                "roles": ["writer"],
                "lifecycle_transitions": ["candidate->accepted"],
                "reason": "OB_POLICY_DENY_PROMOTE_ACCEPTED"
            },
            "provenance": {"actor":"owner"}
        }]
    });
    let policy_promote = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&policy_promote_deny)
        .send()
        .await
        .expect("policy promote write")
        .json::<serde_json::Value>()
        .await
        .expect("policy promote json");
    assert_eq!(
        policy_promote.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let claim_id = format!("claim-{}", uuid::Uuid::new_v4());
    let writer_candidate = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&writer_token)
        .json(&json!({
            "objects": [{
                "type":"claim",
                "id": claim_id,
                "scope": workspace_id,
                "status":"candidate",
                "spec_version":"0.1",
                "tags": [],
                "data":{"k":"v1"},
                "provenance":{"actor":"writer"},
                "lifecycle_state":"candidate"
            }]
        }))
        .send()
        .await
        .expect("writer candidate")
        .json::<serde_json::Value>()
        .await
        .expect("writer candidate json");
    assert_eq!(
        writer_candidate.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let writer_promote = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&writer_token)
        .json(&json!({
            "objects": [{
                "type":"claim",
                "id": claim_id,
                "scope": workspace_id,
                "status":"canonical",
                "spec_version":"0.1",
                "tags": [],
                "data":{"k":"v1"},
                "provenance":{"actor":"writer"},
                "lifecycle_state":"accepted"
            }]
        }))
        .send()
        .await
        .expect("writer promote")
        .json::<serde_json::Value>()
        .await
        .expect("writer promote json");
    assert_eq!(
        writer_promote
            .get("error")
            .and_then(|e| e.get("code"))
            .and_then(|v| v.as_str()),
        Some("OB_FORBIDDEN")
    );

    server.shutdown().await;
}

#[tokio::test]
async fn http_governance_workspace_audit_retention_and_explainability() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };
    let pool = setup_pool().await.expect("pool");
    let (owner_token, workspace_id) = create_workspace_token(&pool, "owner").await;
    let reader_token = create_token_for_workspace(&pool, &workspace_id, "reader").await;
    let client = reqwest::Client::new();
    let base = server.base.clone();

    let info = client
        .post(format!("{}/v1/workspace/info", base))
        .bearer_auth(&owner_token)
        .json(&json!({}))
        .send()
        .await
        .expect("workspace info")
        .json::<serde_json::Value>()
        .await
        .expect("workspace info json");
    assert_eq!(info.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(
        info.get("workspace_id").and_then(|v| v.as_str()),
        Some(workspace_id.as_str())
    );

    let retention_id = format!("policy-retention-{}", uuid::Uuid::new_v4());
    let retention_write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "objects": [{
                "type":"policy.retention",
                "id": retention_id,
                "scope": workspace_id,
                "status":"canonical",
                "spec_version":"0.1",
                "tags":[],
                "data":{
                    "default_ttl_by_kind":{"decision": 3},
                    "max_ttl_by_kind":{"pii": 30},
                    "immutable_kinds":["pii"]
                },
                "provenance":{"actor":"owner"}
            }]
        }))
        .send()
        .await
        .expect("retention write")
        .json::<serde_json::Value>()
        .await
        .expect("retention write json");
    assert_eq!(
        retention_write.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let obj_id = format!("decision-{}", uuid::Uuid::new_v4());
    let write_decision = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "objects": [{
                "type":"decision",
                "id": obj_id,
                "scope": workspace_id,
                "status":"draft",
                "spec_version":"0.1",
                "tags":[],
                "memory_key":"decision:db",
                "data":{"choice":"pg"},
                "provenance":{"actor":"owner"},
                "lifecycle_state":"scratch"
            }]
        }))
        .send()
        .await
        .expect("decision write")
        .json::<serde_json::Value>()
        .await
        .expect("decision write json");
    assert_eq!(
        write_decision.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let read_decision = client
        .post(format!("{}/v1/read", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "scope": workspace_id,
            "refs": [obj_id],
            "include_states": ["scratch"],
            "include_expired": true
        }))
        .send()
        .await
        .expect("decision read")
        .json::<serde_json::Value>()
        .await
        .expect("decision read json");
    let expires = read_decision["objects"][0]["expires_at"].as_str();
    assert!(
        expires.is_some(),
        "retention default ttl should set expires_at"
    );

    let too_long = (chrono::Utc::now() + chrono::Duration::days(90)).to_rfc3339();
    let pii_write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "objects": [{
                "type":"pii",
                "id": format!("pii-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status":"draft",
                "spec_version":"0.1",
                "tags":[],
                "data":{"v":"secret"},
                "provenance":{"actor":"owner"},
                "expires_at": too_long
            }]
        }))
        .send()
        .await
        .expect("pii write")
        .json::<serde_json::Value>()
        .await
        .expect("pii write json");
    assert_eq!(
        pii_write["error"]["code"].as_str(),
        Some("OB_FORBIDDEN"),
        "immutable kind over max ttl must be denied"
    );
    assert!(pii_write["error"]["details"]["policy_rule_id"].is_string());
    assert!(pii_write["error"]["details"]["reason_code"].is_string());

    let audit_obj = client
        .post(format!("{}/v1/audit/object_timeline", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "scope": workspace_id,
            "object_id": read_decision["objects"][0]["id"],
            "limit": 20
        }))
        .send()
        .await
        .expect("audit obj")
        .json::<serde_json::Value>()
        .await
        .expect("audit obj json");
    assert_eq!(audit_obj.get("ok").and_then(|v| v.as_bool()), Some(true));
    assert!(audit_obj["events"]
        .as_array()
        .map(|v| !v.is_empty())
        .unwrap_or(false));

    let audit_key = client
        .post(format!("{}/v1/audit/memory_key_timeline", base))
        .bearer_auth(&owner_token)
        .json(&json!({"scope": workspace_id, "memory_key":"decision:db", "limit": 20}))
        .send()
        .await
        .expect("audit key")
        .json::<serde_json::Value>()
        .await
        .expect("audit key json");
    assert_eq!(audit_key.get("ok").and_then(|v| v.as_bool()), Some(true));

    let audit_actor = client
        .post(format!("{}/v1/audit/actor_activity", base))
        .bearer_auth(&owner_token)
        .json(&json!({"scope": workspace_id, "actor_identity_id":"owner", "limit": 20}))
        .send()
        .await
        .expect("audit actor")
        .json::<serde_json::Value>()
        .await
        .expect("audit actor json");
    assert_eq!(audit_actor.get("ok").and_then(|v| v.as_bool()), Some(true));

    let deny_audit_policy = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&json!({
            "objects": [{
                "type":"policy.rule",
                "id": format!("policy-{}", uuid::Uuid::new_v4()),
                "scope": workspace_id,
                "status":"canonical",
                "spec_version":"0.1",
                "tags":[],
                "data":{
                    "id":"deny-reader-audit",
                    "effect":"deny",
                    "operations":["audit_object_timeline"],
                    "roles":["reader"],
                    "reason":"OB_POLICY_DENY_AUDIT_READ"
                },
                "provenance":{"actor":"owner"}
            }]
        }))
        .send()
        .await
        .expect("deny audit policy")
        .json::<serde_json::Value>()
        .await
        .expect("deny audit policy json");
    assert_eq!(
        deny_audit_policy.get("ok").and_then(|v| v.as_bool()),
        Some(true)
    );

    let reader_denied = client
        .post(format!("{}/v1/audit/object_timeline", base))
        .bearer_auth(&reader_token)
        .json(&json!({
            "scope": workspace_id,
            "object_id": read_decision["objects"][0]["id"],
            "limit": 20
        }))
        .send()
        .await
        .expect("reader denied")
        .json::<serde_json::Value>()
        .await
        .expect("reader denied json");
    assert_eq!(
        reader_denied["error"]["code"].as_str(),
        Some("OB_FORBIDDEN")
    );
    assert_eq!(
        reader_denied["error"]["details"]["reason_code"].as_str(),
        Some("OB_POLICY_DENY_AUDIT_READ")
    );
    assert!(reader_denied["error"]["details"]["policy_rule_id"].is_string());

    server.shutdown().await;
}

#[tokio::test]
async fn http_memory_pack_policy_deny_and_clamp() {
    let Some(server) = TestServer::spawn().await else {
        return;
    };
    let pool = setup_pool().await.expect("pool");
    let (owner_token, workspace_id) = create_workspace_token(&pool, "owner").await;
    let writer_token = create_token_for_workspace(&pool, &workspace_id, "writer").await;
    let reader_token = create_token_for_workspace(&pool, &workspace_id, "reader").await;
    let client = reqwest::Client::new();
    let base = server.base.clone();

    let rule_deny_reader = json!({
        "objects": [{
            "type":"policy.rule",
            "id": format!("policy-{}", uuid::Uuid::new_v4()),
            "scope": workspace_id,
            "status":"canonical",
            "spec_version":"0.1",
            "tags":[],
            "data":{
                "id":"deny-reader-memory-pack",
                "effect":"deny",
                "operations":["memory_pack"],
                "roles":["reader"],
                "reason":"OB_POLICY_DENY_MEMORY_PACK"
            },
            "provenance":{"actor":"owner"}
        }]
    });
    let deny_write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&rule_deny_reader)
        .send()
        .await
        .expect("policy deny write")
        .json::<serde_json::Value>()
        .await
        .expect("policy deny write json");
    assert_eq!(deny_write["ok"].as_bool(), Some(true));

    let rule_clamp = json!({
        "objects": [{
            "type":"policy.rule",
            "id": format!("policy-{}", uuid::Uuid::new_v4()),
            "scope": workspace_id,
            "status":"canonical",
            "spec_version":"0.1",
            "tags":[],
            "data":{
                "id":"clamp-memory-pack-top-k",
                "effect":"allow",
                "operations":["memory_pack"],
                "roles":["writer"],
                "constraints":{"max_top_k":1},
                "reason":"OB_POLICY_ALLOW_MEMORY_PACK_CLAMP"
            },
            "provenance":{"actor":"owner"}
        }]
    });
    let clamp_write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&owner_token)
        .json(&rule_clamp)
        .send()
        .await
        .expect("policy clamp write")
        .json::<serde_json::Value>()
        .await
        .expect("policy clamp write json");
    assert_eq!(clamp_write["ok"].as_bool(), Some(true));

    let obj_a = format!("claim-{}", uuid::Uuid::new_v4());
    let obj_b = format!("claim-{}", uuid::Uuid::new_v4());
    let write = client
        .post(format!("{}/v1/write", base))
        .bearer_auth(&writer_token)
        .json(&json!({
            "objects":[
                {"type":"claim","id":obj_a,"scope":workspace_id,"status":"draft","spec_version":"0.1","tags":[],"data":{"k":"a"},"provenance":{"actor":"writer"},"lifecycle_state":"accepted"},
                {"type":"claim","id":obj_b,"scope":workspace_id,"status":"draft","spec_version":"0.1","tags":[],"data":{"k":"b"},"provenance":{"actor":"writer"},"lifecycle_state":"accepted"}
            ]
        }))
        .send()
        .await
        .expect("object write")
        .json::<serde_json::Value>()
        .await
        .expect("object write json");
    assert_eq!(write["ok"].as_bool(), Some(true));

    let denied = client
        .post(format!("{}/v1/memory/pack", base))
        .bearer_auth(&reader_token)
        .json(&json!({"scope": workspace_id, "task_hint":"test", "top_k": 10}))
        .send()
        .await
        .expect("pack denied")
        .json::<serde_json::Value>()
        .await
        .expect("pack denied json");
    assert_eq!(denied["error"]["code"].as_str(), Some("OB_FORBIDDEN"));
    assert_eq!(
        denied["error"]["details"]["reason_code"].as_str(),
        Some("OB_POLICY_DENY_MEMORY_PACK")
    );

    let clamped = client
        .post(format!("{}/v1/memory/pack", base))
        .bearer_auth(&writer_token)
        .json(&json!({"scope": workspace_id, "task_hint":"test", "top_k": 10, "semantic": false}))
        .send()
        .await
        .expect("pack clamped")
        .json::<serde_json::Value>()
        .await
        .expect("pack clamped json");
    assert_eq!(clamped["ok"].as_bool(), Some(true));
    assert_eq!(clamped["pack"]["items_selected"].as_u64(), Some(1));

    server.shutdown().await;
}
