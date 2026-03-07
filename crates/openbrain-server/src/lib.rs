use axum::{
    body::Bytes,
    extract::{rejection::BytesRejection, DefaultBodyLimit, State},
    http::HeaderMap,
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use openbrain_core::{Envelope, ErrorCode, ErrorEnvelope};
use openbrain_llm::AnthropicClient;
pub mod auth;
pub mod service;
use openbrain_store::{
    AuthStore, EmbedGenerateRequest, GetObjectsRequest, PutObjectsRequest, SearchSemanticRequest,
    SearchStructuredRequest, Store, TokenCreateRequest, TokenCreateResponse, WorkspaceRole,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tower_http::trace::TraceLayer;

const MAX_BODY_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState<S> {
    pub store: S,
    pub llm: AnthropicClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PingResponse {
    pub version: String,
    pub server_time: String,
}

pub fn build_router<S>(state: AppState<S>) -> Router
where
    S: Store + AuthStore + Clone + 'static,
{
    Router::new()
        .route("/v1/ping", post(ping))
        .route("/v1/write", post(write::<S>))
        .route("/v1/read", post(read::<S>))
        .route("/v1/search/structured", post(search_structured::<S>))
        .route("/v1/embed/generate", post(embed_generate::<S>))
        .route("/v1/search/semantic", post(search_semantic::<S>))
        .route("/v1/rerank", post(rerank::<S>))
        .route("/v1/memory/pack", post(memory_pack::<S>))
        .route(
            "/v1/workspace/token/create",
            post(workspace_token_create::<S>),
        )
        .with_state(state)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
}

fn err<T>(code: ErrorCode, message: impl Into<String>, details: Option<Value>) -> Envelope<T> {
    Envelope::err(ErrorEnvelope::new(code, message, details))
}

fn parse_json_body<T>(body: Result<Bytes, BytesRejection>) -> Result<T, ErrorEnvelope>
where
    T: DeserializeOwned,
{
    let bytes = body.map_err(|e| {
        ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "invalid request body",
            Some(serde_json::json!({ "error": e.to_string() })),
        )
    })?;

    if bytes.len() > MAX_BODY_BYTES {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "request body too large",
            Some(serde_json::json!({
                "max_bytes": MAX_BODY_BYTES,
                "got_bytes": bytes.len(),
            })),
        ));
    }

    if bytes.is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "missing JSON body",
            None,
        ));
    }

    serde_json::from_slice(&bytes).map_err(|e| {
        ErrorEnvelope::new(
            ErrorCode::ObInvalidRequest,
            "invalid JSON",
            Some(serde_json::json!({ "error": e.to_string() })),
        )
    })
}

