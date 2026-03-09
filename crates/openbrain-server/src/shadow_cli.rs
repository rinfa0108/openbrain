use openbrain_core::{ErrorCode, ErrorEnvelope, LifecycleState, MemoryObject, SPEC_VERSION};
use serde::Serialize;
use serde_json::{json, Value};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowMode {
    DryRun,
    WriteScratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowInputFormat {
    Text,
    Json,
}

pub fn parse_mode(raw: &str) -> Result<ShadowMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "dry-run" => Ok(ShadowMode::DryRun),
        "write-scratch" => Ok(ShadowMode::WriteScratch),
        _ => Err(format!(
            "invalid --mode value: {raw} (expected dry-run|write-scratch)"
        )),
    }
}

pub fn parse_input_format(raw: &str) -> Result<ShadowInputFormat, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" => Ok(ShadowInputFormat::Text),
        "json" => Ok(ShadowInputFormat::Json),
        _ => Err(format!(
            "invalid --format value: {raw} (expected text|json)"
        )),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShadowCandidate {
    pub kind: String,
    pub memory_key: String,
    pub value: Value,
    pub source_line: String,
    pub rule: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShadowReport {
    pub mode: String,
    pub workspace: String,
    pub extracted_count: usize,
    pub written_count: usize,
    pub extracted: Vec<ShadowCandidate>,
    pub written_refs: Vec<String>,
}

fn parse_topic_after_prefix(line: &str, prefix: &str) -> String {
    let rest = line
        .trim()
        .strip_prefix(prefix)
        .map(|v| v.trim())
        .unwrap_or_default();
    if rest.is_empty() {
        "general".to_string()
    } else {
        rest.to_string()
    }
}

fn short_topic(line: &str) -> String {
    let words = line
        .split_whitespace()
        .take(4)
        .map(|w| w.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        "general".to_string()
    } else {
        words.join(" ")
    }
}

fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        "general".to_string()
    } else {
        out
    }
}

fn stable_hash_hex(input: &str) -> String {
    let mut hash: u64 = 1469598103934665603;
    for b in input.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

fn classify_text_line(line: &str) -> Option<ShadowCandidate> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();

    if lower.starts_with("decision:") || lower.contains("we decided") || lower.contains("approved")
    {
        let topic = if lower.starts_with("decision:") {
            parse_topic_after_prefix(trimmed, "decision:")
        } else {
            short_topic(trimmed)
        };
        return Some(ShadowCandidate {
            kind: "decision".to_string(),
            memory_key: format!("decision:{}", slugify(&topic)),
            value: json!({"text": trimmed, "topic": topic}),
            source_line: trimmed.to_string(),
            rule: "decision-rule".to_string(),
        });
    }

    if lower.contains(" likes ")
        || lower.contains(" prefers ")
        || lower.contains(" favorite ")
        || lower.starts_with("likes ")
        || lower.starts_with("prefers ")
    {
        let topic = short_topic(trimmed);
        return Some(ShadowCandidate {
            kind: "preference".to_string(),
            memory_key: format!("preference:{}", slugify(&topic)),
            value: json!({"text": trimmed, "topic": topic}),
            source_line: trimmed.to_string(),
            rule: "preference-rule".to_string(),
        });
    }

    if lower.starts_with("todo:") || lower.starts_with("next:") || lower.starts_with("action:") {
        let topic = if lower.starts_with("todo:") {
            parse_topic_after_prefix(trimmed, "todo:")
        } else if lower.starts_with("next:") {
            parse_topic_after_prefix(trimmed, "next:")
        } else {
            parse_topic_after_prefix(trimmed, "action:")
        };
        return Some(ShadowCandidate {
            kind: "task".to_string(),
            memory_key: format!("task:{}", slugify(&topic)),
            value: json!({"text": trimmed, "topic": topic}),
            source_line: trimmed.to_string(),
            rule: "task-rule".to_string(),
        });
    }

    if lower.contains(" must ") || lower.contains(" should not ") || lower.contains("prohibited") {
        let topic = short_topic(trimmed);
        return Some(ShadowCandidate {
            kind: "constraint".to_string(),
            memory_key: format!("constraint:{}", slugify(&topic)),
            value: json!({"text": trimmed, "topic": topic}),
            source_line: trimmed.to_string(),
            rule: "constraint-rule".to_string(),
        });
    }

    None
}

fn parse_json_candidate(item: &Value) -> Option<ShadowCandidate> {
    match item {
        Value::String(text) => classify_text_line(text),
        Value::Object(map) => {
            let raw_text = map
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            if raw_text.is_empty() {
                return None;
            }
            let mut inferred = classify_text_line(&raw_text)?;
            if let Some(kind) = map.get("kind").and_then(Value::as_str) {
                let kind = kind.trim().to_ascii_lowercase();
                if !kind.is_empty() {
                    inferred.kind = kind;
                }
            }
            if let Some(memory_key) = map.get("memory_key").and_then(Value::as_str) {
                let key = memory_key.trim().to_ascii_lowercase();
                if !key.is_empty() {
                    inferred.memory_key = key;
                }
            }
            if let Some(data) = map.get("value") {
                inferred.value = data.clone();
            }
            Some(inferred)
        }
        _ => None,
    }
}

