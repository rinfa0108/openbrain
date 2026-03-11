use async_trait::async_trait;
use openbrain_core::query::{parse_where, CmpOp, Expr, FieldPath, Literal, Predicate};
use openbrain_core::textnorm::{
    canonicalize_json, checksum_v01, normalize_object_text, validate_embedding_text, value_hash_v01,
};
use openbrain_core::{
    ConflictStatus, Envelope, ErrorCode, ErrorEnvelope, LifecycleState, MemoryObjectStored,
    ValidatedMemoryObject,
};
use openbrain_embed::{EmbedError, EmbeddingProvider, NoopEmbeddingProvider};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder, Row, Transaction};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    AuditActorActivityRequest, AuditEvent, AuditMemoryKeyTimelineRequest,
    AuditObjectTimelineRequest, AuditResponse, AuthContext, AuthStore, BootstrapToken,
    EmbedGenerateRequest, EmbedGenerateResponse, EmbedTarget, EmbeddingCoverageRequest,
    EmbeddingCoverageResponse, EmbeddingReembedRequest, EmbeddingReembedResponse,
    GetObjectsRequest, GetObjectsResponse, OrderBySpec, PutObjectsRequest, PutObjectsResponse,
    PutResult, SearchItem, SearchMatch, SearchSemanticRequest, SearchSemanticResponse,
    SearchStructuredRequest, SearchStructuredResponse, Store, TokenCreateRequest,
    TokenCreateResponse, WorkspaceInfoResponse, WorkspaceRole,
};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;
const MAX_AUDIT_LIMIT: u32 = 200;

const EMBEDDING_DIMS: i32 = 1536;
const MAX_EMBED_TEXT_LEN: usize = 32 * 1024;
const DEFAULT_EMBED_PROVIDER: &str = "noop";
const DEFAULT_EMBED_KIND: &str = "semantic";
const DEFAULT_EMBED_MODEL: &str = "default";
const DEFAULT_WORKSPACE_ID: &str = "default";
const DEFAULT_WORKSPACE_NAME: &str = "Default Workspace";
const DEFAULT_REEMBED_LIMIT: u32 = 100;
const MAX_REEMBED_LIMIT: u32 = 500;
const MAX_REEMBED_MISSING_SAMPLE: u32 = 25;
const MAX_IDEMPOTENCY_KEY_LEN: usize = 256;
const RECEIPT_OBJECT_IDS_CAP: usize = 50;

