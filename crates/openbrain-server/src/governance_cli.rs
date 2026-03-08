use async_trait::async_trait;
use chrono::DateTime;
use openbrain_core::{Envelope, ErrorEnvelope};
use openbrain_store::{
    AuditActorActivityRequest, AuditMemoryKeyTimelineRequest, AuditObjectTimelineRequest,
    AuditRequest, AuditResponse, GetObjectsRequest, GetObjectsResponse, OrderBySpec,
    OrderDirection, SearchStructuredRequest, SearchStructuredResponse, WorkspaceInfoResponse,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::fmt::Write as _;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

#[derive(Debug, Clone)]
pub struct HttpArgs {
    pub token: String,
    pub scope: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug)]
pub enum CliError {
    Usage(String),
    Transport(String),
    Api(ErrorEnvelope),
    Decode(String),
}

impl CliError {
    pub fn user_message(&self) -> String {
        match self {
            Self::Usage(m) => m.clone(),
            Self::Transport(m) => format!("request failed: {m}"),
            Self::Decode(m) => format!("failed to decode server response: {m}"),
            Self::Api(err) => {
                if err.code == "OB_FORBIDDEN" {
                    let reason = err
                        .details
                        .as_ref()
                        .and_then(|d| d.get("reason_code"))
                        .and_then(Value::as_str)
                        .unwrap_or("OB_POLICY_DENY");
                    let rule = err
                        .details
                        .as_ref()
                        .and_then(|d| d.get("policy_rule_id"))
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    return format!("DENIED: {reason} (rule: {rule})");
                }
                format!("{}: {}", err.code, err.message)
            }
        }
    }
}

#[async_trait]
pub trait GovernanceTransport {
    async fn post_json(&self, path: &str, token: &str, body: Value) -> Result<Value, CliError>;
}

pub struct ReqwestTransport {
    base_url: String,
    client: reqwest::Client,
}

impl ReqwestTransport {
    pub fn new(base_url: String) -> Result<Self, CliError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| CliError::Transport(e.to_string()))?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }
}

#[async_trait]
impl GovernanceTransport for ReqwestTransport {
    async fn post_json(&self, path: &str, token: &str, body: Value) -> Result<Value, CliError> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await
            .map_err(|e| CliError::Transport(e.to_string()))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| CliError::Transport(e.to_string()))?;
        let parsed: Value = serde_json::from_str(&text).map_err(|_| {
            CliError::Transport(format!("http {} returned non-json body", status.as_u16()))
        })?;
        Ok(parsed)
    }
}

fn decode_envelope<T: DeserializeOwned>(value: Value) -> Result<T, CliError> {
    let env: Envelope<T> =
        serde_json::from_value(value).map_err(|e| CliError::Decode(e.to_string()))?;
    match env {
        Envelope::Ok { data, .. } => Ok(data),
        Envelope::Err { error, .. } => Err(CliError::Api(error)),
    }
}

fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT)
}

fn validate_rfc3339(label: &str, raw: &Option<String>) -> Result<(), CliError> {
    if let Some(ts) = raw {
        DateTime::parse_from_rfc3339(ts)
            .map_err(|_| CliError::Usage(format!("invalid {label} timestamp: {ts}")))?;
    }
    Ok(())
}

fn require_scope(scope: &Option<String>) -> Result<String, CliError> {
    scope
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CliError::Usage("--scope is required".to_string()))
}

fn format_workspace_info(info: WorkspaceInfoResponse) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "workspace_id: {}", info.workspace_id);
    let _ = writeln!(out, "owner_identity_id: {}", info.owner_identity_id);
    if let Some(name) = info.owner_display_name {
        let _ = writeln!(out, "owner_display_name: {}", name);
    }
    let _ = writeln!(out, "caller_identity_id: {}", info.caller_identity_id);
    let _ = writeln!(out, "caller_role: {}", info.caller_role.as_str());
    out
}

fn format_audit_table(resp: AuditResponse) -> String {
    let mut events = resp.events;
    events.sort_by(|a, b| b.ts.cmp(&a.ts));

    let mut out = String::new();
    let _ = writeln!(
        out,
        "timestamp | event_type | actor_id | object_id | version | summary"
    );
    for ev in events {
        let object_id = ev.object_id.unwrap_or_else(|| "-".to_string());
        let version = ev
            .object_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        let summary = ev
            .summary
            .unwrap_or_else(|| "-".to_string())
            .replace('\n', " ");
        let _ = writeln!(
            out,
            "{} | {} | {} | {} | {} | {}",
            ev.ts, ev.event_type, ev.actor_identity_id, object_id, version, summary
        );
    }
    let _ = writeln!(out, "limit={} offset={}", resp.limit, resp.offset);
    out
}