pub fn extract_candidates(
    raw: &str,
    input_format: ShadowInputFormat,
    limit: usize,
) -> Result<Vec<ShadowCandidate>, ErrorEnvelope> {
    let mut out = Vec::new();
    match input_format {
        ShadowInputFormat::Text => {
            for line in raw.lines() {
                if let Some(candidate) = classify_text_line(line) {
                    out.push(candidate);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
        ShadowInputFormat::Json => {
            let parsed: Value = serde_json::from_str(raw).map_err(|e| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "invalid JSON input for --format json",
                    Some(json!({"error": e.to_string()})),
                )
            })?;
            let arr = parsed.as_array().ok_or_else(|| {
                ErrorEnvelope::new(
                    ErrorCode::ObInvalidRequest,
                    "json input must be an array",
                    None,
                )
            })?;
            for item in arr {
                if let Some(candidate) = parse_json_candidate(item) {
                    out.push(candidate);
                    if out.len() >= limit {
                        break;
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn candidates_to_objects(workspace: &str, candidates: &[ShadowCandidate]) -> Vec<MemoryObject> {
    candidates
        .iter()
        .map(|c| {
            let object_id = format!(
                "shadow:{}:{}",
                c.kind,
                stable_hash_hex(&format!("{workspace}|{}|{}", c.memory_key, c.source_line))
            );
            MemoryObject {
                object_type: Some(c.kind.clone()),
                id: Some(object_id),
                scope: Some(workspace.to_string()),
                status: Some("active".to_string()),
                spec_version: Some(SPEC_VERSION.to_string()),
                tags: Some(vec!["shadow".to_string()]),
                data: Some(c.value.clone()),
                provenance: Some(json!({
                    "source": "shadow",
                    "extractor": "heuristic.v1",
                    "rule": c.rule
                })),
                lifecycle_state: Some(LifecycleState::Scratch),
                expires_at: None,
                memory_key: Some(c.memory_key.clone()),
                conflict_status: None,
                resolved_by_object_id: None,
                resolved_at: None,
                resolution_note: None,
            }
        })
        .collect()
}

pub fn format_candidates_text(candidates: &[ShadowCandidate]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "kind | memory_key | text");
    for c in candidates {
        let text = c.source_line.replace('\n', " ");
        let _ = writeln!(out, "{} | {} | {}", c.kind, c.memory_key, text);
    }
    out
}

pub fn format_user_error(err: &ErrorEnvelope) -> String {
    if err.code == ErrorCode::ObForbidden.as_str() {
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

pub fn render_html_report(report: &ShadowReport) -> String {
    let mut rows = String::new();
    for c in &report.extracted {
        let value = serde_json::to_string(&c.value).unwrap_or_else(|_| "{}".to_string());
        let _ = write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td><pre>{}</pre></td></tr>",
            html_escape(&c.kind),
            html_escape(&c.memory_key),
            html_escape(&value)
        );
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>OpenBrain Shadow Report</title>\
<style>body{{font-family:Arial,sans-serif;margin:24px}}table{{border-collapse:collapse;width:100%}}\
td,th{{border:1px solid #ccc;padding:8px;vertical-align:top}}pre{{margin:0;white-space:pre-wrap}}</style></head>\
<body><h1>OpenBrain Shadow Report</h1><p>mode: {}</p><p>workspace: {}</p><p>extracted: {} | written: {}</p>\
<table><thead><tr><th>kind</th><th>memory_key</th><th>value</th></tr></thead><tbody>{}</tbody></table></body></html>",
        html_escape(&report.mode),
        html_escape(&report.workspace),
        report.extracted_count,
        report.written_count,
        rows
    )
}

#[cfg(test)]
fn execute_mode<F>(mode: ShadowMode, mut writer: F) -> Result<Vec<String>, ErrorEnvelope>
where
    F: FnMut() -> Result<Vec<String>, ErrorEnvelope>,
{
    match mode {
        ShadowMode::DryRun => Ok(Vec::new()),
        ShadowMode::WriteScratch => writer(),
    }
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_extraction_snapshot_like() {
        let input = "We decided to use Postgres\nAlex likes dark mode\ntodo: ship release notes";
        let a = extract_candidates(input, ShadowInputFormat::Text, 10).expect("extract");
        let b = extract_candidates(input, ShadowInputFormat::Text, 10).expect("extract");
        assert_eq!(a, b);
        assert_eq!(a.len(), 3);
        assert_eq!(a[0].kind, "decision");
        assert_eq!(a[1].kind, "preference");
        assert_eq!(a[2].kind, "task");
    }

    #[test]
    fn respects_limit() {
        let input = "decision: a\ndecision: b\ndecision: c";
        let out = extract_candidates(input, ShadowInputFormat::Text, 2).expect("extract");
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn write_objects_force_scratch_lifecycle() {
        let input = "decision: use local embeddings";
        let c = extract_candidates(input, ShadowInputFormat::Text, 10).expect("extract");
        let objects = candidates_to_objects("ws-default", &c);
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].lifecycle_state, Some(LifecycleState::Scratch));
    }

    #[test]
    fn forbidden_error_formats_explainability() {
        let err = ErrorEnvelope::new(
            ErrorCode::ObForbidden,
            "denied",
            Some(json!({"reason_code":"OB_POLICY_DENY_KIND","policy_rule_id":"rule-1"})),
        );
        assert_eq!(
            format_user_error(&err),
            "DENIED: OB_POLICY_DENY_KIND (rule: rule-1)"
        );
    }

    #[test]
    fn dry_run_does_not_call_writer() {
        let mut called = false;
        let refs = execute_mode(ShadowMode::DryRun, || {
            called = true;
            Ok(vec!["x".to_string()])
        })
        .expect("execute");
        assert!(!called);
        assert!(refs.is_empty());
    }
}
