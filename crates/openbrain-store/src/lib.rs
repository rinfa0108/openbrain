mod pg;

use async_trait::async_trait;
use openbrain_core::{Envelope, MemoryObject, MemoryObjectStored};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use pg::PgStore;

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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetObjectsRequest {
    pub refs: Vec<String>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchItem {
    pub r#ref: String,
    #[serde(rename = "type")]
    pub object_type: String,
    pub status: String,
    pub updated_at: String,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchStructuredResponse {
    pub results: Vec<SearchItem>,
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

#[async_trait]
pub trait Store: Send + Sync {
    async fn put_objects(&self, req: PutObjectsRequest) -> Envelope<PutObjectsResponse>;

    async fn get_objects(&self, req: GetObjectsRequest) -> Envelope<GetObjectsResponse>;

    async fn search_structured(
        &self,
        req: SearchStructuredRequest,
    ) -> Envelope<SearchStructuredResponse>;

    async fn embed_generate(&self, req: EmbedGenerateRequest) -> Envelope<EmbedGenerateResponse>;

    async fn append_event(
        &self,
        scope: &str,
        event_type: &str,
        actor: &str,
        payload_json: Value,
    ) -> ();
}
