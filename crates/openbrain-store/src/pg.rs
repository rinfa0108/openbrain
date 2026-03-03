use async_trait::async_trait;
use openbrain_core::{
    Envelope, ErrorCode, ErrorEnvelope, MemoryObjectStored, ValidatedMemoryObject,
};
use serde_json::Value;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Row, Transaction};

use crate::{
    GetObjectsRequest, GetObjectsResponse, PutObjectsRequest, PutObjectsResponse, PutResult, Store,
};

#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
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
                ));
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
                ));
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
