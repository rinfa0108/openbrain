mod anthropic;

use serde_json::Value;

pub use anthropic::prompt;
pub use anthropic::{AnthropicClient, AnthropicConfig, AnthropicModelOutput};

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("missing api key")]
    MissingApiKey,
    #[error("invalid request: {message}")]
    InvalidRequest {
        message: String,
        details: Option<Value>,
    },
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        details: Option<Value>,
    },
    #[error("provider error: {message}")]
    ProviderError {
        message: String,
        details: Option<Value>,
    },
    #[error("invalid model output: {message}")]
    InvalidModelOutput {
        message: String,
        details: Option<Value>,
    },
}

impl LlmError {
    pub fn details(&self) -> Option<&Value> {
        match self {
            Self::InvalidRequest { details, .. } => details.as_ref(),
            Self::RateLimited { details, .. } => details.as_ref(),
            Self::ProviderError { details, .. } => details.as_ref(),
            Self::InvalidModelOutput { details, .. } => details.as_ref(),
            _ => None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::MissingApiKey => "ANTHROPIC_API_KEY is required".to_string(),
            Self::InvalidRequest { message, .. } => message.clone(),
            Self::RateLimited { message, .. } => message.clone(),
            Self::ProviderError { message, .. } => message.clone(),
            Self::InvalidModelOutput { message, .. } => message.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::anthropic::prompt::*;

    #[test]
    fn rerank_prompt_contains_required_fields() {
        let prompt = build_rerank_prompt("query", "{}");
        assert!(prompt.contains("ranked_refs"));
        assert!(prompt.contains("rationale_short"));
    }

    #[test]
    fn pack_prompt_contains_required_fields() {
        let prompt = build_pack_prompt("scope", "task", "{}", "{}");
        assert!(prompt.contains("summary"));
        assert!(prompt.contains("next_actions"));
    }
}
