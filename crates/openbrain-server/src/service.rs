use openbrain_core::{
    ConflictStatus, Envelope, ErrorCode, ErrorEnvelope, LifecycleState, MemoryObjectStored,
};
use openbrain_llm::prompt::{build_pack_prompt, build_rerank_prompt};
use openbrain_llm::{AnthropicClient, LlmError};
use openbrain_store::{GetObjectsRequest, SearchSemanticRequest, SearchStructuredRequest, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

const MAX_CANDIDATES: usize = 100;
const MAX_SNIPPET_LEN: usize = 400;
const DEFAULT_TOP_K: u32 = 20;
const DEFAULT_BUDGET_TOKENS: u32 = 1200;
const MAX_BUDGET_TOKENS: u32 = 8000;
const MAX_VALUE_PREVIEW_CHARS: usize = 320;

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
    pub structured_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_key: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_states: Option<Vec<LifecycleState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_conflicts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_conflicts_detail: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_key_prefixes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<PackPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_summary: Option<bool>,
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
    pub text: String,
    pub items: Vec<MemoryPackItem>,
    #[serde(default)]
    pub conflict_alerts: Vec<MemoryPackConflictAlert>,
    pub budget_requested: u32,
    pub budget_used: u32,
    pub items_selected: u32,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryPackItem {
    pub id: String,
    #[serde(rename = "type")]
    pub object_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_key: Option<String>,
    pub lifecycle_state: LifecycleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub version: i64,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_score: Option<f32>,
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
    pub provenance: Value,
    pub value_preview: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPackConflictAlert {
    pub memory_key: String,
    pub conflicting_count: u32,
    pub conflict_status: ConflictStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by_object_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone)]
struct CandidateMeta {
    semantic_score: Option<f32>,
    conflict: bool,
    conflict_status: ConflictStatus,
    conflict_count: Option<u32>,
    conflicting_object_ids: Option<Vec<String>>,
    resolved_by_object_id: Option<String>,
    resolved_at: Option<String>,
}

#[derive(Debug, Clone)]
struct RankedCandidate {
    object: MemoryObjectStored,
    meta: CandidateMeta,
}

pub fn apply_pack_request_clamps(
    mut req: MemoryPackRequest,
    max_top_k: Option<u32>,
) -> MemoryPackRequest {
    req.top_k = clamp_u32(req.top_k, max_top_k);
    req
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
                    include_states: None,
                    include_expired: None,
                    include_conflicts: None,
                    now: None,
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

    let prompt = build_rerank_prompt(
        &req.query,
        &serde_json::to_string(&candidates).unwrap_or_else(|_| "[]".to_string()),
    );

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

    let policy = req.policy.clone().unwrap_or_default();
    let candidate_limit = req
        .top_k
        .or(policy.max_items)
        .unwrap_or(DEFAULT_TOP_K)
        .clamp(1, MAX_CANDIDATES as u32) as usize;
    let budget_requested = req
        .budget_tokens
        .unwrap_or(DEFAULT_BUDGET_TOKENS)
        .clamp(1, MAX_BUDGET_TOKENS);
    let max_per_key = req.max_per_key.unwrap_or(1).clamp(1, 10) as usize;
    let include_conflicts = req.include_conflicts.unwrap_or(true);
    let include_conflicts_detail = req.include_conflicts_detail.unwrap_or(false);
    let use_semantic = req.semantic.unwrap_or_else(|| {
        req.query
            .as_ref()
            .map(|q| !q.trim().is_empty())
            .unwrap_or(false)
    });

    let mut candidates = match collect_candidates(
        store,
        &req,
        &policy,
        candidate_limit,
        use_semantic,
        include_conflicts,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return Envelope::err(e),
    };
    candidates.sort_by(compare_candidates);

    let mut selected_items = Vec::new();
    let mut seen_by_key: HashMap<String, usize> = HashMap::new();
    let mut alerts_by_key: BTreeMap<String, MemoryPackConflictAlert> = BTreeMap::new();
    let mut text = format!(
        "MEMORY_PACK\nscope={}\nbudget_tokens={}\nselection=deterministic\n\n",
        req.scope, budget_requested
    );
    let mut truncated = false;

    for candidate in candidates {
        if let Some(prefixes) = req.memory_key_prefixes.as_ref() {
            if let Some(key) = candidate.object.memory_key.as_ref() {
                if !prefixes
                    .iter()
                    .any(|p| !p.trim().is_empty() && key.starts_with(p.trim()))
                {
                    continue;
                }
            }
        }

        if let Some(key) = candidate.object.memory_key.as_ref() {
            let count = seen_by_key.get(key).copied().unwrap_or(0);
            if count >= max_per_key {
                continue;
            }
            seen_by_key.insert(key.clone(), count + 1);
        }

        if candidate.meta.conflict || candidate.meta.conflict_status == ConflictStatus::Unresolved {
            if let Some(key) = candidate.object.memory_key.as_ref() {
                alerts_by_key
                    .entry(key.clone())
                    .or_insert_with(|| MemoryPackConflictAlert {
                        memory_key: key.clone(),
                        conflicting_count: candidate.meta.conflict_count.unwrap_or(2),
                        conflict_status: candidate.meta.conflict_status,
                        object_ids: candidate
                            .meta
                            .conflicting_object_ids
                            .clone()
                            .or_else(|| Some(vec![candidate.object.id.clone()])),
                        resolved_by_object_id: candidate.meta.resolved_by_object_id.clone(),
                        resolved_at: candidate.meta.resolved_at.clone(),
                    });
            }
        }

        let item = MemoryPackItem {
            id: candidate.object.id.clone(),
            object_type: candidate.object.object_type.clone(),
            memory_key: candidate.object.memory_key.clone(),
            lifecycle_state: candidate.object.lifecycle_state,
            expires_at: candidate.object.expires_at.clone(),
            version: candidate.object.version,
            updated_at: candidate.object.updated_at.clone(),
            semantic_score: candidate.meta.semantic_score,
            conflict: candidate.meta.conflict,
            conflict_status: candidate.meta.conflict_status,
            conflict_count: candidate.meta.conflict_count,
            conflicting_object_ids: if include_conflicts_detail {
                candidate.meta.conflicting_object_ids.clone()
            } else {
                None
            },
            resolved_by_object_id: candidate.object.resolved_by_object_id.clone(),
            resolved_at: candidate.object.resolved_at.clone(),
            provenance: candidate.object.provenance.clone(),
            value_preview: preview_value(&candidate.object.data),
        };

        let prospective = format!("{}{}", text, format_item_block(&item));
        if estimate_tokens(&prospective) > budget_requested {
            truncated = true;
            break;
        }
        text = prospective;
        selected_items.push(item);
    }

    let conflict_alerts = alerts_by_key.into_values().collect::<Vec<_>>();
    let budget_used = estimate_tokens(&text).min(budget_requested);
    let relevant = selected_items
        .iter()
        .map(|i| i.id.clone())
        .collect::<Vec<_>>();

    let deterministic_summary = format!(
        "Selected {} items within {} token budget.",
        selected_items.len(),
        budget_requested
    );

    let (summary, next_actions) =
        if req.llm_summary.unwrap_or(false) && !req.task_hint.trim().is_empty() {
            match summarize_pack(
                llm,
                &MemoryPack {
                    scope: req.scope.clone(),
                    canonical: Vec::new(),
                    constraints: Vec::new(),
                    relevant: relevant.clone(),
                    conflicts: conflict_alerts
                        .iter()
                        .map(|a| a.memory_key.clone())
                        .collect(),
                    recent: None,
                    summary: String::new(),
                    next_actions: None,
                    text: text.clone(),
                    items: selected_items.clone(),
                    conflict_alerts: conflict_alerts.clone(),
                    budget_requested,
                    budget_used,
                    items_selected: selected_items.len() as u32,
                    truncated,
                },
                &req.task_hint,
            )
            .await
            {
                Ok(v) => v,
                Err(_) => (deterministic_summary.clone(), None),
            }
        } else {
            (deterministic_summary.clone(), None)
        };

    Envelope::ok(MemoryPackResponse {
        pack: MemoryPack {
            scope: req.scope.clone(),
            canonical: selected_items
                .iter()
                .filter(|i| i.object_type == "decision")
                .map(|i| i.id.clone())
                .collect(),
            constraints: if truncated {
                vec!["pack_truncated_to_budget".to_string()]
            } else {
                Vec::new()
            },
            relevant,
            conflicts: conflict_alerts
                .iter()
                .map(|a| a.memory_key.clone())
                .collect(),
            recent: None,
            summary,
            next_actions,
            text,
            items: selected_items.clone(),
            conflict_alerts,
            budget_requested,
            budget_used,
            items_selected: selected_items.len() as u32,
            truncated,
        },
    })
}

async fn collect_candidates<S>(
    store: &S,
    req: &MemoryPackRequest,
    policy: &PackPolicy,
    candidate_limit: usize,
    use_semantic: bool,
    include_conflicts: bool,
) -> Result<Vec<RankedCandidate>, ErrorEnvelope>
where
    S: Store + Clone + 'static,
{
    let mut by_ref: BTreeMap<String, CandidateMeta> = BTreeMap::new();
    let structured_where = req
        .structured_filter
        .clone()
        .or_else(|| where_expr_from_policy(policy));

    let structured = store
        .search_structured(SearchStructuredRequest {
            scope: req.scope.clone(),
            where_expr: structured_where,
            limit: Some(candidate_limit as u32),
            offset: Some(0),
            order_by: None,
            include_states: req.include_states.clone(),
            include_expired: req.include_expired,
            now: req.now.clone(),
            include_conflicts: Some(include_conflicts),
        })
        .await;

    match structured {
        Envelope::Ok { data, .. } => {
            for item in data.results {
                by_ref.entry(item.r#ref.clone()).or_insert(CandidateMeta {
                    semantic_score: None,
                    conflict: item.conflict,
                    conflict_status: item.conflict_status,
                    conflict_count: item.conflict_count,
                    conflicting_object_ids: item.conflicting_object_ids,
                    resolved_by_object_id: item.resolved_by_object_id,
                    resolved_at: item.resolved_at,
                });
            }
        }
        Envelope::Err { error, .. } => return Err(error),
    }

    if use_semantic {
        if let Some(query) = req.query.as_ref() {
            if !query.trim().is_empty() {
                let semantic = store
                    .search_semantic(SearchSemanticRequest {
                        scope: req.scope.clone(),
                        query: query.clone(),
                        top_k: Some(candidate_limit as u32),
                        model: None,
                        embedding_provider: req.embedding_provider.clone(),
                        embedding_model: req.embedding_model.clone(),
                        embedding_kind: req.embedding_kind.clone(),
                        filters: req.structured_filter.clone(),
                        types: policy.include_types.clone(),
                        status: policy.include_status.clone(),
                        include_states: req.include_states.clone(),
                        include_expired: req.include_expired,
                        now: req.now.clone(),
                        include_conflicts: Some(include_conflicts),
                    })
                    .await;
                match semantic {
                    Envelope::Ok { data, .. } => {
                        for item in data.matches {
                            let entry = by_ref.entry(item.r#ref).or_insert(CandidateMeta {
                                semantic_score: Some(item.score),
                                conflict: item.conflict,
                                conflict_status: item.conflict_status,
                                conflict_count: item.conflict_count,
                                conflicting_object_ids: item.conflicting_object_ids.clone(),
                                resolved_by_object_id: item.resolved_by_object_id.clone(),
                                resolved_at: item.resolved_at.clone(),
                            });
                            entry.semantic_score =
                                Some(entry.semantic_score.unwrap_or(item.score).max(item.score));
                        }
                    }
                    Envelope::Err { error, .. } => return Err(error),
                }
            }
        }
    }

    let refs = by_ref.keys().cloned().collect::<Vec<_>>();
    if refs.is_empty() {
        return Ok(Vec::new());
    }

    let read = store
        .get_objects(GetObjectsRequest {
            scope: req.scope.clone(),
            refs,
            include_states: req.include_states.clone(),
            include_expired: req.include_expired,
            now: req.now.clone(),
            include_conflicts: Some(include_conflicts),
        })
        .await;

    let objects = match read {
        Envelope::Ok { data, .. } => data.objects,
        Envelope::Err { error, .. } => return Err(error),
    };

    Ok(objects
        .into_iter()
        .filter_map(|object| {
            by_ref
                .get(&object.id)
                .cloned()
                .map(|meta| RankedCandidate { object, meta })
        })
        .collect())
}

