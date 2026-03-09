use openbrain_core::{
    ConflictStatus, Envelope, ErrorCode, ErrorEnvelope, LifecycleState, MemoryObjectStored,
};
use openbrain_llm::prompt::{build_pack_prompt, build_rerank_prompt};
use openbrain_llm::{AnthropicClient, LlmError};
use openbrain_store::{GetObjectsRequest, SearchSemanticRequest, SearchStructuredRequest, Store};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_CANDIDATES: usize = 100;
const MAX_SNIPPET_LEN: usize = 400;
const DEFAULT_TOP_K: u32 = 20;
const DEFAULT_BUDGET_TOKENS: u32 = 1200;
const MAX_BUDGET_TOKENS: u32 = 8000;
const MAX_VALUE_PREVIEW_CHARS: usize = 320;
const DEFAULT_MAX_PER_KIND: u32 = 5;
const DEFAULT_MAX_PER_SOURCE: u32 = 5;
const MAX_PER_KIND_CAP: u32 = 20;
const MAX_PER_SOURCE_CAP: u32 = 20;
const MAX_MIN_KIND_COVERAGE: u32 = 10;
const WEIGHT_STATE_ACCEPTED: f64 = 40.0;
const WEIGHT_STATE_CANDIDATE: f64 = 20.0;
const WEIGHT_STATE_SCRATCH: f64 = 10.0;
const WEIGHT_STATE_DEPRECATED: f64 = 0.0;
const WEIGHT_SEMANTIC: f64 = 30.0;
const WEIGHT_VERSION: f64 = 0.02;
const WEIGHT_KEY_PREFIX_MATCH: f64 = 8.0;
const WEIGHT_KIND_FILTER_MATCH: f64 = 6.0;
const PENALTY_CONFLICT_UNRESOLVED: f64 = 8.0;
const PENALTY_CONFLICT_RESOLVED: f64 = 2.0;

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
    pub max_per_kind: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_per_source: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_kind_coverage: Option<u32>,
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
    ranking_score: f64,
    source_tag: String,
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
    let max_per_kind = req
        .max_per_kind
        .unwrap_or(DEFAULT_MAX_PER_KIND)
        .clamp(1, MAX_PER_KIND_CAP) as usize;
    let max_per_source = req
        .max_per_source
        .unwrap_or(DEFAULT_MAX_PER_SOURCE)
        .clamp(1, MAX_PER_SOURCE_CAP) as usize;
    let min_kind_coverage = req
        .min_kind_coverage
        .unwrap_or(0)
        .min(MAX_MIN_KIND_COVERAGE) as usize;
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
    for candidate in &mut candidates {
        candidate.meta.source_tag = provenance_source_tag(&candidate.object.provenance);
        candidate.meta.ranking_score = score_candidate(candidate, &req, &policy);
    }
    candidates.sort_by(compare_candidates);
    let (candidates, relax_stage) = select_candidates_with_diversity(
        candidates,
        candidate_limit,
        max_per_key,
        max_per_kind,
        max_per_source,
        min_kind_coverage,
        req.memory_key_prefixes.as_deref(),
    );

    let mut selected_items = Vec::new();
    let mut alerts_by_key: BTreeMap<String, MemoryPackConflictAlert> = BTreeMap::new();
    let mut text = format!(
        "MEMORY_PACK\nscope={}\nbudget_tokens={}\nselection=deterministic\n\n",
        req.scope, budget_requested
    );
    let mut truncated = false;

    for candidate in candidates {
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
                vec![
                    "pack_truncated_to_budget".to_string(),
                    format!("selection_relax_stage={relax_stage}"),
                ]
            } else {
                vec![format!("selection_relax_stage={relax_stage}")]
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
                    ranking_score: 0.0,
                    source_tag: "unknown".to_string(),
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
                                ranking_score: 0.0,
                                source_tag: "unknown".to_string(),
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
    b.meta
        .ranking_score
        .partial_cmp(&a.meta.ranking_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.object.updated_at.cmp(&a.object.updated_at))
        .then_with(|| b.object.version.cmp(&a.object.version))
        .then_with(|| a.object.id.cmp(&b.object.id))
}

fn state_weight(state: LifecycleState) -> f64 {
    match state {
        LifecycleState::Accepted => WEIGHT_STATE_ACCEPTED,
        LifecycleState::Candidate => WEIGHT_STATE_CANDIDATE,
        LifecycleState::Scratch => WEIGHT_STATE_SCRATCH,
        LifecycleState::Deprecated => WEIGHT_STATE_DEPRECATED,
    }
}