fn embedding_to_pgvector_literal(embedding: &[f32]) -> Result<String, ErrorEnvelope> {
    let mut out = String::with_capacity(embedding.len() * 12 + 2);
    out.push('[');
    for (i, v) in embedding.iter().enumerate() {
        if !v.is_finite() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObEmbeddingFailed,
                "embedding contains non-finite values",
                Some(serde_json::json!({"index": i})),
            ));
        }
        if i > 0 {
            out.push(',');
        }
        out.push_str(&v.to_string());
    }
    out.push(']');
    Ok(out)
}

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
    embedder: Arc<dyn EmbeddingProvider>,
    embedding_provider: String,
    embedding_kind: String,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Ok(Self {
            pool,
            embedder: Arc::new(NoopEmbeddingProvider),
            embedding_provider: DEFAULT_EMBED_PROVIDER.to_string(),
            embedding_kind: DEFAULT_EMBED_KIND.to_string(),
        })
    }

    pub async fn connect_with_embedder(
        database_url: &str,
        embedder: Arc<dyn EmbeddingProvider>,
    ) -> Result<Self, sqlx::Error> {
        Self::connect_with_embedder_and_provider(database_url, embedder, DEFAULT_EMBED_PROVIDER)
            .await
    }

    pub async fn connect_with_embedder_and_provider(
        database_url: &str,
        embedder: Arc<dyn EmbeddingProvider>,
        embedding_provider: &str,
    ) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        let provider = embedding_provider.trim();
        let provider = if provider.is_empty() {
            DEFAULT_EMBED_PROVIDER
        } else {
            provider
        };
        Ok(Self {
            pool,
            embedder,
            embedding_provider: provider.to_string(),
            embedding_kind: DEFAULT_EMBED_KIND.to_string(),
        })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            pool,
            embedder: Arc::new(NoopEmbeddingProvider),
            embedding_provider: DEFAULT_EMBED_PROVIDER.to_string(),
            embedding_kind: DEFAULT_EMBED_KIND.to_string(),
        }
    }

    pub fn from_pool_with_embedder(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self::from_pool_with_embedder_and_provider(pool, embedder, DEFAULT_EMBED_PROVIDER)
    }

    pub fn from_pool_with_embedder_and_provider(
        pool: PgPool,
        embedder: Arc<dyn EmbeddingProvider>,
        embedding_provider: &str,
    ) -> Self {
        let provider = embedding_provider.trim();
        let provider = if provider.is_empty() {
            DEFAULT_EMBED_PROVIDER
        } else {
            provider
        };
        Self {
            pool,
            embedder,
            embedding_provider: provider.to_string(),
            embedding_kind: DEFAULT_EMBED_KIND.to_string(),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn embedding_coverage(
        &self,
        req: EmbeddingCoverageRequest,
    ) -> Result<EmbeddingCoverageResponse, ErrorEnvelope> {
        let scope = req.scope.trim();
        if scope.is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }
        let provider = req.provider.trim();
        let model = req.model.trim();
        let kind = req.kind.trim();
        if provider.is_empty() || model.is_empty() || kind.is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "provider, model, and kind are required",
                None,
            ));
        }

        let now = chrono::Utc::now();
        let state = req.state.as_str();
        let sample_limit = req
            .missing_sample_limit
            .unwrap_or(10)
            .min(MAX_REEMBED_MISSING_SAMPLE);

        let total_eligible: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
               FROM ob_objects o
               WHERE o.scope = $1
                 AND o.lifecycle_state = $2
                 AND (o.expires_at IS NULL OR o.expires_at > $3)"#,
        )
        .bind(scope)
        .bind(state)
        .bind(now)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("coverage total query failed: {e}"),
                None,
            )
        })?;

        let with_embeddings: i64 = sqlx::query_scalar(
            r#"SELECT COUNT(*)
               FROM ob_objects o
               WHERE o.scope = $1
                 AND o.lifecycle_state = $2
                 AND (o.expires_at IS NULL OR o.expires_at > $3)
                 AND EXISTS (
                   SELECT 1 FROM ob_embeddings e
                   WHERE e.scope = o.scope
                     AND e.object_id = o.id
                     AND e.provider = $4
                     AND e.model = $5
                     AND e.kind = $6
                 )"#,
        )
        .bind(scope)
        .bind(state)
        .bind(now)
        .bind(provider)
        .bind(model)
        .bind(kind)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("coverage query failed: {e}"),
                None,
            )
        })?;

        let missing_refs: Vec<String> = sqlx::query_scalar(
            r#"SELECT o.id
               FROM ob_objects o
               WHERE o.scope = $1
                 AND o.lifecycle_state = $2
                 AND (o.expires_at IS NULL OR o.expires_at > $3)
                 AND NOT EXISTS (
                   SELECT 1 FROM ob_embeddings e
                   WHERE e.scope = o.scope
                     AND e.object_id = o.id
                     AND e.provider = $4
                     AND e.model = $5
                     AND e.kind = $6
                 )
               ORDER BY o.id ASC
               LIMIT $7"#,
        )
        .bind(scope)
        .bind(state)
        .bind(now)
        .bind(provider)
        .bind(model)
        .bind(kind)
        .bind(sample_limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("coverage missing sample query failed: {e}"),
                None,
            )
        })?;

        let total_eligible_u = total_eligible.max(0) as u64;
        let with_embeddings_u = with_embeddings.max(0) as u64;
        let missing = total_eligible_u.saturating_sub(with_embeddings_u);
        let percent_coverage = if total_eligible_u == 0 {
            100.0
        } else {
            (with_embeddings_u as f64 * 100.0) / total_eligible_u as f64
        };

        Ok(EmbeddingCoverageResponse {
            total_eligible: total_eligible_u,
            with_embeddings: with_embeddings_u,
            missing,
            percent_coverage,
            missing_refs,
        })
    }

    pub async fn reembed_missing(
        &self,
        req: EmbeddingReembedRequest,
    ) -> Result<EmbeddingReembedResponse, ErrorEnvelope> {
        let scope = req.scope.trim();
        if scope.is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }

        let provider = req.to_provider.trim();
        let model = req.to_model.trim();
        let kind = req.to_kind.trim();
        if provider.is_empty() || model.is_empty() || kind.is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "to_provider, to_model, and to_kind are required",
                None,
            ));
        }

        let limit = req
            .limit
            .unwrap_or(DEFAULT_REEMBED_LIMIT)
            .clamp(1, MAX_REEMBED_LIMIT);
        let max_objects = req.max_objects.unwrap_or(limit).clamp(1, limit);
        let max_bytes = req.max_bytes.unwrap_or(u64::MAX);
        let now = chrono::Utc::now();

        #[derive(sqlx::FromRow)]
        struct CandidateRow {
            id: String,
            r#type: String,
            data: sqlx::types::Json<Value>,
        }

        let rows: Vec<CandidateRow> = sqlx::query_as(
            r#"SELECT o.id, o.type, o.data
               FROM ob_objects o
               WHERE o.scope = $1
                 AND o.lifecycle_state = $2
                 AND (o.expires_at IS NULL OR o.expires_at > $3)
                 AND ($4::text IS NULL OR o.id > $4)
                 AND NOT EXISTS (
                   SELECT 1 FROM ob_embeddings e
                   WHERE e.scope = o.scope
                     AND e.object_id = o.id
                     AND e.provider = $5
                     AND e.model = $6
                     AND e.kind = $7
                 )
               ORDER BY o.id ASC
               LIMIT $8"#,
        )
        .bind(scope)
        .bind(req.state.as_str())
        .bind(now)
        .bind(req.after.as_deref())
        .bind(provider)
        .bind(model)
        .bind(kind)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("reembed candidate query failed: {e}"),
                None,
            )
        })?;

        let scanned = rows.len() as u32;
        let mut processed: u32 = 0;
        let mut bytes_processed: u64 = 0;
        let mut last_processed_id: Option<String> = None;

        for row in rows {
            if processed >= max_objects {
                break;
            }

            let normalized = normalize_object_text(&row.r#type, &row.data.0)?;
            let normalized = validate_embedding_text(&normalized, MAX_EMBED_TEXT_LEN)?;
            let text_bytes = normalized.len() as u64;
            if bytes_processed.saturating_add(text_bytes) > max_bytes {
                break;
            }

            if !req.dry_run {
                let checksum = checksum_v01(&normalized);
                let embedding = self.embedder.embed(model, &normalized).await.map_err(|e| {
                    let (code, message, details) = match e {
                        EmbedError::ProviderUnavailable => (
                            ErrorCode::ObEmbeddingFailed,
                            "embedding provider unavailable".to_string(),
                            Some(serde_json::json!({"reason": "provider_unavailable"})),
                        ),
                        EmbedError::InvalidInput(m) => (ErrorCode::ObEmbeddingFailed, m, None),
                        EmbedError::InvalidRequest {
                            message, details, ..
                        } => (ErrorCode::ObInvalidRequest, message, details),
                        EmbedError::ProviderError {
                            message, details, ..
                        } => (ErrorCode::ObEmbeddingFailed, message, details),
                    };
                    ErrorEnvelope::new(code, message, details)
                })?;

                let got_dims = embedding.len() as i32;
                if got_dims != EMBEDDING_DIMS {
                    return Err(ErrorEnvelope::new(
                        ErrorCode::ObEmbeddingFailed,
                        "provider returned wrong embedding dims",
                        Some(serde_json::json!({"expected": EMBEDDING_DIMS, "got": got_dims})),
                    ));
                }
                let vector_text = embedding_to_pgvector_literal(&embedding)?;
                let embedding_id = Uuid::new_v4().to_string();
                sqlx::query(
                    r#"INSERT INTO ob_embeddings
                       (id, object_id, scope, provider, model, kind, dims, checksum, embedding)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, ($9)::vector)
                       ON CONFLICT (scope, object_id, provider, model, kind)
                       WHERE object_id IS NOT NULL
                       DO UPDATE SET
                         checksum = EXCLUDED.checksum,
                         dims = EXCLUDED.dims,
                         embedding = EXCLUDED.embedding,
                         created_at = now()"#,
                )
                .bind(&embedding_id)
                .bind(&row.id)
                .bind(scope)
                .bind(provider)
                .bind(model)
                .bind(kind)
                .bind(EMBEDDING_DIMS)
                .bind(checksum)
                .bind(vector_text)
                .execute(&self.pool)
                .await
                .map_err(|e| {
                    ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("failed to upsert embedding: {e}"),
                        None,
                    )
                })?;
            }

            processed += 1;
            bytes_processed = bytes_processed.saturating_add(text_bytes);
            last_processed_id = Some(row.id);
        }

        if !req.dry_run && processed > 0 {
            let actor = req
                .actor
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("system.reembed");
            self.append_event(
                scope,
                "embed.reembed.batch",
                actor,
                serde_json::json!({
                    "scope": scope,
                    "provider": provider,
                    "model": model,
                    "kind": kind,
                    "processed": processed,
                    "bytes_processed": bytes_processed,
                }),
            )
            .await;
        }

        Ok(EmbeddingReembedResponse {
            scanned,
            processed,
            next_cursor: last_processed_id,
            dry_run: req.dry_run,
            bytes_processed,
        })
    }

    fn normalize_opt_field(value: Option<&str>) -> Option<String> {
        value
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    fn parse_datetime(
        label: &str,
        value: &str,
    ) -> Result<chrono::DateTime<chrono::Utc>, ErrorEnvelope> {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .map_err(|_| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    format!("invalid {label} timestamp"),
                    Some(serde_json::json!({ "value": value })),
                )
            })
    }

    fn parse_datetime_opt(
        label: &str,
        value: Option<&str>,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, ErrorEnvelope> {
        match value.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(v) => Ok(Some(Self::parse_datetime(label, v)?)),
        }
    }

    fn states_or_default(states: Option<Vec<LifecycleState>>) -> Vec<LifecycleState> {
        states.unwrap_or_else(|| vec![LifecycleState::Accepted])
    }

    fn to_state_strings(states: &[LifecycleState]) -> Vec<String> {
        states.iter().map(|s| s.as_str().to_string()).collect()
    }

    async fn load_conflicts(
        &self,
        scope: &str,
        memory_keys: &[String],
        state_strings: &[String],
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<std::collections::HashMap<String, ConflictInfo>, ErrorEnvelope> {
        if memory_keys.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        #[derive(sqlx::FromRow)]
        struct ConflictRow {
            memory_key: String,
            distinct_count: i64,
            total_count: i64,
            ids: Vec<String>,
        }

        let rows: Vec<ConflictRow> = sqlx::query_as(
            r#"SELECT memory_key,
                      COUNT(DISTINCT value_hash) AS distinct_count,
                      COUNT(*) AS total_count,
                      ARRAY_AGG(id) AS ids
               FROM ob_objects
               WHERE scope = $1
                 AND memory_key = ANY($2)
                 AND lifecycle_state = ANY($3)
                 AND (expires_at IS NULL OR expires_at > $4)
               GROUP BY memory_key"#,
        )
        .bind(scope)
        .bind(memory_keys)
        .bind(state_strings)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("conflict lookup failed: {e}"),
                None,
            )
        })?;

        let mut out = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            out.insert(
                row.memory_key.clone(),
                ConflictInfo {
                    distinct_count: row.distinct_count,
                    total_count: row.total_count,
                    ids: row.ids,
                },
            );
        }
        Ok(out)
    }

    async fn upsert_object(
        tx: &mut Transaction<'_, Postgres>,
        obj: &ValidatedMemoryObject,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
        value_hash: Option<String>,
    ) -> Result<i64, ErrorEnvelope> {
        let existing = sqlx::query(
            r#"SELECT version
               FROM ob_objects
               WHERE id = $1
               FOR UPDATE"#,
        )
        .bind(&obj.id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("object lookup failed: {e}"),
                None,
            )
        })?;

        match existing {
            None => {
                let version: i64 = sqlx::query_scalar(
                    r#"INSERT INTO ob_objects
                       (id, scope, type, status, spec_version, tags, data, provenance, version,
                        lifecycle_state, expires_at, memory_key, value_hash, conflict_status,
                        resolved_by_object_id, resolved_at, resolution_note)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1, $9, $10, $11, $12, $13, $14, $15, $16)
                       RETURNING version"#,
                )
                .bind(&obj.id)
                .bind(&obj.scope)
                .bind(&obj.object_type)
                .bind(&obj.status)
                .bind(&obj.spec_version)
                .bind(&obj.tags)
                .bind(sqlx::types::Json(&obj.data))
                .bind(sqlx::types::Json(&obj.provenance))
                .bind(obj.lifecycle_state.as_str())
                .bind(expires_at)
                .bind(obj.memory_key.as_deref())
                .bind(value_hash)
                .bind(obj.conflict_status.as_str())
                .bind(obj.resolved_by_object_id.as_deref())
                .bind(Self::parse_datetime_opt(
                    "resolved_at",
                    obj.resolved_at.as_deref(),
                )?)
                .bind(obj.resolution_note.as_deref())
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| {
                    ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("object insert failed: {e}"),
                        None,
                    )
                })?;
                Ok(version)
            }
            Some(row) => {
                let current_version: i64 = row.try_get("version").map_err(|e| {
                    ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("object version read failed: {e}"),
                        None,
                    )
                })?;
                let new_version = current_version + 1;
                let version: i64 = sqlx::query_scalar(
                    r#"UPDATE ob_objects
                       SET scope = $2,
                           type = $3,
                           status = $4,
                           spec_version = $5,
                           tags = $6,
                           data = $7,
                           provenance = $8,
                           version = $9,
                           lifecycle_state = $10,
                           expires_at = $11,
                           memory_key = $12,
                           value_hash = $13,
                           conflict_status = $14,
                           resolved_by_object_id = $15,
                           resolved_at = $16,
                           resolution_note = $17,
                           updated_at = now()
                       WHERE id = $1
                       RETURNING version"#,
                )
                .bind(&obj.id)
                .bind(&obj.scope)
                .bind(&obj.object_type)
                .bind(&obj.status)
                .bind(&obj.spec_version)
                .bind(&obj.tags)
                .bind(sqlx::types::Json(&obj.data))
                .bind(sqlx::types::Json(&obj.provenance))
                .bind(new_version)
                .bind(obj.lifecycle_state.as_str())
                .bind(expires_at)
                .bind(obj.memory_key.as_deref())
                .bind(value_hash)
                .bind(obj.conflict_status.as_str())
                .bind(obj.resolved_by_object_id.as_deref())
                .bind(Self::parse_datetime_opt(
                    "resolved_at",
                    obj.resolved_at.as_deref(),
                )?)
                .bind(obj.resolution_note.as_deref())
                .fetch_one(&mut **tx)
                .await
                .map_err(|e| {
                    ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("object update failed: {e}"),
                        None,
                    )
                })?;
                Ok(version)
            }
        }
    }

    async fn insert_event(
        tx: &mut Transaction<'_, Postgres>,
        scope: &str,
        event_type: &str,
        actor: &str,
        payload: &Value,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO ob_events (scope, event_type, actor, payload)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(scope)
        .bind(event_type)
        .bind(actor)
        .bind(sqlx::types::Json(payload))
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    fn parse_audit_ts(
        label: &str,
        value: Option<&str>,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, ErrorEnvelope> {
        Self::parse_datetime_opt(label, value)
    }

    fn event_summary(event_type: &str, payload: &Value) -> Option<String> {
        let object_id = payload.get("ref").and_then(|v| v.as_str()).unwrap_or("");
        if object_id.is_empty() {
            return Some(event_type.to_string());
        }
        Some(format!("{event_type}:{object_id}"))
    }

    fn audit_event_from_row(
        id: i64,
        event_type: String,
        actor: String,
        payload: Value,
        ts: chrono::DateTime<chrono::Utc>,
        memory_key: Option<String>,
    ) -> AuditEvent {
        let object_id = payload
            .get("ref")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let object_version = payload.get("version").and_then(|v| v.as_i64());
        let summary = Self::event_summary(&event_type, &payload);
        AuditEvent {
            id,
            event_type,
            actor_identity_id: actor,
            object_id,
            object_version,
            memory_key,
            ts: ts.to_rfc3339(),
            summary,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldType {
    Text,
    Timestamp,
    JsonText,
    JsonTimestamp,
    Tags,
}

#[derive(Debug, Clone)]
struct ConflictInfo {
    distinct_count: i64,
    total_count: i64,
    ids: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct IdempotencyRow {
    request_hash: String,
    receipt_hash: String,
    accepted_count: i64,
    object_ids: Vec<String>,
    results_json: sqlx::types::Json<Vec<PutResult>>,
}

fn compute_request_hash(
    scope: &str,
    actor: Option<&str>,
    objects: &[ValidatedMemoryObject],
) -> String {
    let canonical_objects: Vec<Value> = objects
        .iter()
        .map(|obj| {
            canonicalize_json(&serde_json::json!({
                "type": obj.object_type,
                "id": obj.id,
                "scope": obj.scope,
                "status": obj.status,
                "spec_version": obj.spec_version,
                "tags": obj.tags,
                "data": obj.data,
                "provenance": obj.provenance,
                "lifecycle_state": obj.lifecycle_state.as_str(),
                "expires_at": obj.expires_at,
                "memory_key": obj.memory_key,
                "conflict_status": obj.conflict_status.as_str(),
                "resolved_by_object_id": obj.resolved_by_object_id,
                "resolved_at": obj.resolved_at,
                "resolution_note": obj.resolution_note,
            }))
        })
        .collect();
    let canonical = canonicalize_json(&serde_json::json!({
        "scope": scope,
        "actor": actor.map(str::trim).filter(|v| !v.is_empty()),
        "objects": canonical_objects,
    }));
    let payload = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"ob.idempotency.request.v1\n");
    hasher.update(&payload);
    hex_encode(&hasher.finalize())
}

fn compute_receipt_hash(
    request_hash: &str,
    accepted_count: usize,
    object_ids: &[String],
    results: &[PutResult],
) -> String {
    let canonical = canonicalize_json(&serde_json::json!({
        "request_hash": request_hash,
        "accepted_count": accepted_count,
        "object_ids": object_ids,
        "results": results,
    }));
    let payload = serde_json::to_vec(&canonical).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(b"ob.idempotency.receipt.v1\n");
    hasher.update(&payload);
    hex_encode(&hasher.finalize())
}

fn make_put_response(
    results: Vec<PutResult>,
    replayed: bool,
    request_hash: Option<String>,
    receipt_hash: Option<String>,
) -> PutObjectsResponse {
    let accepted_count = results.len();
    let mut object_ids: Vec<String> = results.iter().map(|r| r.r#ref.clone()).collect();
    if object_ids.len() > RECEIPT_OBJECT_IDS_CAP {
        object_ids.truncate(RECEIPT_OBJECT_IDS_CAP);
    }
    PutObjectsResponse {
        results,
        replayed,
        request_id: request_hash,
        accepted_count,
        object_ids,
        receipt_hash,
    }
}

fn include_conflicts_enabled(flag: Option<bool>) -> bool {
    flag.unwrap_or(true)
}

fn default_expiry_for_state(
    state: LifecycleState,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    match state {
        LifecycleState::Scratch => Some(now + chrono::Duration::days(7)),
        LifecycleState::Candidate => Some(now + chrono::Duration::days(30)),
        LifecycleState::Accepted | LifecycleState::Deprecated => None,
    }
}

fn parse_audit_limit_offset(limit: Option<u32>, offset: Option<u32>) -> (u32, u32) {
    (
        limit.unwrap_or(DEFAULT_LIMIT).min(MAX_AUDIT_LIMIT),
        offset.unwrap_or(0),
    )
}

fn field_to_sql(field: &FieldPath) -> Result<(String, FieldType), ErrorEnvelope> {
    let segs = &field.segments;
    if segs.is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "empty field path",
            None,
        ));
    }

    match segs[0].as_str() {
        "id" => Ok(("id".to_string(), FieldType::Text)),
        "scope" => Ok(("scope".to_string(), FieldType::Text)),
        "type" => Ok(("type".to_string(), FieldType::Text)),
        "status" => Ok(("status".to_string(), FieldType::Text)),
        "spec_version" => Ok(("spec_version".to_string(), FieldType::Text)),
        "created_at" => Ok(("created_at".to_string(), FieldType::Timestamp)),
        "updated_at" => Ok(("updated_at".to_string(), FieldType::Timestamp)),
        "lifecycle_state" => Ok(("lifecycle_state".to_string(), FieldType::Text)),
        "expires_at" => Ok(("expires_at".to_string(), FieldType::Timestamp)),
        "memory_key" => Ok(("memory_key".to_string(), FieldType::Text)),
        "tags" => {
            if segs.len() != 1 {
                return Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "tags does not support subfields",
                    None,
                ));
            }
            Ok(("tags".to_string(), FieldType::Tags))
        }
        "data" => {
            if segs.len() < 2 {
                return Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "data requires a field path",
                    None,
                ));
            }

            let path = segs[1..]
                .iter()
                .map(|s| {
                    if s.is_empty() {
                        Err(ErrorEnvelope::new(
                            ErrorCode::ObInvalidRequest,
                            "invalid data path segment",
                            None,
                        ))
                    } else {
                        Ok(s.clone())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            let pg_path = path.join(",");
            Ok((format!("data #>> '{{{pg_path}}}'"), FieldType::JsonText))
        }
        "provenance" => {
            if segs.len() == 2 && segs[1] == "ts" {
                Ok((
                    "provenance #>> '{ts}'".to_string(),
                    FieldType::JsonTimestamp,
                ))
            } else {
                Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "only provenance.ts is supported in v0.1",
                    None,
                ))
            }
        }
        _ => Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "unknown field",
            Some(serde_json::json!({"field": segs.join(".")})),
        )),
    }
}

