use openbrain_core::{Envelope, ErrorCode, ErrorEnvelope, LifecycleState, MemoryObject};
use openbrain_store::{GetObjectsRequest, SearchStructuredRequest, Store, WorkspaceRole};
use serde::Deserialize;
use serde_json::Value;

const POLICY_KIND: &str = "policy.rule";
const RETENTION_KIND: &str = "policy.retention";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyOperation {
    Read,
    Write,
    SearchStructured,
    SearchSemantic,
    EmbedGenerate,
    Admin,
    AuditObjectTimeline,
    AuditMemoryKeyTimeline,
    AuditActorActivity,
    WorkspaceInfo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleEffect {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
pub struct PolicyRule {
    pub id: String,
    pub priority: i64,
    pub created_at: String,
    effect: RuleEffect,
    operations: Vec<PolicyOperation>,
    roles: Option<Vec<WorkspaceRole>>,
    identities: Option<Vec<String>>,
    object_kinds: Option<Vec<String>>,
    memory_key_prefixes: Option<Vec<String>>,
    lifecycle_transitions: Option<Vec<String>>,
    max_top_k: Option<u32>,
    max_write_bytes: Option<u64>,
    reason: String,
}

#[derive(Debug, Clone)]
pub struct EvalInput<'a> {
    pub identity_id: &'a str,
    pub role: WorkspaceRole,
    pub operation: PolicyOperation,
    pub object_kind: Option<&'a str>,
    pub memory_key: Option<&'a str>,
    pub lifecycle_transition: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct EvalDecision {
    pub allowed: bool,
    pub reason_code: Option<String>,
    pub policy_rule_id: Option<String>,
    pub max_top_k: Option<u32>,
    pub max_write_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct RetentionPolicy {
    pub policy_object_id: String,
    pub default_ttl_by_kind: std::collections::HashMap<String, i64>,
    pub max_ttl_by_kind: std::collections::HashMap<String, i64>,
    pub immutable_kinds: std::collections::HashSet<String>,
}

#[derive(Debug, Deserialize)]
struct RetentionData {
    #[serde(default)]
    default_ttl_by_kind: std::collections::HashMap<String, i64>,
    #[serde(default)]
    max_ttl_by_kind: std::collections::HashMap<String, i64>,
    #[serde(default)]
    immutable_kinds: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RuleData {
    #[serde(default)]
    id: Option<String>,
    effect: String,
    operations: Vec<String>,
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    identities: Option<Vec<String>>,
    #[serde(default)]
    object_kinds: Option<Vec<String>>,
    #[serde(default)]
    memory_key_prefixes: Option<Vec<String>>,
    #[serde(default)]
    lifecycle_transitions: Option<Vec<String>>,
    #[serde(default)]
    constraints: Option<RuleConstraints>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct RuleConstraints {
    #[serde(default)]
    max_top_k: Option<u32>,
    #[serde(default)]
    max_write_bytes: Option<u64>,
}

fn parse_operation(value: &str) -> Result<PolicyOperation, ErrorEnvelope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "read" => Ok(PolicyOperation::Read),
        "write" => Ok(PolicyOperation::Write),
        "search_structured" => Ok(PolicyOperation::SearchStructured),
        "search_semantic" => Ok(PolicyOperation::SearchSemantic),
        "embed_generate" => Ok(PolicyOperation::EmbedGenerate),
        "admin" => Ok(PolicyOperation::Admin),
        "audit_object_timeline" => Ok(PolicyOperation::AuditObjectTimeline),
        "audit_memory_key_timeline" => Ok(PolicyOperation::AuditMemoryKeyTimeline),
        "audit_actor_activity" => Ok(PolicyOperation::AuditActorActivity),
        "workspace_info" => Ok(PolicyOperation::WorkspaceInfo),
        _ => Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "invalid policy operation",
            Some(serde_json::json!({"operation": value})),
        )),
    }
}

fn parse_role(value: &str) -> Result<WorkspaceRole, ErrorEnvelope> {
    use std::str::FromStr;
    WorkspaceRole::from_str(value).map_err(|_| {
        ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "invalid policy role",
            Some(serde_json::json!({"role": value})),
        )
    })
}