pub async fn run_workspace_info<T: GovernanceTransport + Sync>(
    transport: &T,
    args: &HttpArgs,
) -> Result<String, CliError> {
    let value = transport
        .post_json("/v1/workspace/info", &args.token, json!({}))
        .await?;
    let data: WorkspaceInfoResponse = decode_envelope(value)?;
    Ok(format_workspace_info(data))
}

pub async fn run_audit_object<T: GovernanceTransport + Sync>(
    transport: &T,
    args: &HttpArgs,
    object_id: String,
) -> Result<String, CliError> {
    validate_rfc3339("--from", &args.from)?;
    validate_rfc3339("--to", &args.to)?;
    let req = AuditObjectTimelineRequest {
        query: AuditRequest {
            scope: require_scope(&args.scope)?,
            from: args.from.clone(),
            to: args.to.clone(),
            limit: Some(clamp_limit(args.limit)),
            offset: None,
        },
        object_id,
    };
    let body = serde_json::to_value(req).map_err(|e| CliError::Decode(e.to_string()))?;
    let value = transport
        .post_json("/v1/audit/object_timeline", &args.token, body)
        .await?;
    let data: AuditResponse = decode_envelope(value)?;
    Ok(format_audit_table(data))
}

pub async fn run_audit_key<T: GovernanceTransport + Sync>(
    transport: &T,
    args: &HttpArgs,
    memory_key: String,
) -> Result<String, CliError> {
    validate_rfc3339("--from", &args.from)?;
    validate_rfc3339("--to", &args.to)?;
    let req = AuditMemoryKeyTimelineRequest {
        query: AuditRequest {
            scope: require_scope(&args.scope)?,
            from: args.from.clone(),
            to: args.to.clone(),
            limit: Some(clamp_limit(args.limit)),
            offset: None,
        },
        memory_key,
    };
    let body = serde_json::to_value(req).map_err(|e| CliError::Decode(e.to_string()))?;
    let value = transport
        .post_json("/v1/audit/memory_key_timeline", &args.token, body)
        .await?;
    let data: AuditResponse = decode_envelope(value)?;
    Ok(format_audit_table(data))
}

pub async fn run_audit_actor<T: GovernanceTransport + Sync>(
    transport: &T,
    args: &HttpArgs,
    actor_identity_id: String,
) -> Result<String, CliError> {
    validate_rfc3339("--from", &args.from)?;
    validate_rfc3339("--to", &args.to)?;
    let req = AuditActorActivityRequest {
        query: AuditRequest {
            scope: require_scope(&args.scope)?,
            from: args.from.clone(),
            to: args.to.clone(),
            limit: Some(clamp_limit(args.limit)),
            offset: None,
        },
        actor_identity_id,
    };
    let body = serde_json::to_value(req).map_err(|e| CliError::Decode(e.to_string()))?;
    let value = transport
        .post_json("/v1/audit/actor_activity", &args.token, body)
        .await?;
    let data: AuditResponse = decode_envelope(value)?;
    Ok(format_audit_table(data))
}