fn build_expr(qb: &mut QueryBuilder<'_, Postgres>, expr: &Expr) -> Result<(), ErrorEnvelope> {
    match expr {
        Expr::And(items) => {
            qb.push("(");
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    qb.push(" AND ");
                }
                build_expr(qb, it)?;
            }
            qb.push(")");
            Ok(())
        }
        Expr::Or(items) => {
            qb.push("(");
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                build_expr(qb, it)?;
            }
            qb.push(")");
            Ok(())
        }
        Expr::Not(inner) => {
            qb.push("(NOT ");
            build_expr(qb, inner)?;
            qb.push(")");
            Ok(())
        }
        Expr::Pred(p) => build_pred(qb, p),
    }
}

fn push_cmp_op(qb: &mut QueryBuilder<'_, Postgres>, op: CmpOp) {
    match op {
        CmpOp::Eq => qb.push(" = "),
        CmpOp::Ne => qb.push(" <> "),
        CmpOp::Gt => qb.push(" > "),
        CmpOp::Gte => qb.push(" >= "),
        CmpOp::Lt => qb.push(" < "),
        CmpOp::Lte => qb.push(" <= "),
    };
}

fn build_pred(qb: &mut QueryBuilder<'_, Postgres>, pred: &Predicate) -> Result<(), ErrorEnvelope> {
    match pred {
        Predicate::Compare { field, op, value } => {
            let (field_sql, field_type) = field_to_sql(field)?;
            match (field_type, value) {
                (FieldType::Tags, _) => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "tags only supports IN [..]",
                    None,
                )),
                (FieldType::Timestamp, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push("(");
                    qb.push_bind(s.clone());
                    qb.push("::timestamptz))");
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonTimestamp, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::timestamptz");
                    push_cmp_op(qb, *op);
                    qb.push("(");
                    qb.push_bind(s.clone());
                    qb.push("::timestamptz))");
                    qb.push(")");
                    Ok(())
                }
                (_, Literal::Null) => match op {
                    CmpOp::Eq => {
                        qb.push("(");
                        qb.push(field_sql);
                        qb.push(" IS NULL)");
                        Ok(())
                    }
                    CmpOp::Ne => {
                        qb.push("(");
                        qb.push(field_sql);
                        qb.push(" IS NOT NULL)");
                        Ok(())
                    }
                    _ => Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "NULL only supports == or !=",
                        None,
                    )),
                },
                (FieldType::JsonText, Literal::Number(n)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::double precision");
                    push_cmp_op(qb, *op);
                    qb.push_bind(*n);
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonText, Literal::Bool(b)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::boolean");
                    push_cmp_op(qb, *op);
                    qb.push_bind(*b);
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonText, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push_bind(s.clone());
                    qb.push(")");
                    Ok(())
                }
                (FieldType::Text, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push_bind(s.clone());
                    qb.push(")");
                    Ok(())
                }
                (FieldType::Text, _) => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "text fields require string literals",
                    None,
                )),
                (FieldType::Timestamp, _) | (FieldType::JsonTimestamp, _) => {
                    Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "timestamp fields require string literals",
                        None,
                    ))
                }
            }
        }
        Predicate::In { field, values } => {
            let (field_sql, field_type) = field_to_sql(field)?;
            let strings: Vec<String> = values
                .iter()
                .map(|v| match v {
                    Literal::String(s) => Ok(s.clone()),
                    _ => Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "IN only supports string literals",
                        None,
                    )),
                })
                .collect::<Result<_, _>>()?;

            match field_type {
                FieldType::Tags => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push(" && ");
                    qb.push_bind(strings);
                    qb.push(")");
                    Ok(())
                }
                FieldType::Text | FieldType::JsonText => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push(" = ANY(");
                    qb.push_bind(strings);
                    qb.push(")");
                    qb.push(")");
                    Ok(())
                }
                FieldType::Timestamp | FieldType::JsonTimestamp => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "IN is not supported for timestamp fields in v0.1",
                    None,
                )),
            }
        }
    }
}