fn compare_candidates(a: &RankedCandidate, b: &RankedCandidate) -> std::cmp::Ordering {
    state_rank(b.object.lifecycle_state)
        .cmp(&state_rank(a.object.lifecycle_state))
        .then_with(|| {
            b.meta
                .semantic_score
                .partial_cmp(&a.meta.semantic_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| b.object.updated_at.cmp(&a.object.updated_at))
        .then_with(|| b.object.version.cmp(&a.object.version))
        .then_with(|| a.object.id.cmp(&b.object.id))
}

fn state_rank(state: LifecycleState) -> u8 {
    match state {
        LifecycleState::Accepted => 4,
        LifecycleState::Candidate => 3,
        LifecycleState::Scratch => 2,
        LifecycleState::Deprecated => 1,
    }
}

fn preview_value(value: &Value) -> Value {
    let raw = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    if raw.len() <= MAX_VALUE_PREVIEW_CHARS {
        value.clone()
    } else {
        Value::String(format!("{}...", &raw[..MAX_VALUE_PREVIEW_CHARS]))
    }
}

fn format_item_block(item: &MemoryPackItem) -> String {
    let mut block = String::new();
    block.push_str("---\n");
    block.push_str(&format!("id: {}\n", item.id));
    block.push_str(&format!("type: {}\n", item.object_type));
    if let Some(key) = item.memory_key.as_ref() {
        block.push_str(&format!("memory_key: {key}\n"));
    }
    block.push_str(&format!(
        "lifecycle_state: {}\n",
        item.lifecycle_state.as_str()
    ));
    block.push_str("value_preview: ");
    block.push_str(
        &serde_json::to_string(&item.value_preview).unwrap_or_else(|_| "null".to_string()),
    );
    block.push('\n');
    block
}

fn where_expr_from_policy(policy: &PackPolicy) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(include_types) = policy.include_types.as_ref() {
        let values = include_types
            .iter()
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!("\"{}\"", v.trim().replace('"', "\\\"")))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            parts.push(format!("type IN [{}]", values.join(", ")));
        }
    }
    if let Some(statuses) = policy.include_status.as_ref() {
        let values = statuses
            .iter()
            .filter(|v| !v.trim().is_empty())
            .map(|v| format!("\"{}\"", v.trim().replace('"', "\\\"")))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            parts.push(format!("status IN [{}]", values.join(", ")));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

fn clamp_u32(requested: Option<u32>, max_value: Option<u32>) -> Option<u32> {
    match (requested, max_value) {
        (Some(v), Some(max_v)) => Some(v.min(max_v)),
        (None, Some(max_v)) => Some(max_v),
        (Some(v), None) => Some(v),
        (None, None) => None,
    }
}

fn estimate_tokens(text: &str) -> u32 {
    (text.chars().count() as u32).div_ceil(4)
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

    let res = store
        .get_objects(GetObjectsRequest {
            scope: scope.to_string(),
            refs,
            include_states: None,
            include_expired: None,
            include_conflicts: None,
            now: None,
        })
        .await;
    let data = match res {
        Envelope::Ok { data, .. } => data,
        Envelope::Err { error, .. } => return Err(error),
    };

    Ok(data
        .objects
        .into_iter()
        .filter(|obj| obj.scope == scope)
        .map(|obj| CandidateDoc {
            r#ref: obj.id,
            kind: obj.object_type,
            snippet: truncate_snippet(&serde_json::to_string(&obj.data).unwrap_or_default()),
        })
        .collect())
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