fn parse_effect(value: &str) -> Result<RuleEffect, ErrorEnvelope> {
    match value.trim().to_ascii_lowercase().as_str() {
        "allow" => Ok(RuleEffect::Allow),
        "deny" => Ok(RuleEffect::Deny),
        _ => Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "invalid policy effect",
            Some(serde_json::json!({"effect": value})),
        )),
    }
}

fn parse_rule(
    object_id: &str,
    created_at: &str,
    payload: &Value,
) -> Result<PolicyRule, ErrorEnvelope> {
    let data: RuleData = serde_json::from_value(payload.clone()).map_err(|e| {
        ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "invalid policy.rule data",
            Some(serde_json::json!({"object_id": object_id, "error": e.to_string()})),
        )
    })?;

    if data.operations.is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "policy.rule operations cannot be empty",
            Some(serde_json::json!({"object_id": object_id})),
        ));
    }

    let operations = data
        .operations
        .iter()
        .map(|s| parse_operation(s))
        .collect::<Result<Vec<_>, _>>()?;
    let roles = data
        .roles
        .as_ref()
        .map(|v| {
            v.iter()
                .map(|s| parse_role(s))
                .collect::<Result<Vec<_>, ErrorEnvelope>>()
        })
        .transpose()?;

    Ok(PolicyRule {
        id: data
            .id
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or(object_id)
            .to_string(),
        priority: data.priority.unwrap_or(1000),
        created_at: created_at.to_string(),
        effect: parse_effect(&data.effect)?,
        operations,
        roles,
        identities: data.identities,
        object_kinds: data.object_kinds,
        memory_key_prefixes: data.memory_key_prefixes,
        lifecycle_transitions: data.lifecycle_transitions,
        max_top_k: data.constraints.as_ref().and_then(|c| c.max_top_k),
        max_write_bytes: data.constraints.as_ref().and_then(|c| c.max_write_bytes),
        reason: data
            .reason
            .as_ref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .unwrap_or("OB_POLICY_DENY")
            .to_string(),
    })
}