fn field_to_sql_with_alias(
    field: &FieldPath,
    alias: &str,
) -> Result<(String, FieldType), ErrorEnvelope> {
    let segs = &field.segments;
    if segs.is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "empty field path",
            None,
        ));
    }

    let col = |name: &str| format!("{alias}.{name}");

    match segs[0].as_str() {
        "id" => Ok((col("id"), FieldType::Text)),
        "scope" => Ok((col("scope"), FieldType::Text)),
        "type" => Ok((col("type"), FieldType::Text)),
        "status" => Ok((col("status"), FieldType::Text)),
        "spec_version" => Ok((col("spec_version"), FieldType::Text)),
        "created_at" => Ok((col("created_at"), FieldType::Timestamp)),
        "updated_at" => Ok((col("updated_at"), FieldType::Timestamp)),
        "lifecycle_state" => Ok((col("lifecycle_state"), FieldType::Text)),
        "expires_at" => Ok((col("expires_at"), FieldType::Timestamp)),
        "memory_key" => Ok((col("memory_key"), FieldType::Text)),
        "tags" => {
            if segs.len() != 1 {
                return Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "tags does not support subfields",
                    None,
                ));
            }
            Ok((col("tags"), FieldType::Tags))
        }
        "data" => {
            if segs.len() < 2 {
                return Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "data requires a field path",
                    None,
                ));
            }

            let path = segs[1..]
                .iter()
                .map(|s| {
                    if s.is_empty() {
                        Err(ErrorEnvelope::new(
                            ErrorCode::ObInvalidRequest,
                            "invalid data path segment",
                            None,
                        ))
                    } else {
                        Ok(s.clone())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;

            let pg_path = path.join(",");
            Ok((
                format!("{alias}.data #>> '{{{pg_path}}}'"),
                FieldType::JsonText,
            ))
        }
        "provenance" => {
            if segs.len() == 2 && segs[1] == "ts" {
                Ok((
                    format!("{alias}.provenance #>> '{{ts}}'"),
                    FieldType::JsonTimestamp,
                ))
            } else {
                Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "only provenance.ts is supported in v0.1",
                    None,
                ))
            }
        }
        _ => Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "unknown field",
            Some(serde_json::json!({"field": segs.join(".")})),
        )),
    }
}

fn build_expr_with_alias(
    qb: &mut QueryBuilder<'_, Postgres>,
    expr: &Expr,
    alias: &str,
) -> Result<(), ErrorEnvelope> {
    match expr {
        Expr::And(items) => {
            qb.push("(");
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    qb.push(" AND ");
                }
                build_expr_with_alias(qb, it, alias)?;
            }
            qb.push(")");
            Ok(())
        }
        Expr::Or(items) => {
            qb.push("(");
            for (i, it) in items.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                build_expr_with_alias(qb, it, alias)?;
            }
            qb.push(")");
            Ok(())
        }
        Expr::Not(inner) => {
            qb.push("(NOT ");
            build_expr_with_alias(qb, inner, alias)?;
            qb.push(")");
            Ok(())
        }
        Expr::Pred(p) => build_pred_with_alias(qb, p, alias),
    }
}

fn build_pred_with_alias(
    qb: &mut QueryBuilder<'_, Postgres>,
    pred: &Predicate,
    alias: &str,
) -> Result<(), ErrorEnvelope> {
    match pred {
        Predicate::Compare { field, op, value } => {
            let (field_sql, field_type) = field_to_sql_with_alias(field, alias)?;
            match (field_type, value) {
                (FieldType::Tags, _) => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "tags only supports IN [..]",
                    None,
                )),
                (FieldType::Timestamp, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push("(");
                    qb.push_bind(s.clone());
                    qb.push("::timestamptz))");
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonTimestamp, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::timestamptz");
                    push_cmp_op(qb, *op);
                    qb.push("(");
                    qb.push_bind(s.clone());
                    qb.push("::timestamptz))");
                    qb.push(")");
                    Ok(())
                }
                (_, Literal::Null) => match op {
                    CmpOp::Eq => {
                        qb.push("(");
                        qb.push(field_sql);
                        qb.push(" IS NULL)");
                        Ok(())
                    }
                    CmpOp::Ne => {
                        qb.push("(");
                        qb.push(field_sql);
                        qb.push(" IS NOT NULL)");
                        Ok(())
                    }
                    _ => Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "NULL only supports == or !=",
                        None,
                    )),
                },
                (FieldType::JsonText, Literal::Number(n)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::double precision");
                    push_cmp_op(qb, *op);
                    qb.push_bind(*n);
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonText, Literal::Bool(b)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push("::boolean");
                    push_cmp_op(qb, *op);
                    qb.push_bind(*b);
                    qb.push(")");
                    Ok(())
                }
                (FieldType::JsonText, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push_bind(s.clone());
                    qb.push(")");
                    Ok(())
                }
                (FieldType::Text, Literal::String(s)) => {
                    qb.push("(");
                    qb.push(field_sql);
                    push_cmp_op(qb, *op);
                    qb.push_bind(s.clone());
                    qb.push(")");
                    Ok(())
                }
                (FieldType::Text, _) => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "text fields require string literals",
                    None,
                )),
                (FieldType::Timestamp, _) | (FieldType::JsonTimestamp, _) => {
                    Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "timestamp fields require string literals",
                        None,
                    ))
                }
            }
        }
        Predicate::In { field, values } => {
            let (field_sql, field_type) = field_to_sql_with_alias(field, alias)?;
            let strings: Vec<String> = values
                .iter()
                .map(|v| match v {
                    Literal::String(s) => Ok(s.clone()),
                    _ => Err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "IN only supports string literals",
                        None,
                    )),
                })
                .collect::<Result<_, _>>()?;

            match field_type {
                FieldType::Tags => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push(" && ");
                    qb.push_bind(strings);
                    qb.push(")");
                    Ok(())
                }
                FieldType::Text | FieldType::JsonText => {
                    qb.push("(");
                    qb.push(field_sql);
                    qb.push(" = ANY(");
                    qb.push_bind(strings);
                    qb.push(")");
                    qb.push(")");
                    Ok(())
                }
                FieldType::Timestamp | FieldType::JsonTimestamp => Err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "IN is not supported for timestamp fields in v0.1",
                    None,
                )),
            }
        }
    }
}
fn order_by_sql(order_by: &Option<OrderBySpec>) -> Result<&'static str, ErrorEnvelope> {
    let Some(ob) = order_by else {
        return Ok("updated_at DESC");
    };

    match (ob.field.as_str(), ob.direction) {
        ("updated_at", crate::OrderDirection::Desc) => Ok("updated_at DESC"),
        ("updated_at", crate::OrderDirection::Asc) => Ok("updated_at ASC"),
        ("created_at", crate::OrderDirection::Desc) => Ok("created_at DESC"),
        ("created_at", crate::OrderDirection::Asc) => Ok("created_at ASC"),
        _ => Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "unsupported order_by field",
            Some(serde_json::json!({"field": ob.field})),
        )),
    }
}