fn score_candidate(
    candidate: &RankedCandidate,
    req: &MemoryPackRequest,
    policy: &PackPolicy,
) -> f64 {
    let mut score = state_weight(candidate.object.lifecycle_state);
    if let Some(semantic_score) = candidate.meta.semantic_score {
        score += (semantic_score as f64) * WEIGHT_SEMANTIC;
    }
    score += (candidate.object.version.max(0) as f64).min(1000.0) * WEIGHT_VERSION;
    score += provenance_weight(&candidate.object.object_type, &candidate.object.provenance);

    if let (Some(prefixes), Some(memory_key)) = (
        req.memory_key_prefixes.as_ref(),
        candidate.object.memory_key.as_ref(),
    ) {
        if prefixes
            .iter()
            .any(|p| !p.trim().is_empty() && memory_key.starts_with(p.trim()))
        {
            score += WEIGHT_KEY_PREFIX_MATCH;
        }
    }

    if let Some(include_types) = policy.include_types.as_ref() {
        if include_types
            .iter()
            .any(|kind| kind.eq_ignore_ascii_case(&candidate.object.object_type))
        {
            score += WEIGHT_KIND_FILTER_MATCH;
        }
    }

    if candidate.meta.conflict || candidate.meta.conflict_status == ConflictStatus::Unresolved {
        score -= PENALTY_CONFLICT_UNRESOLVED;
    } else if candidate.meta.conflict_status == ConflictStatus::Resolved {
        score -= PENALTY_CONFLICT_RESOLVED;
    }

    score
}