pub async fn load_workspace_policies<S>(
    store: &S,
    workspace_id: &str,
) -> Result<Vec<PolicyRule>, ErrorEnvelope>
where
    S: Store + ?Sized,
{
    let search = store
        .search_structured(SearchStructuredRequest {
            scope: workspace_id.to_string(),
            where_expr: Some("type == \"policy.rule\"".to_string()),
            limit: Some(200),
            offset: Some(0),
            order_by: None,
            include_states: Some(vec![LifecycleState::Accepted]),
            include_expired: Some(false),
            now: None,
            include_conflicts: Some(false),
        })
        .await;

    let refs: Vec<String> = match search {
        Envelope::Ok { data, .. } => data.results.into_iter().map(|r| r.r#ref).collect(),
        Envelope::Err { error, .. } => return Err(error),
    };

    if refs.is_empty() {
        return Ok(Vec::new());
    }

    let read = store
        .get_objects(GetObjectsRequest {
            scope: workspace_id.to_string(),
            refs,
            include_states: Some(vec![LifecycleState::Accepted]),
            include_expired: Some(false),
            now: None,
            include_conflicts: Some(false),
        })
        .await;

    let mut rules = match read {
        Envelope::Ok { data, .. } => data
            .objects
            .into_iter()
            .filter(|o| o.object_type == POLICY_KIND)
            .map(|o| parse_rule(&o.id, &o.created_at, &o.data))
            .collect::<Result<Vec<_>, _>>()?,
        Envelope::Err { error, .. } => return Err(error),
    };

    rules.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(rules)
}

fn role_is_owner(role: WorkspaceRole) -> bool {
    matches!(role, WorkspaceRole::Owner)
}

fn kind_is_protected(kind: Option<&str>) -> bool {
    kind.map(|v| {
        v.eq_ignore_ascii_case(POLICY_KIND)
            || v.eq_ignore_ascii_case(RETENTION_KIND)
            || v.to_ascii_lowercase().starts_with("policy.")
    })
    .unwrap_or(false)
}

fn operation_matches(rule: &PolicyRule, op: PolicyOperation) -> bool {
    rule.operations.contains(&op)
}

fn role_matches(rule: &PolicyRule, role: WorkspaceRole) -> bool {
    match &rule.roles {
        None => true,
        Some(v) => v.contains(&role),
    }
}

fn identity_matches(rule: &PolicyRule, identity_id: &str) -> bool {
    match &rule.identities {
        None => true,
        Some(v) => v.iter().any(|x| x == identity_id),
    }
}

fn kind_matches(rule: &PolicyRule, object_kind: Option<&str>) -> bool {
    match &rule.object_kinds {
        None => true,
        Some(v) => object_kind
            .map(|k| v.iter().any(|x| x == k))
            .unwrap_or(false),
    }
}

fn memory_key_matches(rule: &PolicyRule, memory_key: Option<&str>) -> bool {
    match &rule.memory_key_prefixes {
        None => true,
        Some(v) => memory_key
            .map(|k| v.iter().any(|p| k.starts_with(p)))
            .unwrap_or(false),
    }
}

fn transition_matches(rule: &PolicyRule, transition: Option<&str>) -> bool {
    match &rule.lifecycle_transitions {
        None => true,
        Some(v) => transition
            .map(|t| v.iter().any(|x| x == t))
            .unwrap_or(false),
    }
}

pub fn evaluate(rules: &[PolicyRule], input: &EvalInput<'_>) -> EvalDecision {
    if kind_is_protected(input.object_kind)
        && !role_is_owner(input.role)
        && matches!(
            input.operation,
            PolicyOperation::Read
                | PolicyOperation::Write
                | PolicyOperation::SearchStructured
                | PolicyOperation::SearchSemantic
        )
    {
        return EvalDecision {
            allowed: false,
            reason_code: Some("OB_POLICY_DENY_PROTECTED_KIND".to_string()),
            policy_rule_id: None,
            max_top_k: None,
            max_write_bytes: None,
        };
    }

    let mut max_top_k: Option<u32> = None;
    let mut max_write_bytes: Option<u64> = None;

    for rule in rules {
        if !operation_matches(rule, input.operation) {
            continue;
        }
        if !role_matches(rule, input.role) {
            continue;
        }
        if !identity_matches(rule, input.identity_id) {
            continue;
        }
        if !kind_matches(rule, input.object_kind) {
            continue;
        }
        if !memory_key_matches(rule, input.memory_key) {
            continue;
        }
        if !transition_matches(rule, input.lifecycle_transition) {
            continue;
        }

        if let Some(v) = rule.max_top_k {
            max_top_k = Some(max_top_k.map(|cur| cur.min(v)).unwrap_or(v));
        }
        if let Some(v) = rule.max_write_bytes {
            max_write_bytes = Some(max_write_bytes.map(|cur| cur.min(v)).unwrap_or(v));
        }

        if matches!(rule.effect, RuleEffect::Deny) {
            return EvalDecision {
                allowed: false,
                reason_code: Some(rule.reason.clone()),
                policy_rule_id: Some(rule.id.clone()),
                max_top_k,
                max_write_bytes,
            };
        }
    }

    EvalDecision {
        allowed: true,
        reason_code: None,
        policy_rule_id: None,
        max_top_k,
        max_write_bytes,
    }
}

pub fn validate_policy_write_permissions(
    role: WorkspaceRole,
    objects: &[MemoryObject],
) -> Result<(), ErrorEnvelope> {
    if role_is_owner(role) {
        return Ok(());
    }

    let has_policy_obj = objects.iter().any(|o| {
        o.object_type
            .as_ref()
            .map(|t| t.to_ascii_lowercase().starts_with("policy."))
            .unwrap_or(false)
    });
    if has_policy_obj {
        return Err(deny_error(
            "OB_POLICY_DENY_PROTECTED_KIND",
            Some(serde_json::json!({"kind": POLICY_KIND})),
        ));
    }
    Ok(())
}

pub fn lifecycle_transition(
    previous: Option<LifecycleState>,
    next: LifecycleState,
) -> Option<String> {
    match previous {
        Some(prev) if prev != next => Some(format!("{}->{}", prev.as_str(), next.as_str())),
        _ => None,
    }
}

pub fn deny_error(reason: &str, details: Option<Value>) -> ErrorEnvelope {
    deny_error_with_rule(reason, None, details)
}

pub fn deny_error_with_rule(
    reason_code: &str,
    policy_rule_id: Option<&str>,
    details: Option<Value>,
) -> ErrorEnvelope {
    ErrorEnvelope::new(
        ErrorCode::ObForbidden,
        "policy denied",
        Some(serde_json::json!({
            "reason_code": reason_code,
            "policy_rule_id": policy_rule_id,
            "details": details.unwrap_or(Value::Null)
        })),
    )
}

pub fn clamp_u32(requested: Option<u32>, max_value: Option<u32>) -> Option<u32> {
    match (requested, max_value) {
        (Some(v), Some(max_v)) => Some(v.min(max_v)),
        (None, Some(max_v)) => Some(max_v),
        (Some(v), None) => Some(v),
        (None, None) => None,
    }
}

fn normalize_kind_map(
    src: std::collections::HashMap<String, i64>,
) -> std::collections::HashMap<String, i64> {
    let mut out = std::collections::HashMap::new();
    for (k, v) in src {
        let key = k.trim().to_ascii_lowercase();
        if !key.is_empty() && v >= 0 {
            out.insert(key, v);
        }
    }
    out
}

pub async fn load_workspace_retention_policy<S>(
    store: &S,
    workspace_id: &str,
) -> Result<Option<RetentionPolicy>, ErrorEnvelope>
where
    S: Store + ?Sized,
{
    let search = store
        .search_structured(SearchStructuredRequest {
            scope: workspace_id.to_string(),
            where_expr: Some("type == \"policy.retention\"".to_string()),
            limit: Some(1),
            offset: Some(0),
            order_by: Some(openbrain_store::OrderBySpec {
                field: "updated_at".to_string(),
                direction: openbrain_store::OrderDirection::Desc,
            }),
            include_states: Some(vec![LifecycleState::Accepted]),
            include_expired: Some(false),
            now: None,
            include_conflicts: Some(false),
        })
        .await;
    let object_id = match search {
        Envelope::Ok { data, .. } => data.results.into_iter().next().map(|r| r.r#ref),
        Envelope::Err { error, .. } => return Err(error),
    };
    let Some(object_id) = object_id else {
        return Ok(None);
    };

    let read = store
        .get_objects(GetObjectsRequest {
            scope: workspace_id.to_string(),
            refs: vec![object_id.clone()],
            include_states: Some(vec![LifecycleState::Accepted]),
            include_expired: Some(false),
            now: None,
            include_conflicts: Some(false),
        })
        .await;
    let obj = match read {
        Envelope::Ok { data, .. } => data.objects.into_iter().next(),
        Envelope::Err { error, .. } => return Err(error),
    };
    let Some(obj) = obj else {
        return Ok(None);
    };
    if !obj.object_type.eq_ignore_ascii_case(RETENTION_KIND) {
        return Ok(None);
    }
    let parsed: RetentionData = serde_json::from_value(obj.data).map_err(|e| {
        ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "invalid policy.retention data",
            Some(serde_json::json!({"object_id": object_id, "error": e.to_string()})),
        )
    })?;
    let immutable_kinds = parsed
        .immutable_kinds
        .into_iter()
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty())
        .collect::<std::collections::HashSet<_>>();

    Ok(Some(RetentionPolicy {
        policy_object_id: object_id,
        default_ttl_by_kind: normalize_kind_map(parsed.default_ttl_by_kind),
        max_ttl_by_kind: normalize_kind_map(parsed.max_ttl_by_kind),
        immutable_kinds,
    }))
}

