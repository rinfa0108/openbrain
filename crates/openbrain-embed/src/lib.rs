mod openai;

use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use openai::{OpenAIConfig, OpenAIEmbeddingProvider};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("provider unavailable")]
    ProviderUnavailable,

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("invalid request: {message}")]
    InvalidRequest {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        details: Option<Value>,
    },

    #[error("provider error: {message}")]
    ProviderError {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        details: Option<Value>,
    },
}

impl EmbedError {
    pub fn invalid_request(message: impl Into<String>, details: Option<Value>) -> Self {
        Self::InvalidRequest {
            message: message.into(),
            source: None,
            details,
        }
    }

    pub fn provider_error(message: impl Into<String>, details: Option<Value>) -> Self {
        Self::ProviderError {
            message: message.into(),
            source: None,
            details,
        }
    }

    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::InvalidRequest { details, .. } => details.as_ref(),
            Self::ProviderError { details, .. } => details.as_ref(),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::ProviderUnavailable => "embedding provider unavailable".to_string(),
            Self::InvalidInput(m) => m.clone(),
            Self::InvalidRequest { message, .. } => message.clone(),
            Self::ProviderError { message, .. } => message.clone(),
        }
    }
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError>;
}

#[derive(Debug, Default, Clone)]
pub struct NoopEmbeddingProvider;

#[async_trait]
impl EmbeddingProvider for NoopEmbeddingProvider {
    async fn embed(&self, _model: &str, _text: &str) -> Result<Vec<f32>, EmbedError> {
        Err(EmbedError::ProviderUnavailable)
    }
}

#[derive(Debug, Default, Clone)]
pub struct FakeEmbeddingProvider;

impl FakeEmbeddingProvider {
    pub const DIMS: usize = 1536;

    fn hash_bytes(model: &str, text: &str, counter: u32) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(model.as_bytes());
        h.update(b"\n");
        h.update(text.as_bytes());
        h.update(b"\n");
        h.update(counter.to_le_bytes());
        h.finalize().into()
    }
}

#[async_trait]
impl EmbeddingProvider for FakeEmbeddingProvider {
    async fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError> {
        if text.is_empty() {
            return Err(EmbedError::InvalidInput("text is empty".to_string()));
        }

        let mut out = Vec::with_capacity(Self::DIMS);
        let mut counter = 0u32;
        while out.len() < Self::DIMS {
            let bytes = Self::hash_bytes(model, text, counter);
            counter = counter.wrapping_add(1);
            for chunk in bytes.chunks_exact(4) {
                if out.len() >= Self::DIMS {
                    break;
                }
                let u = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let f = (u as f64 / u32::MAX as f64) * 2.0 - 1.0;
                out.push(f as f32);
            }
        }
        Ok(out)
    }
}

pub fn embedder_from_env(name: &str) -> std::sync::Arc<dyn EmbeddingProvider> {
    match name.trim().to_ascii_lowercase().as_str() {
        "fake" => std::sync::Arc::new(FakeEmbeddingProvider),
        "openai" => std::sync::Arc::new(OpenAIEmbeddingProvider::from_env()),
        _ => std::sync::Arc::new(NoopEmbeddingProvider),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn select_noop_returns_provider_unavailable() {
        let e = embedder_from_env("noop")
            .embed("default", "hello")
            .await
            .unwrap_err();
        matches!(e, EmbedError::ProviderUnavailable);
    }

    #[tokio::test]
    async fn select_fake_produces_1536_dims() {
        let v = embedder_from_env("fake")
            .embed("default", "hello")
            .await
            .unwrap();
        assert_eq!(v.len(), FakeEmbeddingProvider::DIMS);
    }

    #[tokio::test]
    async fn openai_missing_api_key_is_deterministic_error() {
        let provider = OpenAIEmbeddingProvider::new(OpenAIConfig {
            api_key: None,
            base_url: "https://api.openai.com".to_string(),
            default_model: "text-embedding-3-small".to_string(),
            timeout_secs: 30,
            embed_dims: None,
        });

        let err = provider.embed("default", "hello").await.unwrap_err();
        let msg = err.message();
        assert!(msg.contains("OPENAI_API_KEY"));
    }
}
