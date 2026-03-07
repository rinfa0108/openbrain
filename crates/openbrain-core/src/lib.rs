use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod query;
pub mod textnorm;

pub const SPEC_VERSION: &str = "0.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleState {
    Scratch,
    Candidate,
    Accepted,
    Deprecated,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scratch => "scratch",
            Self::Candidate => "candidate",
            Self::Accepted => "accepted",
            Self::Deprecated => "deprecated",
        }
    }
}

impl std::str::FromStr for LifecycleState {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "scratch" => Ok(Self::Scratch),
            "candidate" => Ok(Self::Candidate),
            "accepted" => Ok(Self::Accepted),
            "deprecated" => Ok(Self::Deprecated),
            _ => Err(()),
        }
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<LifecycleState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_key: Option<String>,
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
        let lifecycle_state = self.lifecycle_state.unwrap_or(LifecycleState::Accepted);
        let expires_at = self
            .expires_at
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let memory_key = self
            .memory_key
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        Ok(ValidatedMemoryObject {
            object_type,
            id,
            scope,
            status,
            spec_version,
            tags,
            data,
            provenance,
            lifecycle_state,
            expires_at,
            memory_key,
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
    pub lifecycle_state: LifecycleState,
    pub expires_at: Option<String>,
    pub memory_key: Option<String>,
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
    pub lifecycle_state: LifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_key: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_state_defaults_to_accepted() {
        let obj = MemoryObject {
            object_type: Some("claim".to_string()),
            id: Some("obj-1".to_string()),
            scope: Some("scope-1".to_string()),
            status: Some("draft".to_string()),
            spec_version: Some(SPEC_VERSION.to_string()),
            tags: Some(vec![]),
            data: Some(serde_json::json!({"k": "v"})),
            provenance: Some(serde_json::json!({"actor": "tester"})),
            lifecycle_state: None,
            expires_at: None,
            memory_key: None,
        };

        let validated = obj.validate().expect("valid");
        assert_eq!(validated.lifecycle_state, LifecycleState::Accepted);
    }

    #[test]
    fn lifecycle_state_parsing_accepts_known_values() {
        use std::str::FromStr;
        assert_eq!(
            LifecycleState::from_str("scratch").ok(),
            Some(LifecycleState::Scratch)
        );
        assert_eq!(
            LifecycleState::from_str("candidate").ok(),
            Some(LifecycleState::Candidate)
        );
        assert_eq!(
            LifecycleState::from_str("accepted").ok(),
            Some(LifecycleState::Accepted)
        );
        assert_eq!(
            LifecycleState::from_str("deprecated").ok(),
            Some(LifecycleState::Deprecated)
        );
    }
}