fn provenance_source_tag(provenance: &Value) -> String {
    ["source", "source_system", "system", "origin"]
        .iter()
        .find_map(|key| provenance.get(*key).and_then(Value::as_str))
        .map(|v| v.trim().to_lowercase())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn provenance_weight(object_type: &str, provenance: &Value) -> f64 {
    let source = provenance_source_tag(provenance);
    if source.contains("shadow") {
        return -4.0;
    }
    if source.contains("jira")
        || source.contains("confluence")
        || source.contains("github")
        || source.contains("salesforce")
    {
        return 6.0;
    }

    match object_type {
        "policy.rule" | "policy.retention" | "decision" => 5.0,
        "runbook" | "evidence" => 3.0,
        "task" | "preference" => 2.0,
        _ => 1.0,
    }
}

fn key_prefix_allowed(memory_key: Option<&String>, prefixes: Option<&[String]>) -> bool {
    let Some(prefixes) = prefixes else {
        return true;
    };
    if prefixes.is_empty() {
        return true;
    }
    let Some(memory_key) = memory_key else {
        return false;
    };
    prefixes
        .iter()
        .any(|p| !p.trim().is_empty() && memory_key.starts_with(p.trim()))
}

fn select_candidates_with_diversity(
    candidates: Vec<RankedCandidate>,
    candidate_limit: usize,
    max_per_key: usize,
    max_per_kind: usize,
    max_per_source: usize,
    min_kind_coverage: usize,
    memory_key_prefixes: Option<&[String]>,
) -> (Vec<RankedCandidate>, u8) {
    let total_kinds = candidates
        .iter()
        .filter(|c| key_prefix_allowed(c.object.memory_key.as_ref(), memory_key_prefixes))
        .map(|c| c.object.object_type.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let target_coverage = min_kind_coverage.min(total_kinds);

    let stage0 = select_stage(
        &candidates,
        candidate_limit,
        max_per_key,
        max_per_kind,
        Some(max_per_source),
        target_coverage,
        memory_key_prefixes,
    );

    if target_coverage > 0 {
        let selected_kinds = stage0
            .iter()
            .map(|c| c.object.object_type.clone())
            .collect::<BTreeSet<_>>()
            .len();
        if selected_kinds < target_coverage {
            let stage1 = select_stage(
                &candidates,
                candidate_limit,
                max_per_key,
                max_per_kind,
                Some(max_per_source),
                0,
                memory_key_prefixes,
            );
            if stage1.len() < candidate_limit {
                let stage2 = select_stage(
                    &candidates,
                    candidate_limit,
                    max_per_key,
                    max_per_kind,
                    None,
                    0,
                    memory_key_prefixes,
                );
                return (stage2, 2);
            }
            return (stage1, 1);
        }
    }

    if stage0.len() < candidate_limit {
        let stage2 = select_stage(
            &candidates,
            candidate_limit,
            max_per_key,
            max_per_kind,
            None,
            target_coverage,
            memory_key_prefixes,
        );
        return (stage2, 2);
    }

    (stage0, 0)
}

fn select_stage(
    candidates: &[RankedCandidate],
    candidate_limit: usize,
    max_per_key: usize,
    max_per_kind: usize,
    max_per_source: Option<usize>,
    min_kind_coverage: usize,
    memory_key_prefixes: Option<&[String]>,
) -> Vec<RankedCandidate> {
    let mut selected = Vec::new();
    let mut seen_keys: HashMap<String, usize> = HashMap::new();
    let mut kind_counts: HashMap<String, usize> = HashMap::new();
    let mut source_counts: HashMap<String, usize> = HashMap::new();
    let mut covered_kinds: BTreeSet<String> = BTreeSet::new();

    if min_kind_coverage > 0 {
        for candidate in candidates {
            if selected.len() >= candidate_limit || covered_kinds.len() >= min_kind_coverage {
                break;
            }
            if !key_prefix_allowed(candidate.object.memory_key.as_ref(), memory_key_prefixes) {
                continue;
            }
            if covered_kinds.contains(&candidate.object.object_type) {
                continue;
            }
            if !can_select_candidate(
                candidate,
                max_per_key,
                max_per_kind,
                max_per_source,
                &seen_keys,
                &kind_counts,
                &source_counts,
            ) {
                continue;
            }
            apply_candidate_selection(
                candidate,
                &mut selected,
                &mut seen_keys,
                &mut kind_counts,
                &mut source_counts,
                &mut covered_kinds,
            );
        }
    }

    for candidate in candidates {
        if selected.len() >= candidate_limit {
            break;
        }
        if !key_prefix_allowed(candidate.object.memory_key.as_ref(), memory_key_prefixes) {
            continue;
        }
        if selected
            .iter()
            .any(|c: &RankedCandidate| c.object.id == candidate.object.id)
        {
            continue;
        }
        if !can_select_candidate(
            candidate,
            max_per_key,
            max_per_kind,
            max_per_source,
            &seen_keys,
            &kind_counts,
            &source_counts,
        ) {
            continue;
        }
        apply_candidate_selection(
            candidate,
            &mut selected,
            &mut seen_keys,
            &mut kind_counts,
            &mut source_counts,
            &mut covered_kinds,
        );
    }

    selected
}

fn can_select_candidate(
    candidate: &RankedCandidate,
    max_per_key: usize,
    max_per_kind: usize,
    max_per_source: Option<usize>,
    seen_keys: &HashMap<String, usize>,
    kind_counts: &HashMap<String, usize>,
    source_counts: &HashMap<String, usize>,
) -> bool {
    if let Some(memory_key) = candidate.object.memory_key.as_ref() {
        let key_count = seen_keys.get(memory_key).copied().unwrap_or(0);
        if key_count >= max_per_key {
            return false;
        }
    }

    let kind_count = kind_counts
        .get(&candidate.object.object_type)
        .copied()
        .unwrap_or(0);
    if kind_count >= max_per_kind {
        return false;
    }

    if let Some(max_per_source) = max_per_source {
        let source_count = source_counts
            .get(&candidate.meta.source_tag)
            .copied()
            .unwrap_or(0);
        if source_count >= max_per_source {
            return false;
        }
    }

    true
}

fn apply_candidate_selection(
    candidate: &RankedCandidate,
    selected: &mut Vec<RankedCandidate>,
    seen_keys: &mut HashMap<String, usize>,
    kind_counts: &mut HashMap<String, usize>,
    source_counts: &mut HashMap<String, usize>,
    covered_kinds: &mut BTreeSet<String>,
) {
    if let Some(memory_key) = candidate.object.memory_key.as_ref() {
        *seen_keys.entry(memory_key.clone()).or_default() += 1;
    }
    *kind_counts
        .entry(candidate.object.object_type.clone())
        .or_default() += 1;
    *source_counts
        .entry(candidate.meta.source_tag.clone())
        .or_default() += 1;
    covered_kinds.insert(candidate.object.object_type.clone());
    selected.push(candidate.clone());
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
    use serde_json::json;

    #[allow(clippy::too_many_arguments)]
    fn candidate(
        id: &str,
        object_type: &str,
        memory_key: Option<&str>,
        source: &str,
        semantic_score: Option<f32>,
        version: i64,
        lifecycle_state: LifecycleState,
        conflict_status: ConflictStatus,
    ) -> RankedCandidate {
        RankedCandidate {
            object: MemoryObjectStored {
                object_type: object_type.to_string(),
                id: id.to_string(),
                scope: "ws-1".to_string(),
                status: "canonical".to_string(),
                spec_version: "0.1".to_string(),
                tags: vec![],
                data: json!({"value": id}),
                provenance: json!({"source": source}),
                version,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                lifecycle_state,
                expires_at: None,
                memory_key: memory_key.map(str::to_string),
                conflict_status,
                resolved_by_object_id: None,
                resolved_at: None,
                resolution_note: None,
            },
            meta: CandidateMeta {
                semantic_score,
                conflict: conflict_status == ConflictStatus::Unresolved,
                conflict_status,
                conflict_count: None,
                conflicting_object_ids: None,
                resolved_by_object_id: None,
                resolved_at: None,
                ranking_score: 0.0,
                source_tag: source.to_string(),
            },
        }
    }

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

    #[test]
    fn deterministic_sort_prefers_trusted_over_shadow() {
        let req = MemoryPackRequest {
            scope: "ws-1".to_string(),
            task_hint: "t".to_string(),
            query: Some("q".to_string()),
            structured_filter: None,
            semantic: Some(true),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            budget_tokens: Some(1200),
            top_k: Some(10),
            max_per_key: Some(1),
            max_per_kind: Some(5),
            max_per_source: Some(5),
            min_kind_coverage: Some(0),
            include_states: None,
            include_expired: None,
            now: None,
            include_conflicts: Some(true),
            include_conflicts_detail: Some(false),
            memory_key_prefixes: None,
            policy: None,
            llm_summary: Some(false),
        };
        let policy = PackPolicy::default();
        let mut rows = vec![
            candidate(
                "a",
                "decision",
                Some("decision:db"),
                "shadow",
                Some(0.8),
                1,
                LifecycleState::Accepted,
                ConflictStatus::None,
            ),
            candidate(
                "b",
                "decision",
                Some("decision:db"),
                "jira",
                Some(0.7),
                1,
                LifecycleState::Accepted,
                ConflictStatus::None,
            ),
        ];
        for row in &mut rows {
            row.meta.source_tag = provenance_source_tag(&row.object.provenance);
            row.meta.ranking_score = score_candidate(row, &req, &policy);
        }
        rows.sort_by(compare_candidates);
        assert_eq!(rows[0].object.id, "b");
    }

    #[test]
    fn diversity_constraints_enforce_kind_and_source_caps() {
        let rows = vec![
            candidate(
                "a",
                "decision",
                Some("decision:a"),
                "jira",
                Some(0.9),
                1,
                LifecycleState::Accepted,
                ConflictStatus::None,
            ),
            candidate(
                "b",
                "decision",
                Some("decision:b"),
                "jira",
                Some(0.8),
                1,
                LifecycleState::Accepted,
                ConflictStatus::None,
            ),
            candidate(
                "c",
                "task",
                Some("task:a"),
                "confluence",
                Some(0.7),
                1,
                LifecycleState::Accepted,
                ConflictStatus::None,
            ),
        ];
        let (picked, _) = select_candidates_with_diversity(rows, 10, 1, 1, 1, 0, None);
        assert_eq!(picked.len(), 2);
        assert!(picked.iter().any(|r| r.object.object_type == "decision"));
        assert!(picked.iter().any(|r| r.object.object_type == "task"));
    }

    #[test]
    fn unresolved_conflict_penalty_lowers_rank() {
        let req = MemoryPackRequest {
            scope: "ws-1".to_string(),
            task_hint: "t".to_string(),
            query: None,
            structured_filter: None,
            semantic: Some(false),
            embedding_provider: None,
            embedding_model: None,
            embedding_kind: None,
            budget_tokens: Some(1200),
            top_k: Some(10),
            max_per_key: Some(1),
            max_per_kind: Some(5),
            max_per_source: Some(5),
            min_kind_coverage: Some(0),
            include_states: None,
            include_expired: None,
            now: None,
            include_conflicts: Some(true),
            include_conflicts_detail: Some(false),
            memory_key_prefixes: None,
            policy: None,
            llm_summary: Some(false),
        };
        let policy = PackPolicy::default();
        let clean = candidate(
            "clean",
            "claim",
            Some("k:1"),
            "notes",
            None,
            10,
            LifecycleState::Accepted,
            ConflictStatus::None,
        );
        let conflicted = candidate(
            "conflict",
            "claim",
            Some("k:2"),
            "notes",
            None,
            10,
            LifecycleState::Accepted,
            ConflictStatus::Unresolved,
        );
        let clean_score = score_candidate(&clean, &req, &policy);
        let conflict_score = score_candidate(&conflicted, &req, &policy);
        assert!(clean_score > conflict_score);
    }

    #[test]
    fn memory_pack_request_is_backward_compatible_without_new_fields() {
        let req: MemoryPackRequest = serde_json::from_value(json!({
            "scope":"ws-1",
            "task_hint":"summarize"
        }))
        .expect("deserialize request");
        assert_eq!(req.max_per_kind, None);
        assert_eq!(req.max_per_source, None);
        assert_eq!(req.min_kind_coverage, None);
    }
}
