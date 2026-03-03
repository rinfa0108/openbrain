use crate::{EmbedError, EmbeddingProvider};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

const DEFAULT_TIMEOUT_SECS: u64 = 15;
const HEADER_PREFIX: &str = "LOCAL_EMBED_HEADER_";
const EXPECTED_DIMS: usize = 1536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalHttpConfig {
    pub url: Option<String>,
    pub model: Option<String>,
    pub timeout_secs: u64,
    pub headers: BTreeMap<String, String>,
}

impl LocalHttpConfig {
    pub fn from_env() -> Self {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    pub fn from_env_with<F>(mut get: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let url = get("LOCAL_EMBED_URL")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let model = get("LOCAL_EMBED_MODEL")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let timeout_secs = get("LOCAL_EMBED_TIMEOUT_SECS")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let mut headers = BTreeMap::new();
        for (k, v) in std::env::vars() {
            if let Some(name) = k.strip_prefix(HEADER_PREFIX) {
                let trimmed = name.trim().replace('_', "-");
                if !trimmed.is_empty() && !v.trim().is_empty() {
                    headers.insert(trimmed, v);
                }
            }
        }

        Self {
            url,
            model,
            timeout_secs,
            headers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalHttpEmbeddingProvider {
    cfg: LocalHttpConfig,
    client: reqwest::Client,
}

impl LocalHttpEmbeddingProvider {
    pub fn from_env() -> Self {
        Self::new(LocalHttpConfig::from_env())
    }

    pub fn new(cfg: LocalHttpConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .expect("reqwest client build");
        Self { cfg, client }
    }

    fn headers(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in &self.cfg.headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                map.insert(name, value);
            }
        }
        map
    }

    fn required_url(&self) -> Result<&str, EmbedError> {
        let url = self.cfg.url.as_deref().unwrap_or("").trim();
        if url.is_empty() {
            return Err(EmbedError::provider_error(
                "LOCAL_EMBED_URL is required for OPENBRAIN_EMBED_PROVIDER=local",
                Some(serde_json::json!({"env": "LOCAL_EMBED_URL"})),
            ));
        }
        Ok(url)
    }
}

#[derive(Debug, Serialize)]
struct LocalEmbedRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    input: &'a str,
}

#[derive(Debug, Deserialize)]
struct LocalEmbedResponse {
    data: Vec<LocalEmbedItem>,
}

#[derive(Debug, Deserialize)]
struct LocalEmbedItem {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingProvider for LocalHttpEmbeddingProvider {
    async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError> {
        let url = self.required_url()?;
        let model_name = model.trim();
        let request = LocalEmbedRequest {
            model: if model_name.is_empty() || model_name.eq_ignore_ascii_case("default") {
                self.cfg.model.as_deref()
            } else {
                Some(model_name)
            },
            input: text,
        };

        let resp = self
            .client
            .post(url)
            .headers(self.headers())
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    EmbedError::provider_error(
                        "local embedding request timed out",
                        Some(serde_json::json!({"kind": "timeout"})),
                    )
                } else {
                    EmbedError::provider_error(
                        "local embedding request failed",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            return Err(EmbedError::provider_error(
                "local embedding request failed",
                Some(serde_json::json!({"status": status.as_u16()})),
            ));
        }

        let parsed: LocalEmbedResponse = resp.json().await.map_err(|e| {
            EmbedError::provider_error(
                "local embeddings returned invalid JSON",
                Some(serde_json::json!({"error": e.to_string()})),
            )
        })?;

        let Some(first) = parsed.data.into_iter().next() else {
            return Err(EmbedError::provider_error(
                "local embeddings returned empty data",
                None,
            ));
        };

        if first.embedding.len() != EXPECTED_DIMS {
            return Err(EmbedError::provider_error(
                "local embeddings returned unexpected dimensions",
                Some(serde_json::json!({
                    "expected": EXPECTED_DIMS,
                    "got": first.embedding.len()
                })),
            ));
        }

        Ok(first.embedding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::json;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn missing_url_is_deterministic_error() {
        let provider = LocalHttpEmbeddingProvider::new(LocalHttpConfig {
            url: None,
            model: None,
            timeout_secs: 1,
            headers: BTreeMap::new(),
        });

        let err = provider.embed("default", "hello").await.unwrap_err();
        assert!(err.message().contains("LOCAL_EMBED_URL"));
    }

    async fn spawn_mock_server(handler: Router) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, handler).await.unwrap();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn successful_embedding_returns_1536() {
        let handler = Router::new().route(
            "/embeddings",
            post(|| async {
                let embedding = vec![0.0f32; 1536];
                Json(json!({ "data": [{ "embedding": embedding }] }))
            }),
        );
        let base_url = spawn_mock_server(handler).await;
        let provider = LocalHttpEmbeddingProvider::new(LocalHttpConfig {
            url: Some(format!("{}/embeddings", base_url)),
            model: None,
            timeout_secs: 2,
            headers: BTreeMap::new(),
        });

        let v = provider.embed("default", "hello").await.unwrap();
        assert_eq!(v.len(), 1536);
    }

    #[tokio::test]
    async fn invalid_json_rejected() {
        let handler = Router::new().route("/embeddings", post(|| async { "not-json" }));
        let base_url = spawn_mock_server(handler).await;
        let provider = LocalHttpEmbeddingProvider::new(LocalHttpConfig {
            url: Some(format!("{}/embeddings", base_url)),
            model: None,
            timeout_secs: 2,
            headers: BTreeMap::new(),
        });

        let err = provider.embed("default", "hello").await.unwrap_err();
        assert!(err.message().contains("invalid JSON"));
    }

    #[tokio::test]
    async fn non_2xx_rejected() {
        let handler = Router::new().route(
            "/embeddings",
            post(|| async { (axum::http::StatusCode::BAD_REQUEST, "oops") }),
        );
        let base_url = spawn_mock_server(handler).await;
        let provider = LocalHttpEmbeddingProvider::new(LocalHttpConfig {
            url: Some(format!("{}/embeddings", base_url)),
            model: None,
            timeout_secs: 2,
            headers: BTreeMap::new(),
        });

        let err = provider.embed("default", "hello").await.unwrap_err();
        let msg = err.message();
        assert!(msg.contains("request failed"));
    }

    #[tokio::test]
    async fn dims_mismatch_rejected_by_caller() {
        let handler = Router::new().route(
            "/embeddings",
            post(|| async {
                let embedding = vec![0.0f32; 8];
                Json(json!({ "data": [{ "embedding": embedding }] }))
            }),
        );
        let base_url = spawn_mock_server(handler).await;
        let provider = LocalHttpEmbeddingProvider::new(LocalHttpConfig {
            url: Some(format!("{}/embeddings", base_url)),
            model: None,
            timeout_secs: 2,
            headers: BTreeMap::new(),
        });

        let err = provider.embed("default", "hello").await.unwrap_err();
        assert!(err.message().contains("unexpected dimensions"));
    }
}