fn parse_rfc3339_utc(input: &str) -> Result<chrono::DateTime<chrono::Utc>, ErrorEnvelope> {
    chrono::DateTime::parse_from_rfc3339(input)
        .map(|v| v.with_timezone(&chrono::Utc))
        .map_err(|_| {
            ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "invalid expires_at timestamp",
                Some(serde_json::json!({"value": input})),
            )
        })
}

pub fn apply_retention_policy_to_objects(
    policy: Option<&RetentionPolicy>,
    objects: &mut [MemoryObject],
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), ErrorEnvelope> {
    for obj in objects {
        let state = obj.lifecycle_state.unwrap_or(LifecycleState::Accepted);
        let kind = obj
            .object_type
            .as_deref()
            .map(|v| v.trim().to_ascii_lowercase())
            .unwrap_or_default();

        let fallback_days = match state {
            LifecycleState::Scratch => Some(7_i64),
            LifecycleState::Candidate => Some(30_i64),
            LifecycleState::Accepted | LifecycleState::Deprecated => None,
        };

        let policy_default = policy.and_then(|p| p.default_ttl_by_kind.get(&kind).copied());
        let max_days = policy.and_then(|p| p.max_ttl_by_kind.get(&kind).copied());
        let immutable = policy
            .map(|p| p.immutable_kinds.contains(&kind))
            .unwrap_or(false);
        let has_explicit = obj
            .expires_at
            .as_deref()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false);

        if has_explicit {
            let mut expires = parse_rfc3339_utc(obj.expires_at.as_deref().unwrap_or_default())?;
            if let Some(max) = max_days {
                let limit = now + chrono::Duration::days(max);
                if expires > limit {
                    if immutable {
                        return Err(deny_error_with_rule(
                            "OB_POLICY_DENY_RETENTION_IMMUTABLE_KIND",
                            policy.map(|p| p.policy_object_id.as_str()),
                            Some(serde_json::json!({"kind": kind, "max_ttl_days": max})),
                        ));
                    }
                    expires = limit;
                }
            }
            obj.expires_at = Some(expires.to_rfc3339());
            continue;
        }

        let effective_days = policy_default.or(fallback_days);
        if let Some(days) = effective_days {
            let mut expires = now + chrono::Duration::days(days);
            if let Some(max) = max_days {
                let limit = now + chrono::Duration::days(max);
                if expires > limit {
                    expires = limit;
                }
            }
            obj.expires_at = Some(expires.to_rfc3339());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_rule(
        id: &str,
        priority: i64,
        effect: RuleEffect,
        op: PolicyOperation,
        reason: &str,
    ) -> PolicyRule {
        PolicyRule {
            id: id.to_string(),
            priority,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            effect,
            operations: vec![op],
            roles: Some(vec![WorkspaceRole::Reader]),
            identities: None,
            object_kinds: Some(vec!["decision".to_string()]),
            memory_key_prefixes: None,
            lifecycle_transitions: None,
            max_top_k: None,
            max_write_bytes: None,
            reason: reason.to_string(),
        }
    }

    #[test]
    fn deny_overrides_allow() {
        let rules = vec![
            mk_rule(
                "allow",
                10,
                RuleEffect::Allow,
                PolicyOperation::Read,
                "ALLOW",
            ),
            mk_rule(
                "deny",
                20,
                RuleEffect::Deny,
                PolicyOperation::Read,
                "OB_POLICY_DENY_DECISION",
            ),
        ];

        let decision = evaluate(
            &rules,
            &EvalInput {
                identity_id: "id1",
                role: WorkspaceRole::Reader,
                operation: PolicyOperation::Read,
                object_kind: Some("decision"),
                memory_key: None,
                lifecycle_transition: None,
            },
        );
        assert!(!decision.allowed);
        assert_eq!(
            decision.reason_code.as_deref(),
            Some("OB_POLICY_DENY_DECISION")
        );
        assert_eq!(decision.policy_rule_id.as_deref(), Some("deny"));
    }

    #[test]
    fn protected_policy_kind_is_owner_only() {
        let decision = evaluate(
            &[],
            &EvalInput {
                identity_id: "id1",
                role: WorkspaceRole::Reader,
                operation: PolicyOperation::Read,
                object_kind: Some("policy.rule"),
                memory_key: None,
                lifecycle_transition: None,
            },
        );
        assert!(!decision.allowed);
        assert_eq!(
            decision.reason_code.as_deref(),
            Some("OB_POLICY_DENY_PROTECTED_KIND")
        );
    }

    #[test]
    fn clamps_top_k_from_matching_rules() {
        let mut r1 = mk_rule(
            "r1",
            1,
            RuleEffect::Allow,
            PolicyOperation::SearchSemantic,
            "ok",
        );
        r1.object_kinds = None;
        r1.roles = None;
        r1.max_top_k = Some(25);
        let mut r2 = mk_rule(
            "r2",
            2,
            RuleEffect::Allow,
            PolicyOperation::SearchSemantic,
            "ok",
        );
        r2.object_kinds = None;
        r2.roles = None;
        r2.max_top_k = Some(10);
        let decision = evaluate(
            &[r1, r2],
            &EvalInput {
                identity_id: "id1",
                role: WorkspaceRole::Writer,
                operation: PolicyOperation::SearchSemantic,
                object_kind: None,
                memory_key: None,
                lifecycle_transition: None,
            },
        );
        assert!(decision.allowed);
        assert_eq!(decision.max_top_k, Some(10));
    }
}
