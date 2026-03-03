use crate::{ErrorCode, ErrorEnvelope};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const CHECKSUM_PREFIX: &str = "sha256:";

pub fn normalize_whitespace(input: &str) -> String {
    let input = input.replace("\r\n", "\n");
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;

    for ch in input.chars() {
        let is_ws = ch.is_whitespace();
        if is_ws {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }

    out.trim().to_string()
}

pub fn canonicalize_json(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) => Value::Number(n.clone()),
        Value::String(s) => Value::String(normalize_whitespace(s)),
        Value::Array(items) => {
            let mut out = Vec::new();
            for it in items {
                let c = canonicalize_json(it);
                if !c.is_null() {
                    out.push(c);
                }
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                let v = canonicalize_json(&map[k]);
                if !v.is_null() {
                    out.insert(k.clone(), v);
                }
            }
            Value::Object(out)
        }
    }
}

fn get_str(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn get_obj<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data.get(key).filter(|v| v.is_object())
}

fn get_arr<'a>(data: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    data.get(key).and_then(|v| v.as_array())
}

pub fn normalize_object_text(object_type: &str, data: &Value) -> Result<String, ErrorEnvelope> {
    let t = object_type.trim().to_ascii_lowercase();

    let normalized = match t.as_str() {
        "entity" => {
            let entity_type = get_str(data, "entity_type").unwrap_or_default();
            let name = get_str(data, "name").unwrap_or_default();
            let props = get_obj(data, "props")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));
            let props = canonicalize_json(&props);
            format!(
                "ENTITY: {entity_type} | {name} | props:{}",
                serde_json::to_string(&props).unwrap_or_else(|_| "{}".to_string())
            )
        }
        "relation" => {
            let src = get_str(data, "src_entity_id").unwrap_or_default();
            let rel_type = get_str(data, "rel_type").unwrap_or_default();
            let dst = get_str(data, "dst_entity_id").unwrap_or_default();
            let props = get_obj(data, "props")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new()));
            let props = canonicalize_json(&props);
            format!(
                "RELATION: {src} {rel_type} {dst} | props:{}",
                serde_json::to_string(&props).unwrap_or_else(|_| "{}".to_string())
            )
        }
        "claim" => {
            let subject = get_str(data, "subject").unwrap_or_default();
            let predicate = get_str(data, "predicate").unwrap_or_default();
            let object = get_str(data, "object").unwrap_or_default();
            let polarity = get_str(data, "polarity").unwrap_or_default();
            format!("CLAIM: subj:{subject} pred:{predicate} obj:{object} pol:{polarity}")
        }
        "decision" => {
            let title = get_str(data, "title").unwrap_or_default();
            let outcome = get_str(data, "outcome").unwrap_or_default();
            let rationale = get_str(data, "rationale").unwrap_or_default();
            format!("DECISION: {title} outcome:{outcome} rationale:{rationale}")
        }
        "task" => {
            let title = get_str(data, "title").unwrap_or_default();
            let state = get_str(data, "state").unwrap_or_default();
            let steps = get_arr(data, "steps")
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            if let Some(s) = v.as_str() {
                                Some(s.to_string())
                            } else {
                                v.get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|s| s.to_string())
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            format!("TASK: {title} state:{state} steps:{}", steps.join(" | "))
        }
        "artifact" => {
            let kind = get_str(data, "kind").unwrap_or_default();
            let uri = get_str(data, "uri").unwrap_or_default();
            let checksum = get_str(data, "checksum").unwrap_or_default();
            format!("ARTIFACT: {kind} uri:{uri} checksum:{checksum}")
        }
        "thought_summary" => {
            let intent = get_str(data, "intent").unwrap_or_default();
            let assumptions = data.get("assumptions").cloned().unwrap_or(Value::Null);
            let constraints = data.get("constraints").cloned().unwrap_or(Value::Null);
            let actions = data.get("actions").cloned().unwrap_or(Value::Null);

            let assumptions = canonicalize_json(&assumptions);
            let constraints = canonicalize_json(&constraints);
            let actions = canonicalize_json(&actions);

            format!(
                "THOUGHT: intent:{intent} assumptions:{} constraints:{} actions:{}",
                serde_json::to_string(&assumptions).unwrap_or_else(|_| "null".to_string()),
                serde_json::to_string(&constraints).unwrap_or_else(|_| "null".to_string()),
                serde_json::to_string(&actions).unwrap_or_else(|_| "null".to_string())
            )
        }
        _ => {
            return Err(ErrorEnvelope::new(
                ErrorCode::ObInvalidRequest,
                "unsupported object type for embedding normalization",
                Some(serde_json::json!({"type": object_type})),
            ));
        }
    };

    Ok(normalize_whitespace(&normalized))
}

