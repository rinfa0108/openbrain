use axum::http::HeaderMap;
use openbrain_core::{ErrorCode, ErrorEnvelope};
use openbrain_store::{AuthContext, AuthStore};

#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Read,
    Search,
    Write,
    Admin,
}

pub async fn authenticate_bearer<S>(
    headers: &HeaderMap,
    store: &S,
) -> Result<AuthContext, ErrorEnvelope>
where
    S: AuthStore,
{
    let token = extract_bearer_token(headers)?;
    store.auth_from_token(&token).await
}

pub fn extract_bearer_token(headers: &HeaderMap) -> Result<String, ErrorEnvelope> {
    let raw = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let mut parts = raw.split_whitespace();
    let scheme = parts.next().unwrap_or("");
    let token = parts.next().unwrap_or("");

    if scheme.eq_ignore_ascii_case("Bearer") && !token.trim().is_empty() {
        Ok(token.to_string())
    } else {
        Err(ErrorEnvelope::new(
            ErrorCode::ObUnauthenticated,
            "missing or invalid Authorization header",
            None,
        ))
    }
}

pub fn authorize(ctx: &AuthContext, op: Operation) -> Result<(), ErrorEnvelope> {
    let allowed = match op {
        Operation::Read | Operation::Search => ctx.role.can_read(),
        Operation::Write => ctx.role.can_write(),
        Operation::Admin => ctx.role.can_admin(),
    };

    if allowed {
        Ok(())
    } else {
        Err(ErrorEnvelope::new(
            ErrorCode::ObForbidden,
            "insufficient role",
            Some(serde_json::json!({"role": ctx.role.as_str(), "operation": format!("{:?}", op)})),
        ))
    }
}

pub fn ensure_scope(ctx: &AuthContext, scope: &str) -> Result<(), ErrorEnvelope> {
    // Workspace boundary is enforced by requiring scope == workspace_id.
    if scope.trim().is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObScopeRequired,
            "scope is required",
            None,
        ));
    }

    if ctx.workspace_id == scope {
        Ok(())
    } else {
        Err(ErrorEnvelope::new(
            ErrorCode::ObForbidden,
            "cross-workspace access denied",
            Some(serde_json::json!({
                "workspace_id": ctx.workspace_id,
                "scope": scope,
            })),
        ))
    }
}

pub fn ensure_object_scopes(
    ctx: &AuthContext,
    objects: &[openbrain_core::MemoryObject],
) -> Result<(), ErrorEnvelope> {
    let mut mismatched = Vec::new();
    for obj in objects {
        let scope = obj.scope.as_deref().unwrap_or("");
        if !scope.is_empty() && scope != ctx.workspace_id {
            mismatched.push(serde_json::json!({"id": obj.id, "scope": scope}));
        }
    }

    if mismatched.is_empty() {
        Ok(())
    } else {
        Err(ErrorEnvelope::new(
            ErrorCode::ObForbidden,
            "cross-workspace access denied",
            Some(serde_json::json!({"workspace_id": ctx.workspace_id, "mismatched": mismatched})),
        ))
    }
}
