use crate::{EmbedError, EmbeddingProvider};
use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_MODEL: &str = "text-embedding-3-small";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAIConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub default_model: String,
    pub timeout_secs: u64,
    pub embed_dims: Option<u32>,
}

impl OpenAIConfig {
    pub fn from_env() -> Self {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    pub fn from_env_with<F>(mut get: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let api_key = get("OPENAI_API_KEY")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let base_url = get("OPENAI_BASE_URL")
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim()
            .trim_end_matches('/')
            .to_string();

        let default_model = get("OPENAI_EMBED_MODEL")
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
            .trim()
            .to_string();

        let timeout_secs = get("OPENAI_TIMEOUT_SECS")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        let embed_dims = get("OPENAI_EMBED_DIMS").and_then(|v| v.parse::<u32>().ok());

        Self {
            api_key,
            base_url,
            default_model,
            timeout_secs,
            embed_dims,
        }
    }

    fn embeddings_url(&self) -> String {
        let base = if self.base_url.ends_with("/v1") {
            self.base_url.clone()
        } else {
            format!("{}/v1", self.base_url)
        };
        format!("{}/embeddings", base)
    }
}

#[derive(Debug, Clone)]
pub struct OpenAIEmbeddingProvider {
    cfg: OpenAIConfig,
    client: reqwest::Client,
}

impl OpenAIEmbeddingProvider {
    pub fn from_env() -> Self {
        Self::new(OpenAIConfig::from_env())
    }

    pub fn new(cfg: OpenAIConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .expect("reqwest client build");

        Self { cfg, client }
    }

    fn model_for_call<'a>(&'a self, model: &'a str) -> &'a str {
        let trimmed = model.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("default") {
            self.cfg.default_model.as_str()
        } else {
            trimmed
        }
    }

    fn validate_dims(&self) -> Result<Option<u32>, EmbedError> {
        let Some(dims) = self.cfg.embed_dims else {
            return Ok(None);
        };

        if dims != 1536 {
            return Err(EmbedError::invalid_request(
                "OPENAI_EMBED_DIMS must be 1536 for v0.1",
                Some(serde_json::json!({"expected": 1536, "got": dims})),
            ));
        }

        Ok(Some(dims))
    }

    fn auth_error(&self, status: StatusCode, body: Option<String>) -> EmbedError {
        EmbedError::provider_error(
            "openai authentication failed",
            Some(serde_json::json!({
                "status": status.as_u16(),
                "body": body,
            })),
        )
    }

    fn rate_limited(&self, status: StatusCode, body: Option<String>) -> EmbedError {
        EmbedError::provider_error(
            "openai rate limited",
            Some(serde_json::json!({
                "status": status.as_u16(),
                "body": body,
            })),
        )
    }

    fn http_failed(&self, status: StatusCode, body: Option<String>) -> EmbedError {
        EmbedError::provider_error(
            "openai embeddings request failed",
            Some(serde_json::json!({
                "status": status.as_u16(),
                "body": body,
            })),
        )
    }

    async fn extract_error_body(resp: reqwest::Response) -> Option<String> {
        let bytes = resp.bytes().await.ok()?;
        if bytes.is_empty() {
            return None;
        }

        let value: Result<OpenAIErrorResponse, _> = serde_json::from_slice(&bytes);
        if let Ok(v) = value {
            if let Some(err) = v.error {
                return Some(err.message);
            }
        }

        String::from_utf8(bytes.to_vec())
            .ok()
            .map(|s| s.chars().take(1000).collect())
    }
}

#[derive(Debug, Serialize)]
struct EmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    dimensions: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingsData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsData {
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct OpenAIErrorResponse {
    #[serde(default)]
    error: Option<OpenAIErrorBody>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OpenAIErrorBody {
    message: String,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    code: Option<Value>,
}

#[async_trait]
impl EmbeddingProvider for OpenAIEmbeddingProvider {
    async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError> {
        let api_key = self.cfg.api_key.as_deref().unwrap_or("").trim();
        if api_key.is_empty() {
            return Err(EmbedError::provider_error(
                "OPENAI_API_KEY is required for OPENBRAIN_EMBED_PROVIDER=openai",
                Some(serde_json::json!({"env": "OPENAI_API_KEY"})),
            ));
        }

        let dims = self.validate_dims()?;

        let request = EmbeddingsRequest {
            model: self.model_for_call(model),
            input: text,
            dimensions: dims,
        };

        let url = self.cfg.embeddings_url();

        let resp = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    EmbedError::provider_error(
                        "openai request timed out",
                        Some(serde_json::json!({"kind": "timeout"})),
                    )
                } else {
                    EmbedError::provider_error(
                        "openai request failed",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = Self::extract_error_body(resp).await;
            return Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => self.auth_error(status, body),
                StatusCode::TOO_MANY_REQUESTS => self.rate_limited(status, body),
                _ => self.http_failed(status, body),
            });
        }

        let parsed: EmbeddingsResponse = resp.json().await.map_err(|e| {
            EmbedError::provider_error(
                "openai returned invalid JSON",
                Some(serde_json::json!({"error": e.to_string()})),
            )
        })?;

        let Some(first) = parsed.data.into_iter().next() else {
            return Err(EmbedError::provider_error(
                "openai returned empty embeddings data",
                None,
            ));
        };

        Ok(first.embedding)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_base_url_normalization() {
        let cfg = OpenAIConfig::from_env_with(|_| None);
        assert_eq!(cfg.base_url, DEFAULT_BASE_URL);
        assert_eq!(cfg.default_model, DEFAULT_MODEL);
        assert_eq!(cfg.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(cfg.embed_dims, None);
        assert_eq!(cfg.embeddings_url(), "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn dims_env_accepts_only_1536() {
        let cfg = OpenAIConfig::from_env_with(|k| {
            if k == "OPENAI_EMBED_DIMS" {
                Some("2048".to_string())
            } else {
                None
            }
        });
        let provider = OpenAIEmbeddingProvider::new(cfg);
        let err = provider.validate_dims().unwrap_err();
        assert!(err.message().contains("OPENAI_EMBED_DIMS"));
    }
}