pub fn checksum_v01(normalized_text: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"ob.v0.1\n");
    h.update(normalized_text.as_bytes());
    let bytes = h.finalize();
    let mut hex = String::with_capacity(64);
    for b in bytes {
        hex.push_str(&format!("{b:02x}"));
    }
    format!("{CHECKSUM_PREFIX}{hex}")
}

pub fn validate_checksum_format(checksum: &str) -> Result<(), ErrorEnvelope> {
    if !checksum.starts_with(CHECKSUM_PREFIX) {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "checksum must start with sha256:",
            None,
        ));
    }
    let hex = &checksum[CHECKSUM_PREFIX.len()..];
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidSchema,
            "checksum must be sha256:<64 hex>",
            None,
        ));
    }
    Ok(())
}

pub fn reject_obvious_secrets(text: &str) -> Result<(), ErrorEnvelope> {
    let lower = text.to_ascii_lowercase();
    let bad = [
        "-----begin private key-----",
        "api_key",
        "apikey",
        "secret_key",
        "sk-",
        "xoxb-",
        "ghp_",
    ];

    if bad.iter().any(|p| lower.contains(p)) {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "text appears to contain secrets; refusing to embed",
            Some(serde_json::json!({"reason": "secret_heuristic"})),
        ));
    }

    Ok(())
}

pub fn validate_embedding_text(text: &str, max_len: usize) -> Result<String, ErrorEnvelope> {
    if text.chars().any(|c| c == '\0') {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "text appears to be binary",
            None,
        ));
    }
    if text.len() > max_len {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "text exceeds maximum length",
            Some(serde_json::json!({"max_len": max_len, "len": text.len()})),
        ));
    }

    let normalized = normalize_whitespace(text);
    reject_obvious_secrets(&normalized)?;

    if normalized.is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "text is empty after normalization",
            None,
        ));
    }

    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_format_is_sha256_prefixed() {
        let t = "hello world";
        let c = checksum_v01(t);
        assert!(c.starts_with("sha256:"));
        assert_eq!(c.len(), "sha256:".len() + 64);
        validate_checksum_format(&c).unwrap();
    }

    #[test]
    fn canonicalization_sorts_keys_and_removes_nulls() {
        let v = serde_json::json!({"b":1,"a":null,"c":{"z":2,"y":null}});
        let c = canonicalize_json(&v);
        assert_eq!(c, serde_json::json!({"b":1,"c":{"z":2}}));
    }

    #[test]
    fn whitespace_normalizes_crlf_and_collapses() {
        let s = "a\r\n  b\t c";
        assert_eq!(normalize_whitespace(s), "a b c");
    }

    #[test]
    fn normalization_is_stable_for_entity_props_and_line_endings() {
        let a = serde_json::json!({
            "entity_type": "person",
            "name": "Alice\r\n Smith",
            "props": {"b": 2, "a": null, "c": {"z": 1, "y": null}}
        });
        let b = serde_json::json!({
            "name": "Alice\n  Smith",
            "props": {"c": {"y": null, "z": 1}, "a": null, "b": 2},
            "entity_type": "person"
        });

        let na = normalize_object_text("entity", &a).unwrap();
        let nb = normalize_object_text("entity", &b).unwrap();
        assert_eq!(na, nb);

        let ca = checksum_v01(&na);
        let cb = checksum_v01(&nb);
        assert_eq!(ca, cb);
    }
}