pub async fn run_retention_show<T: GovernanceTransport + Sync>(
    transport: &T,
    args: &HttpArgs,
) -> Result<String, CliError> {
    let scope = require_scope(&args.scope)?;
    let search_req = SearchStructuredRequest {
        scope: scope.clone(),
        where_expr: Some("type == \"policy.retention\"".to_string()),
        limit: Some(1),
        offset: Some(0),
        order_by: Some(OrderBySpec {
            field: "updated_at".to_string(),
            direction: OrderDirection::Desc,
        }),
        include_states: None,
        include_expired: None,
        now: None,
        include_conflicts: None,
    };
    let search_body =
        serde_json::to_value(search_req).map_err(|e| CliError::Decode(e.to_string()))?;
    let search_value = transport
        .post_json("/v1/search/structured", &args.token, search_body)
        .await?;
    let search: SearchStructuredResponse = decode_envelope(search_value)?;

    let Some(item) = search.results.into_iter().next() else {
        return Ok("No active policy.retention object found for this scope.".to_string());
    };

    let read_req = GetObjectsRequest {
        scope,
        refs: vec![item.r#ref.clone()],
        include_states: None,
        include_expired: None,
        now: None,
        include_conflicts: None,
    };
    let read_body = serde_json::to_value(read_req).map_err(|e| CliError::Decode(e.to_string()))?;
    let read_value = transport
        .post_json("/v1/read", &args.token, read_body)
        .await?;
    let read: GetObjectsResponse = decode_envelope(read_value)?;
    let Some(obj) = read.objects.into_iter().next() else {
        return Ok("No active policy.retention object found for this scope.".to_string());
    };

    let defaults = obj
        .data
        .get("default_ttl_by_kind")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let max = obj
        .data
        .get("max_ttl_by_kind")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let immutable = obj
        .data
        .get("immutable_kinds")
        .cloned()
        .unwrap_or_else(|| json!([]));

    let mut out = String::new();
    let _ = writeln!(out, "retention_ref: {}", item.r#ref);
    let _ = writeln!(out, "retention_version: {}", obj.version);
    let _ = writeln!(out, "default_ttl_by_kind: {}", defaults);
    let _ = writeln!(out, "max_ttl_by_kind: {}", max);
    let _ = writeln!(out, "immutable_kinds: {}", immutable);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::Mutex;

    struct FakeTransport {
        payloads: Mutex<HashMap<String, Value>>,
    }

    impl FakeTransport {
        fn new(payloads: HashMap<String, Value>) -> Self {
            Self {
                payloads: Mutex::new(payloads),
            }
        }
    }

    #[async_trait]
    impl GovernanceTransport for FakeTransport {
        async fn post_json(
            &self,
            path: &str,
            _token: &str,
            _body: Value,
        ) -> Result<Value, CliError> {
            let mut map = self.payloads.lock().await;
            map.remove(path)
                .ok_or_else(|| CliError::Transport(format!("missing fake payload for {path}")))
        }
    }

    fn args_with_scope() -> HttpArgs {
        HttpArgs {
            token: "token".to_string(),
            scope: Some("ws-default".to_string()),
            from: None,
            to: None,
            limit: Some(500),
        }
    }

    #[tokio::test]
    async fn audit_output_is_rendered_and_sorted_desc() {
        let mut payloads = HashMap::new();
        payloads.insert(
            "/v1/audit/object_timeline".to_string(),
            json!({
                "ok": true,
                "events": [
                    {"id": 1, "event_type": "write", "actor_identity_id": "a", "object_id": "o1", "object_version": 1, "memory_key": null, "ts": "2026-01-01T00:00:00Z", "summary": "older"},
                    {"id": 2, "event_type": "promote", "actor_identity_id": "a", "object_id": "o1", "object_version": 2, "memory_key": null, "ts": "2026-01-02T00:00:00Z", "summary": "newer"}
                ],
                "limit": 200,
                "offset": 0
            }),
        );
        let fake = FakeTransport::new(payloads);
        let out = run_audit_object(&fake, &args_with_scope(), "o1".to_string())
            .await
            .expect("audit rendered");

        assert!(out.contains("limit=200 offset=0"));
        let newer_idx = out.find("newer").expect("newer row");
        let older_idx = out.find("older").expect("older row");
        assert!(newer_idx < older_idx);
    }

    #[tokio::test]
    async fn forbidden_message_surfaces_reason_and_rule() {
        let mut payloads = HashMap::new();
        payloads.insert(
            "/v1/workspace/info".to_string(),
            json!({
                "ok": false,
                "error": {
                    "code": "OB_FORBIDDEN",
                    "message": "forbidden",
                    "details": {
                        "reason_code": "OB_POLICY_DENY_AUDIT",
                        "policy_rule_id": "rule-123"
                    }
                }
            }),
        );
        let fake = FakeTransport::new(payloads);
        let err = run_workspace_info(&fake, &args_with_scope())
            .await
            .expect_err("must fail");
        let msg = err.user_message();
        assert_eq!(msg, "DENIED: OB_POLICY_DENY_AUDIT (rule: rule-123)");
    }

    #[test]
    fn invalid_timestamp_is_rejected() {
        let args = HttpArgs {
            token: "token".to_string(),
            scope: Some("ws".to_string()),
            from: Some("not-a-time".to_string()),
            to: None,
            limit: None,
        };
        let err = validate_rfc3339("--from", &args.from).expect_err("invalid");
        assert!(err.user_message().contains("invalid --from timestamp"));
    }
}
