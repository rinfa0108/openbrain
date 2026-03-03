use crate::prompt::JSON_ONLY_SYSTEM;
use crate::LlmError;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
const DEFAULT_MODEL: &str = "claude-3-5-sonnet-latest";
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicConfig {
    pub api_key: Option<String>,
    pub base_url: String,
    pub model: String,
    pub timeout_secs: u64,
}

impl AnthropicConfig {
    pub fn from_env() -> Self {
        Self::from_env_with(|k| std::env::var(k).ok())
    }

    pub fn from_env_with<F>(mut get: F) -> Self
    where
        F: FnMut(&str) -> Option<String>,
    {
        let api_key = get("ANTHROPIC_API_KEY")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let base_url = get("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
            .trim()
            .trim_end_matches('/')
            .to_string();

        let model = get("ANTHROPIC_MODEL")
            .unwrap_or_else(|| DEFAULT_MODEL.to_string())
            .trim()
            .to_string();

        let timeout_secs = get("ANTHROPIC_TIMEOUT_SECS")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECS);

        Self {
            api_key,
            base_url,
            model,
            timeout_secs,
        }
    }

    fn messages_url(&self) -> String {
        let base = if self.base_url.ends_with("/v1") {
            self.base_url.clone()
        } else {
            format!("{}/v1", self.base_url)
        };
        format!("{}/messages", base)
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicClient {
    cfg: AnthropicConfig,
    client: reqwest::Client,
}

impl AnthropicClient {
    pub fn from_env() -> Self {
        Self::new(AnthropicConfig::from_env())
    }

    pub fn new(cfg: AnthropicConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .expect("reqwest client build");
        Self { cfg, client }
    }

    pub fn has_key(&self) -> bool {
        self.cfg
            .api_key
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    }

    pub async fn complete_json(&self, prompt: &str) -> Result<AnthropicModelOutput, LlmError> {
        let api_key = self.cfg.api_key.as_deref().unwrap_or("").trim();
        if api_key.is_empty() {
            return Err(LlmError::MissingApiKey);
        }

        let req = MessagesRequest {
            model: &self.cfg.model,
            max_tokens: 800,
            temperature: 0.0,
            system: JSON_ONLY_SYSTEM,
            messages: vec![Message {
                role: "user",
                content: vec![ContentBlock {
                    r#type: "text",
                    text: prompt.to_string(),
                }],
            }],
        };

        let resp = self
            .client
            .post(self.cfg.messages_url())
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&req)
            .send()
            .await
            .map_err(|e| LlmError::ProviderError {
                message: if e.is_timeout() {
                    "anthropic request timed out".to_string()
                } else {
                    "anthropic request failed".to_string()
                },
                details: Some(serde_json::json!({"error": e.to_string()})),
            })?;

        let status = resp.status();
        if !status.is_success() {
            let body = extract_error_body(resp).await;
            return Err(match status {
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => LlmError::InvalidRequest {
                    message: "anthropic authentication failed".to_string(),
                    details: body,
                },
                StatusCode::TOO_MANY_REQUESTS => LlmError::RateLimited {
                    message: "anthropic rate limited".to_string(),
                    details: body,
                },
                _ => LlmError::ProviderError {
                    message: "anthropic request failed".to_string(),
                    details: body,
                },
            });
        }

        let parsed: MessagesResponse = resp.json().await.map_err(|e| LlmError::ProviderError {
            message: "anthropic returned invalid JSON".to_string(),
            details: Some(serde_json::json!({"error": e.to_string()})),
        })?;

        let text = parsed
            .content
            .into_iter()
            .find(|c| c.r#type == "text")
            .map(|c| c.text)
            .unwrap_or_default();

        Ok(AnthropicModelOutput { text })
    }
}

#[derive(Debug, Serialize)]
struct MessagesRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    temperature: f32,
    system: &'a str,
    messages: Vec<Message>,
}

#[derive(Debug, Serialize)]
struct Message {
    role: &'static str,
    content: Vec<ContentBlock>,
}

#[derive(Debug, Serialize)]
struct ContentBlock {
    r#type: &'static str,
    text: String,
}

#[derive(Debug, Deserialize)]
struct MessagesResponse {
    content: Vec<ContentBlockResponse>,
}

#[derive(Debug, Deserialize)]
struct ContentBlockResponse {
    r#type: String,
    text: String,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorResponse {
    #[serde(default)]
    error: Option<AnthropicErrorBody>,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorBody {
    message: String,
}

async fn extract_error_body(resp: reqwest::Response) -> Option<Value> {
    let bytes = resp.bytes().await.ok()?;
    if bytes.is_empty() {
        return None;
    }

    let value: Result<AnthropicErrorResponse, _> = serde_json::from_slice(&bytes);
    if let Ok(v) = value {
        if let Some(err) = v.error {
            return Some(serde_json::json!({"message": err.message}));
        }
    }

    String::from_utf8(bytes.to_vec())
        .ok()
        .map(|s| serde_json::json!({"message": s.chars().take(1000).collect::<String>()}))
}

pub mod prompt {
    pub const JSON_ONLY_SYSTEM: &str =
        "You are a JSON-only API. Return only strict JSON, no markdown, no commentary.";

    pub fn build_rerank_prompt(query: &str, candidates_json: &str) -> String {
        format!(
            "Rerank the candidates for the query. Return JSON with keys: ranked_refs (array of strings), rationale_short (array of {{ref, why}}). Only use provided candidates.\n\nQuery: {}\n\nCandidates JSON: {}",
            query, candidates_json
        )
    }

    pub fn build_pack_prompt(
        scope: &str,
        task_hint: &str,
        refs_json: &str,
        notes_json: &str,
    ) -> String {
        format!(
            "Build a short summary and next_actions for a memory pack. Return JSON with keys: summary (string), next_actions (array of strings). Only use provided refs and notes.\n\nScope: {}\nTask: {}\n\nRefs JSON: {}\n\nNotes JSON: {}",
            scope, task_hint, refs_json, notes_json
        )
    }
}

#[derive(Debug, Clone)]
pub struct AnthropicModelOutput {
    pub text: String,
}