#[async_trait]
impl Store for PgStore {
    async fn put_objects(&self, req: PutObjectsRequest) -> Envelope<PutObjectsResponse> {
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("failed to start transaction: {e}"),
                    None,
                ))
            }
        };

        let mut validated_objects = Vec::with_capacity(req.objects.len());
        for obj in &req.objects {
            let validated = match obj.validate() {
                Ok(v) => v,
                Err(err) => {
                    let _ = tx.rollback().await;
                    return Envelope::err(err);
                }
            };
            validated_objects.push(validated);
        }

        let mut idempotency_scope: Option<String> = None;
        let mut idempotency_key: Option<String> = None;
        let mut request_hash: Option<String> = None;

        if let Some(raw_key) = req.idempotency_key.as_deref() {
            let key = raw_key.trim();
            if key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_LEN {
                let _ = tx.rollback().await;
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "invalid idempotency_key length",
                    Some(serde_json::json!({
                        "reason_code": "OB_IDEMPOTENCY_KEY_INVALID",
                        "max_len": MAX_IDEMPOTENCY_KEY_LEN
                    })),
                ));
            }
            if validated_objects.is_empty() {
                let _ = tx.rollback().await;
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "idempotency_key requires at least one object",
                    Some(serde_json::json!({"reason_code": "OB_IDEMPOTENCY_KEY_EMPTY_BATCH"})),
                ));
            }
            let scope = validated_objects[0].scope.clone();
            if validated_objects.iter().any(|o| o.scope != scope) {
                let _ = tx.rollback().await;
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "idempotency_key requires a single scope per request",
                    Some(serde_json::json!({"reason_code": "OB_IDEMPOTENCY_SCOPE_MISMATCH"})),
                ));
            }

            let computed_request_hash =
                compute_request_hash(&scope, req.actor.as_deref(), &validated_objects);

            let existing: Option<IdempotencyRow> = match sqlx::query_as(
                r#"SELECT request_hash, receipt_hash, accepted_count, object_ids, results_json
                   FROM ob_idempotency
                   WHERE scope = $1 AND idempotency_key = $2
                   FOR UPDATE"#,
            )
            .bind(&scope)
            .bind(key)
            .fetch_optional(&mut *tx)
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.rollback().await;
                    return Envelope::err(ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("idempotency lookup failed: {e}"),
                        None,
                    ));
                }
            };

            if let Some(row) = existing {
                if row.request_hash != computed_request_hash {
                    let _ = tx.rollback().await;
                    return Envelope::err(ErrorEnvelope::new(
                        ErrorCode::ObInvalidRequest,
                        "idempotency_key already used with different payload",
                        Some(serde_json::json!({
                            "reason_code": "OB_IDEMPOTENCY_KEY_REUSE_MISMATCH",
                            "idempotency_key": key
                        })),
                    ));
                }
                if let Err(e) = tx.commit().await {
                    return Envelope::err(ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("commit failed: {e}"),
                        None,
                    ));
                }
                let mut object_ids = row.object_ids;
                if object_ids.len() > RECEIPT_OBJECT_IDS_CAP {
                    object_ids.truncate(RECEIPT_OBJECT_IDS_CAP);
                }
                return Envelope::ok(PutObjectsResponse {
                    results: row.results_json.0,
                    replayed: true,
                    request_id: Some(row.request_hash),
                    accepted_count: row.accepted_count.max(0) as usize,
                    object_ids,
                    receipt_hash: Some(row.receipt_hash),
                });
            }

            idempotency_scope = Some(scope);
            idempotency_key = Some(key.to_string());
            request_hash = Some(computed_request_hash);
        }

        let mut results = Vec::with_capacity(validated_objects.len());
        for validated in &validated_objects {
            let actor = validated
                .provenance
                .get("actor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| req.actor.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let expires_at =
                match Self::parse_datetime_opt("expires_at", validated.expires_at.as_deref()) {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = tx.rollback().await;
                        return Envelope::err(e);
                    }
                };
            let expires_at = expires_at.or_else(|| {
                default_expiry_for_state(validated.lifecycle_state, chrono::Utc::now())
            });
            let value_hash = validated
                .memory_key
                .as_ref()
                .map(|_| value_hash_v01(&validated.data));

            let version =
                match Self::upsert_object(&mut tx, validated, expires_at, value_hash).await {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = tx.rollback().await;
                        return Envelope::err(e);
                    }
                };

            let payload = serde_json::json!({
                "ref": validated.id,
                "type": validated.object_type,
                "status": validated.status,
                "version": version,
            });

            if let Err(e) = Self::insert_event(
                &mut tx,
                &validated.scope,
                "object_written",
                &actor,
                &payload,
            )
            .await
            {
                let _ = tx.rollback().await;
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("failed to append event: {e}"),
                    None,
                ));
            }

            results.push(PutResult {
                r#ref: validated.id.clone(),
                object_type: validated.object_type.clone(),
                status: validated.status.clone(),
                version,
            });
        }

        let mut response = make_put_response(results, false, request_hash.clone(), None);
        let receipt_hash = compute_receipt_hash(
            request_hash.as_deref().unwrap_or(""),
            response.accepted_count,
            &response.object_ids,
            &response.results,
        );
        response.receipt_hash = Some(receipt_hash.clone());

        if let (Some(scope), Some(key), Some(req_hash)) =
            (idempotency_scope, idempotency_key, request_hash)
        {
            let accepted_count = response.accepted_count as i64;
            let object_ids = response.object_ids.clone();
            let results_json = sqlx::types::Json(response.results.clone());
            if let Err(e) = sqlx::query(
                r#"INSERT INTO ob_idempotency
                   (scope, idempotency_key, request_hash, receipt_hash, accepted_count, object_ids, results_json)
                   VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            )
            .bind(scope)
            .bind(key)
            .bind(req_hash)
            .bind(receipt_hash)
            .bind(accepted_count)
            .bind(object_ids)
            .bind(results_json)
            .execute(&mut *tx)
            .await
            {
                let _ = tx.rollback().await;
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("idempotency ledger insert failed: {e}"),
                    None,
                ));
            }
        }

        if let Err(e) = tx.commit().await {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("commit failed: {e}"),
                None,
            ));
        }

        Envelope::ok(response)
    }

    async fn get_objects(&self, req: GetObjectsRequest) -> Envelope<GetObjectsResponse> {
        if req.refs.is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "refs must not be empty",
                None,
            ));
        }
        if req.scope.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }

        let include_states = Self::states_or_default(req.include_states.clone());
        if include_states.is_empty() {
            return Envelope::ok(GetObjectsResponse { objects: vec![] });
        }
        let include_expired = req.include_expired.unwrap_or(false);
        let now = match Self::parse_datetime_opt("now", req.now.as_deref()) {
            Ok(Some(v)) => v,
            Ok(None) => chrono::Utc::now(),
            Err(e) => return Envelope::err(e),
        };
        let now = if include_expired {
            chrono::DateTime::<chrono::Utc>::MIN_UTC
        } else {
            now
        };
        let state_strings = Self::to_state_strings(&include_states);

        #[derive(sqlx::FromRow)]
        struct ObRow {
            id: String,
            scope: String,
            r#type: String,
            status: String,
            spec_version: String,
            tags: Vec<String>,
            data: sqlx::types::Json<Value>,
            provenance: sqlx::types::Json<Value>,
            version: i64,
            created_at: chrono::DateTime<chrono::Utc>,
            updated_at: chrono::DateTime<chrono::Utc>,
            lifecycle_state: String,
            expires_at: Option<chrono::DateTime<chrono::Utc>>,
            memory_key: Option<String>,
            conflict_status: String,
            resolved_by_object_id: Option<String>,
            resolved_at: Option<chrono::DateTime<chrono::Utc>>,
            resolution_note: Option<String>,
        }

        let rows: Vec<ObRow> = match sqlx::query_as(
            r#"SELECT id,
                      scope,
                      type,
                      status,
                      spec_version,
                      tags,
                      data,
                      provenance,
                      version,
                      created_at,
                      updated_at,
                      lifecycle_state,
                      expires_at,
                      memory_key,
                      conflict_status,
                      resolved_by_object_id,
                      resolved_at,
                      resolution_note
               FROM ob_objects
               WHERE id = ANY($1)
                 AND scope = $2
                 AND lifecycle_state = ANY($3)
                 AND (expires_at IS NULL OR expires_at > $4)"#,
        )
        .bind(&req.refs)
        .bind(&req.scope)
        .bind(state_strings)
        .bind(now)
        .fetch_all(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("read failed: {e}"),
                    None,
                ))
            }
        };

        let mut by_id = std::collections::HashMap::with_capacity(rows.len());
        for r in rows {
            by_id.insert(
                r.id.clone(),
                MemoryObjectStored {
                    object_type: r.r#type,
                    id: r.id,
                    scope: r.scope,
                    status: r.status,
                    spec_version: r.spec_version,
                    tags: r.tags,
                    data: r.data.0,
                    provenance: r.provenance.0,
                    version: r.version,
                    created_at: r.created_at.to_rfc3339(),
                    updated_at: r.updated_at.to_rfc3339(),
                    lifecycle_state: match lifecycle_from_row(&r.lifecycle_state) {
                        Ok(v) => v,
                        Err(e) => return Envelope::err(e),
                    },
                    expires_at: r.expires_at.map(|dt| dt.to_rfc3339()),
                    memory_key: r.memory_key,
                    conflict_status: match conflict_status_from_row(&r.conflict_status) {
                        Ok(v) => v,
                        Err(e) => return Envelope::err(e),
                    },
                    resolved_by_object_id: r.resolved_by_object_id,
                    resolved_at: r.resolved_at.map(|dt| dt.to_rfc3339()),
                    resolution_note: r.resolution_note,
                },
            );
        }

        let mut missing = Vec::new();
        let mut ordered = Vec::with_capacity(req.refs.len());
        for r in &req.refs {
            if let Some(obj) = by_id.remove(r) {
                ordered.push(obj);
            } else {
                missing.push(r.clone());
            }
        }

        if !missing.is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObNotFound,
                "one or more refs not found",
                Some(serde_json::json!({ "missing_refs": missing })),
            ));
        }

        Envelope::ok(GetObjectsResponse { objects: ordered })
    }

    async fn search_structured(
        &self,
        req: SearchStructuredRequest,
    ) -> Envelope<SearchStructuredResponse> {
        if req.scope.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }

        let include_states = Self::states_or_default(req.include_states.clone());
        if include_states.is_empty() {
            return Envelope::ok(SearchStructuredResponse { results: vec![] });
        }
        let include_expired = req.include_expired.unwrap_or(false);
        let now = match Self::parse_datetime_opt("now", req.now.as_deref()) {
            Ok(Some(v)) => v,
            Ok(None) => chrono::Utc::now(),
            Err(e) => return Envelope::err(e),
        };
        let now = if include_expired {
            chrono::DateTime::<chrono::Utc>::MIN_UTC
        } else {
            now
        };
        let state_strings = Self::to_state_strings(&include_states);
        let include_conflicts = include_conflicts_enabled(req.include_conflicts);

        let limit = req.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT) as i64;
        let offset = req.offset.unwrap_or(0) as i64;

        let order_by = match order_by_sql(&req.order_by) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        let expr = match req.where_expr.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(s) => match parse_where(s) {
                Ok(e) => Some(e),
                Err(err) => return Envelope::err(err),
            },
        };

        let mut qb = QueryBuilder::new(
            "SELECT id, type, status, updated_at, version, memory_key, conflict_status, resolved_by_object_id, resolved_at FROM ob_objects WHERE scope = ",
        );
        qb.push_bind(&req.scope);
        qb.push(" AND lifecycle_state = ANY(");
        qb.push_bind(state_strings.clone());
        qb.push(")");
        qb.push(" AND (expires_at IS NULL OR expires_at > ");
        qb.push_bind(now);
        qb.push(")");

        if let Some(e) = &expr {
            qb.push(" AND ");
            if let Err(err) = build_expr(&mut qb, e) {
                return Envelope::err(err);
            }
        }

        qb.push(" ORDER BY ");
        qb.push(order_by);
        qb.push(" LIMIT ");
        qb.push_bind(limit);
        qb.push(" OFFSET ");
        qb.push_bind(offset);

        #[derive(sqlx::FromRow)]
        struct SearchRow {
            id: String,
            r#type: String,
            status: String,
            updated_at: chrono::DateTime<chrono::Utc>,
            version: i64,
            memory_key: Option<String>,
            conflict_status: String,
            resolved_by_object_id: Option<String>,
            resolved_at: Option<chrono::DateTime<chrono::Utc>>,
        }

        let rows: Vec<SearchRow> = match qb.build_query_as().fetch_all(&self.pool).await {
            Ok(r) => r,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("search failed: {e}"),
                    None,
                ))
            }
        };

        let conflict_map = if include_conflicts {
            let memory_keys: Vec<String> =
                rows.iter().filter_map(|r| r.memory_key.clone()).collect();
            match self
                .load_conflicts(&req.scope, &memory_keys, &state_strings, now)
                .await
            {
                Ok(v) => v,
                Err(e) => return Envelope::err(e),
            }
        } else {
            std::collections::HashMap::new()
        };

        let results = rows
            .into_iter()
            .map(|r| {
                let conflict_status = match conflict_status_from_row(&r.conflict_status) {
                    Ok(v) => v,
                    Err(e) => return Err(e),
                };

                let (conflict, conflict_count, conflicting_object_ids) =
                    match r.memory_key.as_ref().and_then(|k| conflict_map.get(k)) {
                        Some(info) if info.distinct_count > 1 => {
                            let mut ids: Vec<String> =
                                info.ids.iter().filter(|id| *id != &r.id).cloned().collect();
                            if ids.len() > 10 {
                                ids.truncate(10);
                            }
                            (true, Some(info.total_count as u32), Some(ids))
                        }
                        _ => (false, None, None),
                    };

                Ok(SearchItem {
                    r#ref: r.id,
                    object_type: r.r#type,
                    status: r.status,
                    updated_at: r.updated_at.to_rfc3339(),
                    version: r.version,
                    conflict,
                    conflict_status,
                    conflict_count,
                    conflicting_object_ids,
                    resolved_by_object_id: r.resolved_by_object_id,
                    resolved_at: r.resolved_at.map(|dt| dt.to_rfc3339()),
                })
            })
            .collect::<Result<Vec<_>, ErrorEnvelope>>();

        let results = match results {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        Envelope::ok(SearchStructuredResponse { results })
    }
    async fn embed_generate(&self, req: EmbedGenerateRequest) -> Envelope<EmbedGenerateResponse> {
        if req.scope.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }
        if req.model.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "model is required",
                None,
            ));
        }

        let dims = req.dims.unwrap_or(EMBEDDING_DIMS);
        if dims != EMBEDDING_DIMS {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "dims must be 1536 for v0.1",
                Some(serde_json::json!({"expected": EMBEDDING_DIMS, "got": dims})),
            ));
        }

        let (normalized_text, checksum, object_id) = match &req.target {
            EmbedTarget::Text { text } => {
                let normalized = match validate_embedding_text(text, MAX_EMBED_TEXT_LEN) {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
                let checksum = checksum_v01(&normalized);
                (normalized, checksum, None)
            }
            EmbedTarget::Ref { r#ref } => {
                #[derive(sqlx::FromRow)]
                struct ObjRow {
                    id: String,
                    r#type: String,
                    data: sqlx::types::Json<Value>,
                }

                let row: Option<ObjRow> = match sqlx::query_as(
                    r#"SELECT id, type, data
                       FROM ob_objects
                       WHERE id = $1 AND scope = $2
                       LIMIT 1"#,
                )
                .bind(r#ref)
                .bind(&req.scope)
                .fetch_optional(&self.pool)
                .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        return Envelope::err(ErrorEnvelope::new(
                            ErrorCode::ObStorageError,
                            format!("read failed: {e}"),
                            None,
                        ));
                    }
                };

                let Some(row) = row else {
                    return Envelope::err(ErrorEnvelope::new(
                        ErrorCode::ObNotFound,
                        "ref not found",
                        Some(serde_json::json!({"ref": r#ref})),
                    ));
                };

                let normalized = match normalize_object_text(&row.r#type, &row.data.0) {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };

                let normalized = match validate_embedding_text(&normalized, MAX_EMBED_TEXT_LEN) {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };

                let checksum = checksum_v01(&normalized);
                (normalized, checksum, Some(row.id))
            }
        };

        #[derive(sqlx::FromRow)]
        struct ExistingRow {
            id: String,
            object_id: Option<String>,
        }

        let provider = self.embedding_provider.as_str();
        let kind = self.embedding_kind.as_str();

        let existing: Option<ExistingRow> = match sqlx::query_as(
            r#"SELECT id, object_id
               FROM ob_embeddings
               WHERE scope = $1
                 AND provider = $2
                 AND model = $3
                 AND kind = $4
                 AND checksum = $5
               LIMIT 1"#,
        )
        .bind(&req.scope)
        .bind(provider)
        .bind(&req.model)
        .bind(kind)
        .bind(&checksum)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(r) => r,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("dedupe lookup failed: {e}"),
                    None,
                ));
            }
        };

        if let Some(row) = existing {
            return Envelope::ok(EmbedGenerateResponse {
                embedding_id: row.id,
                object_id: row.object_id.or(object_id),
                model: req.model,
                dims,
                checksum,
                reused: true,
            });
        }

        let embedding = match self.embedder.embed(&req.model, &normalized_text).await {
            Ok(v) => v,
            Err(e) => {
                let (code, message, details) = match e {
                    EmbedError::ProviderUnavailable => (
                        ErrorCode::ObEmbeddingFailed,
                        "embedding provider unavailable".to_string(),
                        Some(serde_json::json!({"reason": "provider_unavailable"})),
                    ),
                    EmbedError::InvalidInput(m) => (ErrorCode::ObEmbeddingFailed, m, None),
                    EmbedError::InvalidRequest {
                        message, details, ..
                    } => (ErrorCode::ObInvalidRequest, message, details),
                    EmbedError::ProviderError {
                        message, details, ..
                    } => (ErrorCode::ObEmbeddingFailed, message, details),
                };
                return Envelope::err(ErrorEnvelope::new(code, message, details));
            }
        };

        let got_dims = embedding.len() as i32;
        if got_dims != EMBEDDING_DIMS {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObEmbeddingFailed,
                "provider returned wrong embedding dims",
                Some(serde_json::json!({"expected": EMBEDDING_DIMS, "got": got_dims})),
            ));
        }

        let embedding_id = Uuid::new_v4().to_string();
        let vector_text = match embedding_to_pgvector_literal(&embedding) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        // If an embedding already exists for this object/provider/model/kind, update it (latest wins).
        let inserted: Result<(String,), sqlx::Error> = sqlx::query_as(
            r#"INSERT INTO ob_embeddings
               (id, object_id, scope, provider, model, kind, dims, checksum, embedding)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, ($9)::vector)
               ON CONFLICT (scope, object_id, provider, model, kind)
               WHERE object_id IS NOT NULL
               DO UPDATE SET
                 checksum = EXCLUDED.checksum,
                 dims = EXCLUDED.dims,
                 embedding = EXCLUDED.embedding,
                 created_at = now()
               RETURNING id"#,
        )
        .bind(&embedding_id)
        .bind(object_id.as_deref())
        .bind(&req.scope)
        .bind(provider)
        .bind(&req.model)
        .bind(kind)
        .bind(dims)
        .bind(&checksum)
        .bind(&vector_text)
        .fetch_one(&self.pool)
        .await;

        let embedding_id = match inserted {
            Ok((id,)) => id,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("failed to insert embedding: {e}"),
                    None,
                ));
            }
        };

        Envelope::ok(EmbedGenerateResponse {
            embedding_id,
            object_id,
            model: req.model,
            dims,
            checksum,
            reused: false,
        })
    }
    async fn search_semantic(
        &self,
        req: SearchSemanticRequest,
    ) -> Envelope<SearchSemanticResponse> {
        if req.scope.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObScopeRequired,
                "scope is required",
                None,
            ));
        }
        if req.query.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "query is required",
                None,
            ));
        }

        let include_states = Self::states_or_default(req.include_states.clone());
        if include_states.is_empty() {
            return Envelope::ok(SearchSemanticResponse { matches: vec![] });
        }
        let include_expired = req.include_expired.unwrap_or(false);
        let now = match Self::parse_datetime_opt("now", req.now.as_deref()) {
            Ok(Some(v)) => v,
            Ok(None) => chrono::Utc::now(),
            Err(e) => return Envelope::err(e),
        };
        let now = if include_expired {
            chrono::DateTime::<chrono::Utc>::MIN_UTC
        } else {
            now
        };
        let state_strings = Self::to_state_strings(&include_states);
        let include_conflicts = include_conflicts_enabled(req.include_conflicts);

        let top_k = req.top_k.unwrap_or(10).min(50) as i64;

        let provider = Self::normalize_opt_field(req.embedding_provider.as_deref())
            .unwrap_or_else(|| self.embedding_provider.clone());
        let model = Self::normalize_opt_field(req.embedding_model.as_deref())
            .or_else(|| Self::normalize_opt_field(req.model.as_deref()))
            .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string());
        let kind = Self::normalize_opt_field(req.embedding_kind.as_deref())
            .unwrap_or_else(|| self.embedding_kind.clone());

        let has_embeddings: Option<i64> = match sqlx::query_scalar(
            r#"SELECT 1
               FROM ob_embeddings
               WHERE scope = $1
                 AND provider = $2
                 AND model = $3
                 AND kind = $4
               LIMIT 1"#,
        )
        .bind(&req.scope)
        .bind(&provider)
        .bind(&model)
        .bind(&kind)
        .fetch_optional(&self.pool)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("semantic search failed: {e}"),
                    None,
                ));
            }
        };

        if has_embeddings.is_none() {
            return Envelope::ok(SearchSemanticResponse { matches: vec![] });
        }

        let normalized_query = match validate_embedding_text(&req.query, MAX_EMBED_TEXT_LEN) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        let embedding = match self.embedder.embed(&model, &normalized_query).await {
            Ok(v) => v,
            Err(e) => {
                let (code, message, details) = match e {
                    EmbedError::ProviderUnavailable => (
                        ErrorCode::ObEmbeddingFailed,
                        "embedding provider unavailable".to_string(),
                        Some(serde_json::json!({"reason": "provider_unavailable"})),
                    ),
                    EmbedError::InvalidInput(m) => (ErrorCode::ObEmbeddingFailed, m, None),
                    EmbedError::InvalidRequest {
                        message, details, ..
                    } => (ErrorCode::ObInvalidRequest, message, details),
                    EmbedError::ProviderError {
                        message, details, ..
                    } => (ErrorCode::ObEmbeddingFailed, message, details),
                };
                return Envelope::err(ErrorEnvelope::new(code, message, details));
            }
        };

        let got_dims = embedding.len() as i32;
        if got_dims != EMBEDDING_DIMS {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObEmbeddingFailed,
                "provider returned wrong embedding dims",
                Some(serde_json::json!({"expected": EMBEDDING_DIMS, "got": got_dims})),
            ));
        }

        let vector_text = match embedding_to_pgvector_literal(&embedding) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        let expr = match req.filters.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(s) => match parse_where(s) {
                Ok(e) => Some(e),
                Err(err) => return Envelope::err(err),
            },
        };

        let mut qb = QueryBuilder::<Postgres>::new(
            "SELECT o.id as ref, o.type as kind, (1.0 - (e.embedding <=> (",
        );
        qb.push_bind(&vector_text);
        qb.push(")::vector))::float4 as score, o.updated_at::text as updated_at, o.memory_key as memory_key, o.conflict_status as conflict_status, o.resolved_by_object_id as resolved_by_object_id, o.resolved_at::text as resolved_at ");
        qb.push("FROM ob_embeddings e ");
        qb.push("JOIN ob_objects o ON o.id = e.object_id ");
        qb.push("WHERE e.object_id IS NOT NULL ");
        qb.push("AND e.scope = ");
        qb.push_bind(&req.scope);
        qb.push(" AND o.scope = ");
        qb.push_bind(&req.scope);
        qb.push(" AND o.lifecycle_state = ANY(");
        qb.push_bind(state_strings.clone());
        qb.push(")");
        qb.push(" AND (o.expires_at IS NULL OR o.expires_at > ");
        qb.push_bind(now);
        qb.push(")");
        qb.push(" AND e.provider = ");
        qb.push_bind(&provider);
        qb.push(" AND e.model = ");
        qb.push_bind(&model);
        qb.push(" AND e.kind = ");
        qb.push_bind(&kind);
        qb.push(" AND e.dims = ");
        qb.push_bind(EMBEDDING_DIMS);

        if let Some(types) = &req.types {
            if types.is_empty() {
                return Envelope::ok(SearchSemanticResponse { matches: vec![] });
            }
            qb.push(" AND o.type = ANY(");
            qb.push_bind(types.clone());
            qb.push(")");
        }

        if let Some(status) = &req.status {
            if status.is_empty() {
                return Envelope::ok(SearchSemanticResponse { matches: vec![] });
            }
            qb.push(" AND o.status = ANY(");
            qb.push_bind(status.clone());
            qb.push(")");
        }

        if let Some(e) = &expr {
            qb.push(" AND ");
            if let Err(err) = build_expr_with_alias(&mut qb, e, "o") {
                return Envelope::err(err);
            }
        }

        qb.push(" ORDER BY e.embedding <=> (");
        qb.push_bind(&vector_text);
        qb.push(")::vector ASC");
        qb.push(" LIMIT ");
        qb.push_bind(top_k);

        #[derive(sqlx::FromRow)]
        struct RowOut {
            r#ref: String,
            kind: String,
            score: f32,
            updated_at: String,
            memory_key: Option<String>,
            conflict_status: String,
            resolved_by_object_id: Option<String>,
            resolved_at: Option<String>,
        }

        let rows: Vec<RowOut> = match qb.build_query_as().fetch_all(&self.pool).await {
            Ok(r) => r,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("semantic search failed: {e}"),
                    None,
                ));
            }
        };

        let conflict_map = if include_conflicts {
            let memory_keys: Vec<String> =
                rows.iter().filter_map(|r| r.memory_key.clone()).collect();
            match self
                .load_conflicts(&req.scope, &memory_keys, &state_strings, now)
                .await
            {
                Ok(v) => v,
                Err(e) => return Envelope::err(e),
            }
        } else {
            std::collections::HashMap::new()
        };

        let matches = rows
            .into_iter()
            .map(|r| {
                let conflict_status = match conflict_status_from_row(&r.conflict_status) {
                    Ok(v) => v,
                    Err(e) => return Err(e),
                };

                let (conflict, conflict_count, conflicting_object_ids) =
                    match r.memory_key.as_ref().and_then(|k| conflict_map.get(k)) {
                        Some(info) if info.distinct_count > 1 => {
                            let mut ids: Vec<String> = info
                                .ids
                                .iter()
                                .filter(|id| *id != &r.r#ref)
                                .cloned()
                                .collect();
                            if ids.len() > 10 {
                                ids.truncate(10);
                            }
                            (true, Some(info.total_count as u32), Some(ids))
                        }
                        _ => (false, None, None),
                    };

                Ok(SearchMatch {
                    r#ref: r.r#ref,
                    kind: r.kind,
                    score: r.score,
                    updated_at: r.updated_at,
                    snippet: None,
                    conflict,
                    conflict_status,
                    conflict_count,
                    conflicting_object_ids,
                    resolved_by_object_id: r.resolved_by_object_id,
                    resolved_at: r.resolved_at,
                })
            })
            .collect::<Result<Vec<_>, ErrorEnvelope>>();

        let matches = match matches {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };

        Envelope::ok(SearchSemanticResponse { matches })
    }

    async fn append_event(
        &self,
        scope: &str,
        event_type: &str,
        actor: &str,
        payload_json: Value,
    ) -> () {
        let _ = sqlx::query(
            r#"INSERT INTO ob_events (scope, event_type, actor, payload)
               VALUES ($1, $2, $3, $4)"#,
        )
        .bind(scope)
        .bind(event_type)
        .bind(actor)
        .bind(sqlx::types::Json(&payload_json))
        .execute(&self.pool)
        .await;
    }

    async fn audit_object_timeline(
        &self,
        req: AuditObjectTimelineRequest,
    ) -> Envelope<AuditResponse> {
        if req.query.scope.trim().is_empty() || req.object_id.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "scope and object_id are required",
                None,
            ));
        }
        let from = match Self::parse_audit_ts("from", req.query.from.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let to = match Self::parse_audit_ts("to", req.query.to.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let (limit, offset) = parse_audit_limit_offset(req.query.limit, req.query.offset);

        let rows = match sqlx::query(
            r#"SELECT e.id, e.event_type, e.actor, e.payload, e.ts, o.memory_key
               FROM ob_events e
               LEFT JOIN ob_objects o
                 ON o.scope = e.scope
                AND o.id = (e.payload ->> 'ref')
               WHERE e.scope = $1
                 AND (e.payload ->> 'ref') = $2
                 AND ($3::timestamptz IS NULL OR e.ts >= $3)
                 AND ($4::timestamptz IS NULL OR e.ts <= $4)
               ORDER BY e.ts DESC, e.id DESC
               LIMIT $5 OFFSET $6"#,
        )
        .bind(&req.query.scope)
        .bind(req.object_id.trim())
        .bind(from)
        .bind(to)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("audit object timeline failed: {e}"),
                    None,
                ));
            }
        };

        let events = rows
            .into_iter()
            .map(|r| {
                Self::audit_event_from_row(
                    r.get("id"),
                    r.get("event_type"),
                    r.get("actor"),
                    r.get::<sqlx::types::Json<Value>, _>("payload").0,
                    r.get("ts"),
                    r.get("memory_key"),
                )
            })
            .collect();
        Envelope::ok(AuditResponse {
            events,
            limit,
            offset,
        })
    }

    async fn audit_memory_key_timeline(
        &self,
        req: AuditMemoryKeyTimelineRequest,
    ) -> Envelope<AuditResponse> {
        if req.query.scope.trim().is_empty() || req.memory_key.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "scope and memory_key are required",
                None,
            ));
        }
        let from = match Self::parse_audit_ts("from", req.query.from.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let to = match Self::parse_audit_ts("to", req.query.to.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let (limit, offset) = parse_audit_limit_offset(req.query.limit, req.query.offset);

        let rows = match sqlx::query(
            r#"SELECT e.id, e.event_type, e.actor, e.payload, e.ts, o.memory_key
               FROM ob_events e
               JOIN ob_objects o
                 ON o.scope = e.scope
                AND o.id = (e.payload ->> 'ref')
               WHERE e.scope = $1
                 AND o.memory_key = $2
                 AND ($3::timestamptz IS NULL OR e.ts >= $3)
                 AND ($4::timestamptz IS NULL OR e.ts <= $4)
               ORDER BY e.ts DESC, e.id DESC
               LIMIT $5 OFFSET $6"#,
        )
        .bind(&req.query.scope)
        .bind(req.memory_key.trim())
        .bind(from)
        .bind(to)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("audit memory_key timeline failed: {e}"),
                    None,
                ));
            }
        };

        let events = rows
            .into_iter()
            .map(|r| {
                Self::audit_event_from_row(
                    r.get("id"),
                    r.get("event_type"),
                    r.get("actor"),
                    r.get::<sqlx::types::Json<Value>, _>("payload").0,
                    r.get("ts"),
                    r.get("memory_key"),
                )
            })
            .collect();
        Envelope::ok(AuditResponse {
            events,
            limit,
            offset,
        })
    }

    async fn audit_actor_activity(
        &self,
        req: AuditActorActivityRequest,
    ) -> Envelope<AuditResponse> {
        if req.query.scope.trim().is_empty() || req.actor_identity_id.trim().is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "scope and actor_identity_id are required",
                None,
            ));
        }
        let from = match Self::parse_audit_ts("from", req.query.from.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let to = match Self::parse_audit_ts("to", req.query.to.as_deref()) {
            Ok(v) => v,
            Err(e) => return Envelope::err(e),
        };
        let (limit, offset) = parse_audit_limit_offset(req.query.limit, req.query.offset);

        let rows = match sqlx::query(
            r#"SELECT e.id, e.event_type, e.actor, e.payload, e.ts, o.memory_key
               FROM ob_events e
               LEFT JOIN ob_objects o
                 ON o.scope = e.scope
                AND o.id = (e.payload ->> 'ref')
               WHERE e.scope = $1
                 AND e.actor = $2
                 AND ($3::timestamptz IS NULL OR e.ts >= $3)
                 AND ($4::timestamptz IS NULL OR e.ts <= $4)
               ORDER BY e.ts DESC, e.id DESC
               LIMIT $5 OFFSET $6"#,
        )
        .bind(&req.query.scope)
        .bind(req.actor_identity_id.trim())
        .bind(from)
        .bind(to)
        .bind(limit as i64)
        .bind(offset as i64)
        .fetch_all(&self.pool)
        .await
        {
            Ok(v) => v,
            Err(e) => {
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("audit actor activity failed: {e}"),
                    None,
                ));
            }
        };

        let events = rows
            .into_iter()
            .map(|r| {
                Self::audit_event_from_row(
                    r.get("id"),
                    r.get("event_type"),
                    r.get("actor"),
                    r.get::<sqlx::types::Json<Value>, _>("payload").0,
                    r.get("ts"),
                    r.get("memory_key"),
                )
            })
            .collect();
        Envelope::ok(AuditResponse {
            events,
            limit,
            offset,
        })
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let digest = hasher.finalize();
    hex_encode(&digest)
}