async fn ping(body: Result<Bytes, BytesRejection>) -> impl IntoResponse {
    if let Err(e) = body {
        return Json(err::<PingResponse>(
            ErrorCode::ObInvalidRequest,
            "invalid request body",
            Some(serde_json::json!({ "error": e.to_string() })),
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();
    Json(Envelope::ok(PingResponse {
        version: openbrain_core::SPEC_VERSION.to_string(),
        server_time: now,
    }))
}

async fn write<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<openbrain_store::PutObjectsResponse>>(Envelope::err(e)),
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Write) {
        return Json::<Envelope<openbrain_store::PutObjectsResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<PutObjectsRequest>(body) {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<openbrain_store::PutObjectsResponse>>(Envelope::err(e)),
    };

    if let Err(e) = auth::ensure_object_scopes(&auth, &req.objects) {
        return Json::<Envelope<openbrain_store::PutObjectsResponse>>(Envelope::err(e));
    }

    Json(state.store.put_objects(req).await)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ReadRequest {
    pub scope: String,
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_states: Option<Vec<openbrain_core::LifecycleState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_expired: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct WorkspaceTokenCreateRequest {
    pub role: WorkspaceRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

async fn read<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<openbrain_store::GetObjectsResponse>>(Envelope::err(e)),
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Read) {
        return Json::<Envelope<openbrain_store::GetObjectsResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<ReadRequest>(body) {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<openbrain_store::GetObjectsResponse>>(Envelope::err(e)),
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<openbrain_store::GetObjectsResponse>>(Envelope::err(e));
    }

    let resp = state
        .store
        .get_objects(GetObjectsRequest {
            scope: req.scope.clone(),
            refs: req.refs,
            include_states: req.include_states,
            include_expired: req.include_expired,
            now: req.now,
        })
        .await;

    match resp {
        Envelope::Ok { ok, data } => {
            let mismatched: Vec<String> = data
                .objects
                .iter()
                .filter(|o| o.scope != req.scope)
                .map(|o| o.id.clone())
                .collect();

            if mismatched.is_empty() {
                Json(Envelope::Ok { ok, data })
            } else {
                Json(err::<openbrain_store::GetObjectsResponse>(
                    ErrorCode::ObNotFound,
                    "one or more refs not found in scope",
                    Some(serde_json::json!({ "missing_refs": mismatched })),
                ))
            }
        }
        Envelope::Err { ok, error } => Json(Envelope::Err { ok, error }),
    }
}

async fn search_structured<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::SearchStructuredResponse>>(Envelope::err(e))
        }
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Search) {
        return Json::<Envelope<openbrain_store::SearchStructuredResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<SearchStructuredRequest>(body) {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::SearchStructuredResponse>>(Envelope::err(e))
        }
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<openbrain_store::SearchStructuredResponse>>(Envelope::err(e));
    }

    Json(state.store.search_structured(req).await)
}

async fn embed_generate<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::EmbedGenerateResponse>>(Envelope::err(e))
        }
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Write) {
        return Json::<Envelope<openbrain_store::EmbedGenerateResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<EmbedGenerateRequest>(body) {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::EmbedGenerateResponse>>(Envelope::err(e))
        }
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<openbrain_store::EmbedGenerateResponse>>(Envelope::err(e));
    }

    Json(state.store.embed_generate(req).await)
}

async fn search_semantic<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::SearchSemanticResponse>>(Envelope::err(e))
        }
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Search) {
        return Json::<Envelope<openbrain_store::SearchSemanticResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<SearchSemanticRequest>(body) {
        Ok(v) => v,
        Err(e) => {
            return Json::<Envelope<openbrain_store::SearchSemanticResponse>>(Envelope::err(e))
        }
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<openbrain_store::SearchSemanticResponse>>(Envelope::err(e));
    }

    Json(state.store.search_semantic(req).await)
}

async fn rerank<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<service::RerankResponse>>(Envelope::err(e)),
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Search) {
        return Json::<Envelope<service::RerankResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<service::RerankRequest>(body) {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<service::RerankResponse>>(Envelope::err(e)),
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<service::RerankResponse>>(Envelope::err(e));
    }

    Json(service::rerank(&state.store, &state.llm, req).await)
}

async fn memory_pack<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<service::MemoryPackResponse>>(Envelope::err(e)),
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Search) {
        return Json::<Envelope<service::MemoryPackResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<service::MemoryPackRequest>(body) {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<service::MemoryPackResponse>>(Envelope::err(e)),
    };

    if let Err(e) = auth::ensure_scope(&auth, &req.scope) {
        return Json::<Envelope<service::MemoryPackResponse>>(Envelope::err(e));
    }

    Json(service::build_pack(&state.store, &state.llm, req).await)
}

async fn workspace_token_create<S>(
    State(state): State<AppState<S>>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> impl IntoResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let auth = match auth::authenticate_bearer(&headers, &state.store).await {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<TokenCreateResponse>>(Envelope::err(e)),
    };
    if let Err(e) = auth::authorize(&auth, auth::Operation::Admin) {
        return Json::<Envelope<TokenCreateResponse>>(Envelope::err(e));
    }

    let req = match parse_json_body::<WorkspaceTokenCreateRequest>(body) {
        Ok(v) => v,
        Err(e) => return Json::<Envelope<TokenCreateResponse>>(Envelope::err(e)),
    };

    let resp = state
        .store
        .create_token(TokenCreateRequest {
            workspace_id: auth.workspace_id.clone(),
            role: req.role,
            label: req.label,
            display_name: req.display_name,
        })
        .await;

    match resp {
        Ok(v) => Json(Envelope::ok(v)),
        Err(e) => Json(Envelope::err(e)),
    }
}
