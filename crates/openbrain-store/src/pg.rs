use async_trait::async_trait;
use openbrain_core::query::{parse_where, CmpOp, Expr, FieldPath, Literal, Predicate};
use openbrain_core::textnorm::{checksum_v01, normalize_object_text, validate_embedding_text};
use openbrain_core::{
    Envelope, ErrorCode, ErrorEnvelope, MemoryObjectStored, ValidatedMemoryObject,
};
use openbrain_embed::{EmbedError, EmbeddingProvider, NoopEmbeddingProvider};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, QueryBuilder, Row, Transaction};
use std::sync::Arc;
use uuid::Uuid;

use crate::{
    EmbedGenerateRequest, EmbedGenerateResponse, EmbedTarget, GetObjectsRequest,
    GetObjectsResponse, OrderBySpec, PutObjectsRequest, PutObjectsResponse, PutResult, SearchItem,
    SearchStructuredRequest, SearchStructuredResponse, Store,
};

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

const EMBEDDING_DIMS: i32 = 1536;
const MAX_EMBED_TEXT_LEN: usize = 32 * 1024;

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
        })
    }

    pub async fn connect_with_embedder(
        database_url: &str,
        embedder: Arc<dyn EmbeddingProvider>,
    ) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Ok(Self { pool, embedder })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self {
            pool,
            embedder: Arc::new(NoopEmbeddingProvider),
        }
    }

    pub fn from_pool_with_embedder(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self { pool, embedder }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    async fn upsert_object(
        tx: &mut Transaction<'_, Postgres>,
        obj: &ValidatedMemoryObject,
    ) -> Result<i64, sqlx::Error> {
        let existing = sqlx::query(
            r#"SELECT version
               FROM ob_objects
               WHERE id = $1
               FOR UPDATE"#,
        )
        .bind(&obj.id)
        .fetch_optional(&mut **tx)
        .await?;

        match existing {
            None => {
                let version: i64 = sqlx::query_scalar(
                    r#"INSERT INTO ob_objects
                       (id, scope, type, status, spec_version, tags, data, provenance, version)
                       VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 1)
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
                .fetch_one(&mut **tx)
                .await?;
                Ok(version)
            }
            Some(row) => {
                let current_version: i64 = row.try_get("version")?;
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
                .fetch_one(&mut **tx)
                .await?;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldType {
    Text,
    Timestamp,
    JsonText,
    JsonTimestamp,
    Tags,
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

        let mut results = Vec::with_capacity(req.objects.len());

        for obj in &req.objects {
            let validated = match obj.validate() {
                Ok(v) => v,
                Err(err) => {
                    let _ = tx.rollback().await;
                    return Envelope::err(err);
                }
            };

            let actor = validated
                .provenance
                .get("actor")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| req.actor.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let version = match Self::upsert_object(&mut tx, &validated).await {
                Ok(v) => v,
                Err(e) => {
                    let _ = tx.rollback().await;
                    return Envelope::err(ErrorEnvelope::new(
                        ErrorCode::ObStorageError,
                        format!("write failed: {e}"),
                        None,
                    ));
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
                r#ref: validated.id,
                object_type: validated.object_type,
                status: validated.status,
                version,
            });
        }

        if let Err(e) = tx.commit().await {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("commit failed: {e}"),
                None,
            ));
        }

        Envelope::ok(PutObjectsResponse { results })
    }

    async fn get_objects(&self, req: GetObjectsRequest) -> Envelope<GetObjectsResponse> {
        if req.refs.is_empty() {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "refs must not be empty",
                None,
            ));
        }

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
                      updated_at
               FROM ob_objects
               WHERE id = ANY($1)"#,
        )
        .bind(&req.refs)
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
            "SELECT id, type, status, updated_at, version FROM ob_objects WHERE scope = ",
        );
        qb.push_bind(&req.scope);

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

        let results = rows
            .into_iter()
            .map(|r| SearchItem {
                r#ref: r.id,
                object_type: r.r#type,
                status: r.status,
                updated_at: r.updated_at.to_rfc3339(),
                version: r.version,
            })
            .collect();

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

        let existing: Option<ExistingRow> = match sqlx::query_as(
            r#"SELECT id, object_id
               FROM ob_embeddings
               WHERE scope = $1 AND model = $2 AND checksum = $3
               LIMIT 1"#,
        )
        .bind(&req.scope)
        .bind(&req.model)
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

        let embedding = match self.embedder.embed(&req.model, &normalized_text) {
            Ok(v) => v,
            Err(e) => {
                let (message, details) = match e {
                    EmbedError::ProviderUnavailable => (
                        "embedding provider unavailable".to_string(),
                        Some(serde_json::json!({"reason": "provider_unavailable"})),
                    ),
                    EmbedError::InvalidInput(m) => (m, None),
                    EmbedError::ProviderError(m) => (m, None),
                };
                return Envelope::err(ErrorEnvelope::new(
                    ErrorCode::ObEmbeddingFailed,
                    message,
                    details,
                ));
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

        if let Err(e) = sqlx::query(
            r#"INSERT INTO ob_embeddings
               (id, object_id, scope, model, dims, checksum, embedding)
               VALUES ($1, $2, $3, $4, $5, $6, ($7)::vector)"#,
        )
        .bind(&embedding_id)
        .bind(object_id.as_deref())
        .bind(&req.scope)
        .bind(&req.model)
        .bind(dims)
        .bind(&checksum)
        .bind(&vector_text)
        .execute(&self.pool)
        .await
        {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObStorageError,
                format!("failed to insert embedding: {e}"),
                None,
            ));
        }

        Envelope::ok(EmbedGenerateResponse {
            embedding_id,
            object_id,
            model: req.model,
            dims,
            checksum,
            reused: false,
        })
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
}



