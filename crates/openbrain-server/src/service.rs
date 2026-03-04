use openbrain_core::{Envelope, ErrorCode, ErrorEnvelope};
use openbrain_llm::prompt::{build_pack_prompt, build_rerank_prompt};
use openbrain_llm::{AnthropicClient, LlmError};
use openbrain_store::{GetObjectsRequest, SearchSemanticRequest, SearchStructuredRequest, Store};
use serde::{Deserialize, Serialize};

const MAX_CANDIDATES: usize = 50;
const MAX_SNIPPET_LEN: usize = 400;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankRequest {
    pub scope: String,
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidates: Option<RerankCandidates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RerankCandidates {
    Refs { refs: Vec<String> },
    Docs { candidates: Vec<CandidateDoc> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateDoc {
    pub r#ref: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RerankResponse {
    pub ranked_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale_short: Option<Vec<RationaleItem>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RationaleItem {
    pub r#ref: String,
    pub why: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPackRequest {
    pub scope: String,
    pub task_hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PackPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PackPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_items: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_types: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_status: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPackResponse {
    pub pack: MemoryPack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPack {
    pub scope: String,
    pub canonical: Vec<String>,
    pub constraints: Vec<String>,
    pub relevant: Vec<String>,
    pub conflicts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent: Option<Vec<String>>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_actions: Option<Vec<String>>,
}

pub async fn rerank<S>(
    store: &S,
    llm: &AnthropicClient,
    req: RerankRequest,
) -> Envelope<RerankResponse>
where
    S: Store + Clone + 'static,
{
    if let Err(e) = validate_rerank(&req) {
        return Envelope::err(e);
    }

    let top_k = req.top_k.unwrap_or(10).min(50) as usize;

    let candidate_docs = match req.candidates {
        Some(RerankCandidates::Docs { candidates }) => candidates,
        Some(RerankCandidates::Refs { refs }) => {
            match fetch_candidate_docs(store, &req.scope, refs).await {
                Ok(v) => v,
                Err(e) => return Envelope::err(e),
            }
        }
        None => {
            let search = store
                .search_semantic(SearchSemanticRequest {
                    scope: req.scope.clone(),
                    query: req.query.clone(),
                    top_k: Some(MAX_CANDIDATES as u32),
                    model: None,
                    embedding_provider: None,
                    embedding_model: None,
                    embedding_kind: None,
                    filters: None,
                    types: None,
                    status: None,
                })
                .await;
            let refs = match search {
                Envelope::Ok { data, .. } => data.matches.into_iter().map(|m| m.r#ref).collect(),
                Envelope::Err { error, .. } => return Envelope::err(error),
            };
            match fetch_candidate_docs(store, &req.scope, refs).await {
                Ok(v) => v,
                Err(e) => return Envelope::err(e),
            }
        }
    };

    let mut candidates = candidate_docs;
    candidates.truncate(MAX_CANDIDATES);

    let candidates_json = serde_json::to_string(&candidates).unwrap_or_else(|_| "[]".to_string());
    let prompt = build_rerank_prompt(&req.query, &candidates_json);

    let model_out = match llm.complete_json(&prompt).await {
        Ok(v) => v,
        Err(e) => return Envelope::err(map_llm_error(e)),
    };

    let parsed: RerankResponse = match serde_json::from_str(&model_out.text) {
        Ok(v) => v,
        Err(e) => {
            return Envelope::err(ErrorEnvelope::new(
                ErrorCode::ObInternal,
                "invalid model output (expected JSON)",
                Some(serde_json::json!({"error": e.to_string()})),
            ))
        }
    };

    let mut ranked_refs = parsed.ranked_refs;
    ranked_refs.truncate(top_k);

    Envelope::ok(RerankResponse {
        ranked_refs,
        rationale_short: parsed.rationale_short,
    })
}

pub async fn build_pack<S>(
    store: &S,
    llm: &AnthropicClient,
    req: MemoryPackRequest,
) -> Envelope<MemoryPackResponse>
where
    S: Store + Clone + 'static,
{
    if req.scope.trim().is_empty() {
        return Envelope::err(ErrorEnvelope::new(
            ErrorCode::ObScopeRequired,
            "scope is required",
            None,
        ));
    }
    if req.task_hint.trim().is_empty() {
        return Envelope::err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "task_hint is required",
            None,
        ));
    }

    let policy = req.policy.clone().unwrap_or_default();
    let max_items = policy.max_items.unwrap_or(10).min(50) as usize;

    let canonical = fetch_canonical_decisions(store, &req.scope, max_items)
        .await
        .unwrap_or_default();

    let relevant = if let Some(query) = req.query.as_ref() {
        match rerank(
            store,
            llm,
            RerankRequest {
                scope: req.scope.clone(),
                query: query.clone(),
                candidates: None,
                top_k: Some(max_items as u32),
            },
        )
        .await
        {
            Envelope::Ok { data, .. } => data.ranked_refs,
            Envelope::Err { error, .. } => return Envelope::err(error),
        }
    } else {
        Vec::new()
    };

    let pack = MemoryPack {
        scope: req.scope.clone(),
        canonical,
        constraints: Vec::new(),
        relevant,
        conflicts: Vec::new(),
        recent: None,
        summary: String::new(),
        next_actions: None,
    };

    let summary = match summarize_pack(llm, &pack, &req.task_hint).await {
        Ok((summary, next_actions)) => (summary, next_actions),
        Err(e) => return Envelope::err(e),
    };

    Envelope::ok(MemoryPackResponse {
        pack: MemoryPack {
            summary: summary.0,
            next_actions: summary.1,
            ..pack
        },
    })
}

async fn summarize_pack(
    llm: &AnthropicClient,
    pack: &MemoryPack,
    task_hint: &str,
) -> Result<(String, Option<Vec<String>>), ErrorEnvelope> {
    let refs_json = serde_json::to_string(&serde_json::json!({
        "canonical": pack.canonical,
        "relevant": pack.relevant,
        "constraints": pack.constraints,
        "conflicts": pack.conflicts
    }))
    .unwrap_or_else(|_| "{}".to_string());

    let notes_json = serde_json::to_string(&serde_json::json!({"task_hint": task_hint}))
        .unwrap_or_else(|_| "{}".to_string());

    let prompt = build_pack_prompt(&pack.scope, task_hint, &refs_json, &notes_json);

    let model_out = llm.complete_json(&prompt).await.map_err(map_llm_error)?;

    #[derive(Deserialize)]
    struct SummaryOut {
        summary: String,
        #[serde(default)]
        next_actions: Option<Vec<String>>,
    }

    let parsed: SummaryOut = serde_json::from_str(&model_out.text).map_err(|e| {
        ErrorEnvelope::new(
            ErrorCode::ObInternal,
            "invalid model output (expected JSON)",
            Some(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    Ok((parsed.summary, parsed.next_actions))
}

async fn fetch_canonical_decisions<S>(
    store: &S,
    scope: &str,
    max_items: usize,
) -> Result<Vec<String>, ErrorEnvelope>
where
    S: Store + Clone + 'static,
{
    let res = store
        .search_structured(SearchStructuredRequest {
            scope: scope.to_string(),
            where_expr: Some("type == \"decision\" AND status == \"canonical\"".to_string()),
            limit: Some(max_items as u32),
            offset: Some(0),
            order_by: None,
        })
        .await;

    match res {
        Envelope::Ok { data, .. } => Ok(data.results.into_iter().map(|r| r.r#ref).collect()),
        Envelope::Err { error, .. } => Err(error),
    }
}

async fn fetch_candidate_docs<S>(
    store: &S,
    scope: &str,
    refs: Vec<String>,
) -> Result<Vec<CandidateDoc>, ErrorEnvelope>
where
    S: Store + Clone + 'static,
{
    if refs.is_empty() {
        return Ok(Vec::new());
    }

    let res = store.get_objects(GetObjectsRequest { refs }).await;
    let data = match res {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => return Err(error),
    };

    let mut docs = Vec::new();
    for obj in data.objects {
        if obj.scope != scope {
            continue;
        }
        let snippet = truncate_snippet(&serde_json::to_string(&obj.data).unwrap_or_default());
        docs.push(CandidateDoc {
            r#ref: obj.id,
            kind: obj.object_type,
            snippet,
        });
    }

    Ok(docs)
}

fn truncate_snippet(s: &str) -> String {
    let mut out = s.to_string();
    if out.len() > MAX_SNIPPET_LEN {
        out.truncate(MAX_SNIPPET_LEN);
    }
    out
}

fn validate_rerank(req: &RerankRequest) -> Result<(), ErrorEnvelope> {
    if req.scope.trim().is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObScopeRequired,
            "scope is required",
            None,
        ));
    }
    if req.query.trim().is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "query is required",
            None,
        ));
    }
    Ok(())
}

fn map_llm_error(err: LlmError) -> ErrorEnvelope {
    match err {
        LlmError::MissingApiKey => ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "ANTHROPIC_API_KEY is required",
            Some(serde_json::json!({"env": "ANTHROPIC_API_KEY"})),
        ),
        LlmError::InvalidRequest { message, details } => {
            ErrorEnvelope::new(ErrorCode::ObInvalidRequest, message, details)
        }
        LlmError::RateLimited { message, details } => {
            ErrorEnvelope::new(ErrorCode::ObEmbeddingFailed, message, details)
        }
        LlmError::ProviderError { message, details } => {
            ErrorEnvelope::new(ErrorCode::ObInternal, message, details)
        }
        LlmError::InvalidModelOutput { message, details } => {
            ErrorEnvelope::new(ErrorCode::ObInternal, message, details)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerank_validation_requires_scope() {
        let err = validate_rerank(&RerankRequest {
            scope: "".to_string(),
            query: "q".to_string(),
            candidates: None,
            top_k: None,
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ObScopeRequired.as_str());
    }

    #[test]
    fn rerank_validation_requires_query() {
        let err = validate_rerank(&RerankRequest {
            scope: "s".to_string(),
            query: "".to_string(),
            candidates: None,
            top_k: None,
        })
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::ObInvalidRequest.as_str());
    }
}
