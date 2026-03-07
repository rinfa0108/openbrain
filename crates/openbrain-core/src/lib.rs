use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod query;
pub mod textnorm;

pub const SPEC_VERSION: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    ObInvalidRequest,
    ObInvalidSchema,
    ObUnsupportedVersion,
    ObScopeRequired,
    ObUnauthenticated,
    ObForbidden,
    ObNotFound,
    ObConflict,
    ObStorageError,
    ObEmbeddingFailed,
    ObInternal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObInvalidRequest => "OB_INVALID_REQUEST",
            Self::ObInvalidSchema => "OB_INVALID_SCHEMA",
            Self::ObUnsupportedVersion => "OB_UNSUPPORTED_VERSION",
            Self::ObScopeRequired => "OB_SCOPE_REQUIRED",
            Self::ObUnauthenticated => "OB_UNAUTHENTICATED",
            Self::ObForbidden => "OB_FORBIDDEN",
            Self::ObNotFound => "OB_NOT_FOUND",
            Self::ObConflict => "OB_CONFLICT",
            Self::ObStorageError => "OB_STORAGE_ERROR",
            Self::ObEmbeddingFailed => "OB_EMBEDDING_FAILED",
            Self::ObInternal => "OB_INTERNAL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ErrorEnvelope {
    pub fn new(code: ErrorCode, message: impl Into<String>, details: Option<Value>) -> Self {
        Self {
            code: code.as_str().to_string(),
            message: message.into(),
            details,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Envelope<T> {
    Ok {
        ok: bool,
        #[serde(flatten)]
        data: T,
    },
    Err {
        ok: bool,
        error: ErrorEnvelope,
    },
}

impl<T> Envelope<T> {
    pub fn ok(data: T) -> Self {
        Self::Ok { ok: true, data }
    }

    pub fn err(error: ErrorEnvelope) -> Self {
        Self::Err { ok: false, error }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryObject {
    #[serde(rename = "type")]
    pub object_type: Option<String>,
    pub id: Option<String>,
    pub scope: Option<String>,
    pub status: Option<String>,
    pub spec_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    pub data: Option<Value>,
    pub provenance: Option<Value>,
}

impl MemoryObject {
    pub fn validate(&self) -> Result<ValidatedMemoryObject, ErrorEnvelope> {
        let object_type = self
            .object_type
            .as_ref()
            .and_then(|s| (!s.trim().is_empty()).then(|| s.clone()))
            .ok_or_else(|| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidSchema,
                    "missing required field: type",
                    None,
                )
            })?;

        let id = self
            .id
            .as_ref()
            .and_then(|s| (!s.trim().is_empty()).then(|| s.clone()))
            .ok_or_else(|| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidSchema,
                    "missing required field: id",
                    None,
                )
            })?;

        let scope = self
            .scope
            .as_ref()
            .and_then(|s| (!s.trim().is_empty()).then(|| s.clone()))
            .ok_or_else(|| {
                ErrorEnvelope::new(ErrorCode::ObScopeRequired, "scope is required", None)
            })?;

        let status = self
            .status
            .as_ref()
            .and_then(|s| (!s.trim().is_empty()).then(|| s.clone()))
            .ok_or_else(|| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidSchema,
                    "missing required field: status",
                    None,
                )
            })?;

        let spec_version = self
            .spec_version
            .as_ref()
            .and_then(|s| (!s.trim().is_empty()).then(|| s.clone()))
            .ok_or_else(|| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidSchema,
                    "missing required field: spec_version",
                    None,
                )
            })?;

        if spec_version != SPEC_VERSION {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObUnsupportedVersion,
                format!("unsupported spec_version: {spec_version}"),
                Some(serde_json::json!({ "supported": [SPEC_VERSION] })),
            ));
        }

        let data = self.data.clone().ok_or_else(|| {
            ErrorEnvelope::new(
                ErrorCode::ObInvalidSchema,
                "missing required field: data",
                None,
            )
        })?;

        let provenance = self.provenance.clone().ok_or_else(|| {
            ErrorEnvelope::new(
                ErrorCode::ObInvalidSchema,
                "missing required field: provenance",
                None,
            )
        })?;

        let tags = self.tags.clone().unwrap_or_default();

        Ok(ValidatedMemoryObject {
            object_type,
            id,
            scope,
            status,
            spec_version,
            tags,
            data,
            provenance,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedMemoryObject {
    pub object_type: String,
    pub id: String,
    pub scope: String,
    pub status: String,
    pub spec_version: String,
    pub tags: Vec<String>,
    pub data: Value,
    pub provenance: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryObjectStored {
    #[serde(rename = "type")]
    pub object_type: String,
    pub id: String,
    pub scope: String,
    pub status: String,
    pub spec_version: String,
    pub tags: Vec<String>,
    pub data: Value,
    pub provenance: Value,
    pub version: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectType {
    Entity,
    Relation,
    Claim,
    Decision,
    Task,
    Artifact,
    ThoughtSummary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectStatus {
    Draft,
    Candidate,
    Canonical,
    Deprecated,
    Superseded,
}
