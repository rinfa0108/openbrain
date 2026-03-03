use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("provider unavailable")]
    ProviderUnavailable,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("provider error: {0}")]
    ProviderError(String),
}

pub trait EmbeddingProvider: Send + Sync {
    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError>;
}

#[derive(Debug, Default, Clone)]
pub struct NoopEmbeddingProvider;

impl EmbeddingProvider for NoopEmbeddingProvider {
    fn embed(&self, _model: &str, _text: &str) -> Result<Vec<f32>, EmbedError> {
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

impl EmbeddingProvider for FakeEmbeddingProvider {
    fn embed(&self, model: &str, text: &str) -> Result<Vec<f32>, EmbedError> {
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