fn role_from_row(value: &str) -> Result<WorkspaceRole, ErrorEnvelope> {
    use std::str::FromStr;
    WorkspaceRole::from_str(value).map_err(|_| {
        ErrorEnvelope::new(
            ErrorCode::ObInternal,
            "invalid role stored in database",
            Some(serde_json::json!({"role": value})),
        )
    })
}

fn lifecycle_from_row(value: &str) -> Result<LifecycleState, ErrorEnvelope> {
    use std::str::FromStr;
    LifecycleState::from_str(value).map_err(|_| {
        ErrorEnvelope::new(
            ErrorCode::ObInternal,
            "invalid lifecycle state stored in database",
            Some(serde_json::json!({"lifecycle_state": value})),
        )
    })
}

fn conflict_status_from_row(value: &str) -> Result<ConflictStatus, ErrorEnvelope> {
    use std::str::FromStr;
    ConflictStatus::from_str(value).map_err(|_| {
        ErrorEnvelope::new(
            ErrorCode::ObInternal,
            "invalid conflict status stored in database",
            Some(serde_json::json!({"conflict_status": value})),
        )
    })
}

#[async_trait]
impl AuthStore for PgStore {
    async fn auth_from_token(&self, token: &str) -> Result<AuthContext, ErrorEnvelope> {
        if token.trim().is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObUnauthenticated,
                "missing auth token",
                None,
            ));
        }

        let token_hash = hash_token(token);

        #[derive(sqlx::FromRow)]
        struct TokenRow {
            identity_id: String,
            workspace_id: String,
            role: String,
        }

        let row: Option<TokenRow> = sqlx::query_as(
            r#"SELECT identity_id, workspace_id, role
               FROM ob_tokens
               WHERE token_hash = $1
                 AND revoked_at IS NULL
               LIMIT 1"#,
        )
        .bind(&token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("auth lookup failed: {e}"),
                None,
            )
        })?;

        let Some(row) = row else {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObUnauthenticated,
                "invalid auth token",
                None,
            ));
        };

        let role = role_from_row(&row.role)?;

        Ok(AuthContext {
            identity_id: row.identity_id,
            workspace_id: row.workspace_id,
            role,
        })
    }

    async fn create_token(
        &self,
        req: TokenCreateRequest,
    ) -> Result<TokenCreateResponse, ErrorEnvelope> {
        let workspace_id = req.workspace_id.trim().to_string();
        if workspace_id.is_empty() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "workspace_id is required",
                None,
            ));
        }

        let exists: Option<String> =
            sqlx::query_scalar(r#"SELECT id FROM ob_workspaces WHERE id = $1 LIMIT 1"#)
                .bind(&workspace_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| {
                    ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("workspace lookup failed: {e}"),
                        None,
                    )
                })?;

        if exists.is_none() {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "workspace not found",
                Some(serde_json::json!({"workspace_id": workspace_id})),
            ));
        }

        let identity_id = Uuid::new_v4().to_string();
        let display_name = req
            .display_name
            .clone()
            .or_else(|| req.label.clone())
            .unwrap_or_else(|| "api-token".to_string());

        sqlx::query(
            r#"INSERT INTO ob_identities (id, display_name)
               VALUES ($1, $2)"#,
        )
        .bind(&identity_id)
        .bind(&display_name)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("identity insert failed: {e}"),
                None,
            )
        })?;

        let token = format!("ob_{}", Uuid::new_v4());
        let token_hash = hash_token(&token);

        #[derive(sqlx::FromRow)]
        struct TokenOut {
            created_at: chrono::DateTime<chrono::Utc>,
        }

        let created: TokenOut = sqlx::query_as(
            r#"INSERT INTO ob_tokens (token_hash, identity_id, workspace_id, role, label)
               VALUES ($1, $2, $3, $4, $5)
               RETURNING created_at"#,
        )
        .bind(&token_hash)
        .bind(&identity_id)
        .bind(&workspace_id)
        .bind(req.role.as_str())
        .bind(req.label)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("token insert failed: {e}"),
                None,
            )
        })?;

        Ok(TokenCreateResponse {
            token,
            workspace_id,
            role: req.role,
            identity_id,
            created_at: created.created_at.to_rfc3339(),
        })
    }

    async fn bootstrap_default_workspace(&self) -> Result<Option<BootstrapToken>, ErrorEnvelope> {
        let mut tx = self.pool.begin().await.map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("bootstrap transaction failed: {e}"),
                None,
            )
        })?;

        let has_tokens: Option<i64> = sqlx::query_scalar("SELECT 1 FROM ob_tokens LIMIT 1")
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| {
                ErrorEnvelope::new(
                    ErrorCode::ObStorageError,
                    format!("bootstrap check failed: {e}"),
                    None,
                )
            })?;

        if has_tokens.is_some() {
            tx.commit().await.ok();
            return Ok(None);
        }

        sqlx::query(
            r#"INSERT INTO ob_workspaces (id, name)
               VALUES ($1, $2)
               ON CONFLICT (id) DO NOTHING"#,
        )
        .bind(DEFAULT_WORKSPACE_ID)
        .bind(DEFAULT_WORKSPACE_NAME)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("bootstrap workspace insert failed: {e}"),
                None,
            )
        })?;

        let identity_id = Uuid::new_v4().to_string();
        sqlx::query(
            r#"INSERT INTO ob_identities (id, display_name)
               VALUES ($1, $2)"#,
        )
        .bind(&identity_id)
        .bind("bootstrap-owner")
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("bootstrap identity insert failed: {e}"),
                None,
            )
        })?;

        let token = format!("ob_{}", Uuid::new_v4());
        let token_hash = hash_token(&token);

        sqlx::query(
            r#"INSERT INTO ob_tokens (token_hash, identity_id, workspace_id, role, label)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(&token_hash)
        .bind(&identity_id)
        .bind(DEFAULT_WORKSPACE_ID)
        .bind(WorkspaceRole::Owner.as_str())
        .bind("bootstrap-owner")
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("bootstrap token insert failed: {e}"),
                None,
            )
        })?;

        tx.commit().await.ok();

        Ok(Some(BootstrapToken {
            token,
            workspace_id: DEFAULT_WORKSPACE_ID.to_string(),
            role: WorkspaceRole::Owner,
        }))
    }

    async fn workspace_info(
        &self,
        workspace_id: &str,
        caller_identity_id: &str,
        caller_role: WorkspaceRole,
    ) -> Result<WorkspaceInfoResponse, ErrorEnvelope> {
        #[derive(sqlx::FromRow)]
        struct OwnerRow {
            identity_id: String,
            display_name: Option<String>,
        }

        let row: Option<OwnerRow> = sqlx::query_as(
            r#"SELECT t.identity_id, i.display_name
               FROM ob_tokens t
               LEFT JOIN ob_identities i ON i.id = t.identity_id
               WHERE t.workspace_id = $1
                 AND t.role = 'owner'
                 AND t.revoked_at IS NULL
               ORDER BY t.created_at ASC
               LIMIT 1"#,
        )
        .bind(workspace_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("workspace info lookup failed: {e}"),
                None,
            )
        })?;

        let Some(owner) = row else {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObNotFound,
                "workspace owner not found",
                Some(serde_json::json!({"workspace_id": workspace_id})),
            ));
        };

        Ok(WorkspaceInfoResponse {
            workspace_id: workspace_id.to_string(),
            owner_identity_id: owner.identity_id,
            owner_display_name: owner.display_name,
            caller_identity_id: caller_identity_id.to_string(),
            caller_role,
        })
    }
}
