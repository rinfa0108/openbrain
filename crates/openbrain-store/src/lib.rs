mod pg;

use async_trait::async_trait;
use openbrain_core::{
    ConflictStatus, Envelope, ErrorEnvelope, LifecycleState, MemoryObject, MemoryObjectStored,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use pg::{hash_token, PgStore};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PutObjectsRequest {
    pub objects: Vec<MemoryObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PutResult {
    pub r#ref: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub status: String,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PutObjectsResponse {
    pub results: Vec<PutResult>,
    #[serde(default)]
    pub replayed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default)]
    pub accepted_count: usize,
    #[serde(default)]
    pub object_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetObjectsRequest {
    pub scope: String,
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_states: Option<Vec<LifecycleState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_conflicts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetObjectsResponse {
    pub objects: Vec<MemoryObjectStored>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderBySpec {
    pub field: String,
    pub direction: OrderDirection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchStructuredRequest {
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub where_expr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_by: Option<OrderBySpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_states: Option<Vec<LifecycleState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_conflicts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchItem {
    pub r#ref: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub status: String,
    pub updated_at: String,
    pub version: i64,
    #[serde(default)]
    pub conflict: bool,
    #[serde(default)]
    pub conflict_status: ConflictStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_object_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchStructuredResponse {
    pub results: Vec<SearchItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchSemanticRequest {
    pub scope: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filters: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_states: Option<Vec<LifecycleState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_conflicts: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub r#ref: String,
    pub kind: String,
    pub score: f32,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default)]
    pub conflict: bool,
    #[serde(default)]
    pub conflict_status: ConflictStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflicting_object_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchSemanticResponse {
    pub matches: Vec<SearchMatch>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbedTarget {
    Text { text: String },
    Ref { r#ref: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedGenerateRequest {
    pub scope: String,
    pub target: EmbedTarget,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dims: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedGenerateResponse {
    pub embedding_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    pub model: String,
    pub dims: i32,
    pub checksum: String,
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingCoverageRequest {
    pub scope: String,
    pub provider: String,
    pub model: String,
    pub kind: String,
    pub state: LifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing_sample_limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingCoverageResponse {
    pub total_eligible: u64,
    pub with_embeddings: u64,
    pub missing: u64,
    pub percent_coverage: f64,
    pub missing_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingReembedRequest {
    pub scope: String,
    pub to_provider: String,
    pub to_model: String,
    pub to_kind: String,
    pub state: LifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_objects: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingReembedResponse {
    pub scanned: u32,
    pub processed: u32,
    pub next_cursor: Option<String>,
    pub dry_run: bool,
    pub bytes_processed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceRole {
    Owner,
    Writer,
    Reader,
}

impl WorkspaceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Writer => "writer",
            Self::Reader => "reader",
        }
    }

    pub fn can_read(self) -> bool {
        matches!(self, Self::Owner | Self::Writer | Self::Reader)
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Owner | Self::Writer)
    }

    pub fn can_admin(self) -> bool {
        matches!(self, Self::Owner)
    }
}

impl std::str::FromStr for WorkspaceRole {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "owner" => Ok(Self::Owner),
            "writer" => Ok(Self::Writer),
            "reader" => Ok(Self::Reader),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthContext {
    pub identity_id: String,
    pub workspace_id: String,
    pub role: WorkspaceRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenCreateRequest {
    pub workspace_id: String,
    pub role: WorkspaceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenCreateResponse {
    pub token: String,
    pub workspace_id: String,
    pub role: WorkspaceRole,
    pub identity_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfoResponse {
    pub workspace_id: String,
    pub owner_identity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_display_name: Option<String>,
    pub caller_identity_id: String,
    pub caller_role: WorkspaceRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRequest {
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditObjectTimelineRequest {
    #[serde(flatten)]
    pub query: AuditRequest,
    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditMemoryKeyTimelineRequest {
    #[serde(flatten)]
    pub query: AuditRequest,
    pub memory_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditActorActivityRequest {
    #[serde(flatten)]
    pub query: AuditRequest,
    pub actor_identity_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub event_type: String,
    pub actor_identity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_key: Option<String>,
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditResponse {
    pub events: Vec<AuditEvent>,
    pub limit: u32,
    pub offset: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BootstrapToken {
    pub token: String,
    pub workspace_id: String,
    pub role: WorkspaceRole,
}

#[async_trait]
pub trait Store: Send + Sync {
    async fn put_objects(&self, req: PutObjectsRequest) -> Envelope<PutObjectsResponse>;

    async fn get_objects(&self, req: GetObjectsRequest) -> Envelope<GetObjectsResponse>;

    async fn search_structured(
        &self,
        req: SearchStructuredRequest,
    ) -> Envelope<SearchStructuredResponse>;

    async fn embed_generate(&self, req: EmbedGenerateRequest) -> Envelope<EmbedGenerateResponse>;

    async fn search_semantic(&self, req: SearchSemanticRequest)
        -> Envelope<SearchSemanticResponse>;

    async fn append_event(
        &self,
        scope: &str,
        event_type: &str,
        actor: &str,
        payload_json: Value,
    ) -> Result<(), ErrorEnvelope>;

    async fn audit_object_timeline(
        &self,
        req: AuditObjectTimelineRequest,
    ) -> Envelope<AuditResponse>;

    async fn audit_memory_key_timeline(
        &self,
        req: AuditMemoryKeyTimelineRequest,
    ) -> Envelope<AuditResponse>;

    async fn audit_actor_activity(&self, req: AuditActorActivityRequest)
        -> Envelope<AuditResponse>;
}

#[async_trait]
pub trait AuthStore: Send + Sync {
    async fn auth_from_token(
        &self,
        token: &str,
    ) -> Result<AuthContext, openbrain_core::ErrorEnvelope>;

    async fn create_token(
        &self,
        req: TokenCreateRequest,
    ) -> Result<TokenCreateResponse, openbrain_core::ErrorEnvelope>;

    async fn bootstrap_default_workspace(
        &self,
    ) -> Result<Option<BootstrapToken>, openbrain_core::ErrorEnvelope>;

    async fn workspace_info(
        &self,
        workspace_id: &str,
        caller_identity_id: &str,
        caller_role: WorkspaceRole,
    ) -> Result<WorkspaceInfoResponse, openbrain_core::ErrorEnvelope>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_parsing_accepts_known_roles() {
        use std::str::FromStr;
        assert_eq!(
            WorkspaceRole::from_str("owner").ok(),
            Some(WorkspaceRole::Owner)
        );
        assert_eq!(
            WorkspaceRole::from_str("writer").ok(),
            Some(WorkspaceRole::Writer)
        );
        assert_eq!(
            WorkspaceRole::from_str("reader").ok(),
            Some(WorkspaceRole::Reader)
        );
    }

    #[test]
    fn role_permissions_match_expectations() {
        assert!(WorkspaceRole::Owner.can_admin());
        assert!(WorkspaceRole::Owner.can_write());
        assert!(WorkspaceRole::Owner.can_read());

        assert!(!WorkspaceRole::Writer.can_admin());
        assert!(WorkspaceRole::Writer.can_write());
        assert!(WorkspaceRole::Writer.can_read());

        assert!(!WorkspaceRole::Reader.can_admin());
        assert!(!WorkspaceRole::Reader.can_write());
        assert!(WorkspaceRole::Reader.can_read());
    }

    #[test]
    fn token_hash_is_deterministic() {
        let a = hash_token("token-123");
        let b = hash_token("token-123");
        assert_eq!(a, b);
        assert_ne!(a, hash_token("token-456"));
    }
}
