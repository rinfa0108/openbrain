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

#[async_trait]
pub trait Store: Send + Sync {
    async fn put_objects(&self, req: PutObjectsRequest) -> Envelope<PutObjectsResponse>;

    async fn get_objects(&self, req: GetObjectsRequest) -> Envelope<GetObjectsResponse>;

    async fn append_event(
        &self,
        scope: &str,
        event_type: &str,
        actor: &str,
        payload_json: Value,
    ) -> ();
}
