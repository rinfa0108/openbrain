use openbrain_core::{Envelope, ErrorCode, ErrorEnvelope, SPEC_VERSION};
use openbrain_llm::AnthropicClient;
use openbrain_server::service;
use openbrain_store::{
    AuditActorActivityRequest, AuditMemoryKeyTimelineRequest, AuditObjectTimelineRequest,
    AuthContext, AuthStore, EmbedGenerateRequest, GetObjectsRequest, PutObjectsRequest,
    SearchSemanticRequest, SearchStructuredRequest, Store, TokenCreateRequest, WorkspaceRole,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use openbrain_server::auth;
use openbrain_server::policy;

#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Default)]
struct SessionState {
    auth: Option<AuthContext>,
}

#[derive(Debug)]
struct RpcResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    payload: RpcPayload,
}

#[derive(Debug)]
enum RpcPayload {
    Result(Value),
    Error {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

impl RpcResponse {
    fn result(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Result(result),
        }
    }

    fn error(
        id: Option<Value>,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            payload: RpcPayload::Error {
                code,
                message: message.into(),
                data,
            },
        }
    }

    fn to_json(&self) -> Value {
        match &self.payload {
            RpcPayload::Result(result) => {
                serde_json::json!({"jsonrpc": self.jsonrpc, "id": self.id, "result": result})
            }
            RpcPayload::Error {
                code,
                message,
                data,
            } => serde_json::json!({
                "jsonrpc": self.jsonrpc,
                "id": self.id,
                "error": { "code": code, "message": message, "data": data }
            }),
        }
    }
}

const MAX_LINE_BYTES: usize = 12 * 1024 * 1024;

fn env_err<T>(code: ErrorCode, message: impl Into<String>, details: Option<Value>) -> Envelope<T> {
    Envelope::err(ErrorEnvelope::new(code, message, details))
}

fn unauthenticated() -> Envelope<Value> {
    Envelope::err(ErrorEnvelope::new(
        ErrorCode::ObUnauthenticated,
        "authentication required",
        None,
    ))
}

fn envelope_to_value<T>(env: Envelope<T>) -> Envelope<Value>
where
    T: Serialize,
{
    match env {
        Envelope::Ok { ok, data } => match serde_json::to_value(data) {
            Ok(v) => Envelope::Ok { ok, data: v },
            Err(e) => env_err(
                ErrorCode::ObInternal,
                "serialization error",
                Some(serde_json::json!({"error": e.to_string()})),
            ),
        },
        Envelope::Err { ok, error } => Envelope::Err { ok, error },
    }
}

#[derive(Debug, Deserialize)]
struct ToolsCallParams {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ReadArgs {
    scope: String,
    refs: Vec<String>,
    #[serde(default)]
    include_states: Option<Vec<openbrain_core::LifecycleState>>,
    #[serde(default)]
    include_expired: Option<bool>,
    #[serde(default)]
    include_conflicts: Option<bool>,
    #[serde(default)]
    now: Option<String>,
}

pub async fn run_mcp_stdio<S>(store: S, llm: AnthropicClient) -> std::io::Result<()>
where
    S: Store + AuthStore + Clone + 'static,
{
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut reader = BufReader::new(stdin).lines();
    let mut out = tokio::io::BufWriter::new(stdout);
    let mut session = SessionState::default();

    while let Some(line) = reader.next_line().await? {
        if line.len() > MAX_LINE_BYTES {
            let resp = RpcResponse::error(
                None,
                -32700,
                "parse error",
                Some(serde_json::json!({"error": "line too large"})),
            );
            out.write_all(serde_json::to_string(&resp.to_json()).unwrap().as_bytes())
                .await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
            continue;
        }

        let req: Result<RpcRequest, _> = serde_json::from_str(&line);
        let resp = match req {
            Ok(r) => handle_request(store.clone(), llm.clone(), &mut session, r).await,
            Err(e) => RpcResponse::error(
                None,
                -32700,
                "parse error",
                Some(serde_json::json!({"error": e.to_string()})),
            ),
        };

        let serialized = serde_json::to_string(&resp.to_json()).unwrap_or_else(|_| {
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32603,\"message\":\"internal error\"}}".to_string()
        });

        out.write_all(serialized.as_bytes()).await?;
        out.write_all(b"\n").await?;
        out.flush().await?;
    }

    Ok(())
}

async fn handle_request<S>(
    store: S,
    llm: AnthropicClient,
    session: &mut SessionState,
    req: RpcRequest,
) -> RpcResponse
where
    S: Store + AuthStore + Clone + 'static,
{
    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => {
            if let Some(params) = req.params.as_ref() {
                if let Some(token) = params.get("auth_token").and_then(|v| v.as_str()) {
                    match store.auth_from_token(token).await {
                        Ok(ctx) => session.auth = Some(ctx),
                        Err(e) => {
                            return RpcResponse::error(
                                id,
                                -32001,
                                "unauthenticated",
                                Some(serde_json::to_value(Envelope::<Value>::err(e)).unwrap()),
                            )
                        }
                    }
                }
            }
            let server_info = serde_json::json!({"name":"openbrain","version":SPEC_VERSION});
            let result = serde_json::json!({
                "serverInfo": server_info,
                "capabilities": {"tools": {}},
                "protocolVersion": req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("protocolVersion"))
                    .cloned()
                    .unwrap_or(Value::Null)
            });
            RpcResponse::result(id, result)
        }
        "tools/list" => {
            let tools = serde_json::json!({
                "tools": [
                    {"name":"openbrain.ping","description":"Ping OpenBrain","inputSchema": {"type":"object","properties":{},"additionalProperties":false}},
                    {"name":"openbrain.write","description":"Write objects","inputSchema": {"type":"object"}},
                    {"name":"openbrain.read","description":"Read objects (scoped)","inputSchema": {"type":"object"}},
                    {"name":"openbrain.search.structured","description":"Structured search","inputSchema": {"type":"object"}},
                    {"name":"openbrain.embed.generate","description":"Generate embedding","inputSchema": {"type":"object"}},
                    {"name":"openbrain.search.semantic","description":"Semantic search","inputSchema": {"type":"object"}},
                    {"name":"openbrain.rerank","description":"Rerank candidates","inputSchema": {"type":"object"}},
                    {"name":"openbrain.memory.pack","description":"Build memory pack","inputSchema": {"type":"object"}},
                    {"name":"openbrain.workspace.token.create","description":"Create workspace token","inputSchema": {"type":"object"}},
                    {"name":"openbrain.workspace.info","description":"Get workspace ownership and caller role","inputSchema": {"type":"object","properties":{},"additionalProperties":false}},
                    {"name":"openbrain.audit.object_timeline","description":"Audit timeline by object id","inputSchema": {"type":"object"}},
                    {"name":"openbrain.audit.memory_key_timeline","description":"Audit timeline by memory key","inputSchema": {"type":"object"}},
                    {"name":"openbrain.audit.actor_activity","description":"Audit activity by actor identity","inputSchema": {"type":"object"}}
                ]
            });
            RpcResponse::result(id, tools)
        }
        "tools/call" => {
            let params = req.params.unwrap_or(Value::Null);
            let call: ToolsCallParams = match serde_json::from_value(params) {
                Ok(v) => v,
                Err(e) => {
                    return RpcResponse::result(
                        id,
                        serde_json::to_value(env_err::<Value>(
                            ErrorCode::ObInvalidRequest,
                            "invalid params",
                            Some(serde_json::json!({"error": e.to_string()})),
                        ))
                        .unwrap(),
                    )
                }
            };

            let result = dispatch_tool(store, llm, session.auth.as_ref(), call).await;
            RpcResponse::result(id, serde_json::to_value(result).unwrap())
        }
        _ => RpcResponse::error(id, -32601, "method not found", None),
    }
}

async fn dispatch_tool<S>(
    store: S,
    llm: AnthropicClient,
    auth_ctx: Option<&AuthContext>,
    call: ToolsCallParams,
) -> Envelope<Value>
where
    S: Store + AuthStore + Clone + 'static,
{
    match call.name.as_str() {
        "openbrain.ping" => {
            let now = chrono::Utc::now().to_rfc3339();
            Envelope::ok(serde_json::json!({
                "version": SPEC_VERSION,
                "server_time": now
            }))
        }
        "openbrain.write" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Write) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let mut req: PutObjectsRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };
            if let Err(e) = auth::ensure_object_scopes(auth_ctx, &req.objects) {
                return Envelope::err(e);
            }
            if let Err(e) = policy::validate_policy_write_permissions(auth_ctx.role, &req.objects) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let retention =
                match policy::load_workspace_retention_policy(&store, &auth_ctx.workspace_id).await
                {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            if let Err(e) = policy::apply_retention_policy_to_objects(
                retention.as_ref(),
                &mut req.objects,
                chrono::Utc::now(),
            ) {
                return Envelope::err(e);
            }
            let refs: Vec<String> = req.objects.iter().filter_map(|o| o.id.clone()).collect();
            let mut existing =
                std::collections::HashMap::<String, openbrain_core::LifecycleState>::new();
            if !refs.is_empty() {
                let current = store
                    .get_objects(GetObjectsRequest {
                        scope: auth_ctx.workspace_id.clone(),
                        refs,
                        include_states: Some(vec![
                            openbrain_core::LifecycleState::Scratch,
                            openbrain_core::LifecycleState::Candidate,
                            openbrain_core::LifecycleState::Accepted,
                            openbrain_core::LifecycleState::Deprecated,
                        ]),
                        include_expired: Some(true),
                        include_conflicts: Some(false),
                        now: None,
                    })
                    .await;
                if let Envelope::Ok { data, .. } = current {
                    for obj in data.objects {
                        existing.insert(obj.id, obj.lifecycle_state);
                    }
                }
            }
            for obj in &req.objects {
                let next = obj
                    .lifecycle_state
                    .unwrap_or(openbrain_core::LifecycleState::Accepted);
                let prev = obj.id.as_ref().and_then(|id| existing.get(id).copied());
                let transition = policy::lifecycle_transition(prev, next);
                let decision = policy::evaluate(
                    &policies,
                    &policy::EvalInput {
                        identity_id: &auth_ctx.identity_id,
                        role: auth_ctx.role,
                        operation: policy::PolicyOperation::Write,
                        object_kind: obj.object_type.as_deref(),
                        memory_key: obj.memory_key.as_deref(),
                        lifecycle_transition: transition.as_deref(),
                    },
                );
                if !decision.allowed {
                    return Envelope::err(policy::deny_error_with_rule(
                        decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                        decision.policy_rule_id.as_deref(),
                        Some(serde_json::json!({"object_id": obj.id, "operation": "write"})),
                    ));
                }
                if let Some(max_bytes) = decision.max_write_bytes {
                    let payload_size = serde_json::to_vec(obj).map(|b| b.len()).unwrap_or(0) as u64;
                    if payload_size > max_bytes {
                        return Envelope::err(policy::deny_error_with_rule(
                            "OB_POLICY_DENY_MAX_WRITE_BYTES",
                            decision.policy_rule_id.as_deref(),
                            Some(
                                serde_json::json!({"object_id": obj.id, "max_write_bytes": max_bytes, "payload_bytes": payload_size}),
                            ),
                        ));
                    }
                }
            }

            envelope_to_value(store.put_objects(req).await)
        }
        "openbrain.read" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Read) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let req: ReadArgs = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };

            let resp = store
                .get_objects(GetObjectsRequest {
                    scope: req.scope.clone(),
                    refs: req.refs,
                    include_states: req.include_states,
                    include_expired: req.include_expired,
                    include_conflicts: req.include_conflicts,
                    now: req.now,
                })
                .await;

            match resp {
                Envelope::Ok { ok: _, data } => {
                    for object in &data.objects {
                        let decision = policy::evaluate(
                            &policies,
                            &policy::EvalInput {
                                identity_id: &auth_ctx.identity_id,
                                role: auth_ctx.role,
                                operation: policy::PolicyOperation::Read,
                                object_kind: Some(object.object_type.as_str()),
                                memory_key: object.memory_key.as_deref(),
                                lifecycle_transition: None,
                            },
                        );
                        if !decision.allowed {
                            return Envelope::err(policy::deny_error_with_rule(
                                decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                                decision.policy_rule_id.as_deref(),
                                Some(
                                    serde_json::json!({"object_id": object.id, "operation": "read"}),
                                ),
                            ));
                        }
                    }
                    let mismatched: Vec<String> = data
                        .objects
                        .iter()
                        .filter(|o| o.scope != req.scope)
                        .map(|o| o.id.clone())
                        .collect();

                    if mismatched.is_empty() {
                        envelope_to_value(Envelope::ok(data))
                    } else {
                        env_err(
                            ErrorCode::ObNotFound,
                            "one or more refs not found in scope",
                            Some(serde_json::json!({"missing_refs": mismatched})),
                        )
                    }
                }
                Envelope::Err { ok: _, error } => Envelope::err(error),
            }
        }
        "openbrain.search.structured" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Search) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let mut req: SearchStructuredRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::SearchStructured,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"search_structured"})),
                ));
            }
            req.limit = policy::clamp_u32(req.limit, decision.max_top_k);
            match store.search_structured(req).await {
                Envelope::Ok { ok, mut data } => {
                    data.results.retain(|item| {
                        policy::evaluate(
                            &policies,
                            &policy::EvalInput {
                                identity_id: &auth_ctx.identity_id,
                                role: auth_ctx.role,
                                operation: policy::PolicyOperation::SearchStructured,
                                object_kind: Some(item.object_type.as_str()),
                                memory_key: None,
                                lifecycle_transition: None,
                            },
                        )
                        .allowed
                    });
                    envelope_to_value(Envelope::Ok { ok, data })
                }
                Envelope::Err { ok, error } => Envelope::Err { ok, error },
            }
        }
        "openbrain.embed.generate" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Write) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let req: EmbedGenerateRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::EmbedGenerate,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"embed_generate"})),
                ));
            }

            envelope_to_value(store.embed_generate(req).await)
        }
        "openbrain.search.semantic" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Search) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let mut req: SearchSemanticRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::SearchSemantic,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"search_semantic"})),
                ));
            }
            req.top_k = policy::clamp_u32(req.top_k, decision.max_top_k);
            match store.search_semantic(req).await {
                Envelope::Ok { ok, mut data } => {
                    data.matches.retain(|item| {
                        policy::evaluate(
                            &policies,
                            &policy::EvalInput {
                                identity_id: &auth_ctx.identity_id,
                                role: auth_ctx.role,
                                operation: policy::PolicyOperation::SearchSemantic,
                                object_kind: Some(item.kind.as_str()),
                                memory_key: None,
                                lifecycle_transition: None,
                            },
                        )
                        .allowed
                    });
                    envelope_to_value(Envelope::Ok { ok, data })
                }
                Envelope::Err { ok, error } => Envelope::Err { ok, error },
            }
        }
        "openbrain.rerank" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Search) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let req: service::RerankRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({ "error": e.to_string() })),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }

            envelope_to_value(service::rerank(&store, &llm, req).await)
        }
        "openbrain.memory.pack" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Search) {
                return Envelope::err(e);
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let req: service::MemoryPackRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({ "error": e.to_string() })),
                    )
                }
            };

            if let Err(e) = auth::ensure_scope(auth_ctx, &req.scope) {
                return Envelope::err(e);
            }

            envelope_to_value(service::build_pack(&store, &llm, req).await)
        }
        "openbrain.workspace.token.create" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Admin) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::Admin,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"admin"})),
                ));
            }

            #[derive(Debug, Deserialize)]
            struct WorkspaceTokenCreateArgs {
                role: WorkspaceRole,
                #[serde(default)]
                label: Option<String>,
                #[serde(default)]
                display_name: Option<String>,
            }

            let args = call.arguments.unwrap_or(Value::Null);
            let req: WorkspaceTokenCreateArgs = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({ "error": e.to_string() })),
                    )
                }
            };

            match store
                .create_token(TokenCreateRequest {
                    workspace_id: auth_ctx.workspace_id.clone(),
                    role: req.role,
                    label: req.label,
                    display_name: req.display_name,
                })
                .await
            {
                Ok(v) => envelope_to_value(Envelope::ok(v)),
                Err(e) => Envelope::err(e),
            }
        }
        "openbrain.workspace.info" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Read) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::WorkspaceInfo,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"workspace_info"})),
                ));
            }
            match store
                .workspace_info(&auth_ctx.workspace_id, &auth_ctx.identity_id, auth_ctx.role)
                .await
            {
                Ok(v) => envelope_to_value(Envelope::ok(v)),
                Err(e) => Envelope::err(e),
            }
        }
        "openbrain.audit.object_timeline" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Read) {
                return Envelope::err(e);
            }
            let args = call.arguments.unwrap_or(Value::Null);
            let req: AuditObjectTimelineRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };
            if let Err(e) = auth::ensure_scope(auth_ctx, &req.query.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::AuditObjectTimeline,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"audit_object_timeline"})),
                ));
            }
            envelope_to_value(store.audit_object_timeline(req).await)
        }
        "openbrain.audit.memory_key_timeline" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Read) {
                return Envelope::err(e);
            }
            let args = call.arguments.unwrap_or(Value::Null);
            let req: AuditMemoryKeyTimelineRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };
            if let Err(e) = auth::ensure_scope(auth_ctx, &req.query.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::AuditMemoryKeyTimeline,
                    object_kind: None,
                    memory_key: Some(req.memory_key.as_str()),
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"audit_memory_key_timeline"})),
                ));
            }
            envelope_to_value(store.audit_memory_key_timeline(req).await)
        }
        "openbrain.audit.actor_activity" => {
            let Some(auth_ctx) = auth_ctx else {
                return unauthenticated();
            };
            if let Err(e) = auth::authorize(auth_ctx, auth::Operation::Read) {
                return Envelope::err(e);
            }
            let args = call.arguments.unwrap_or(Value::Null);
            let req: AuditActorActivityRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };
            if let Err(e) = auth::ensure_scope(auth_ctx, &req.query.scope) {
                return Envelope::err(e);
            }
            let policies =
                match policy::load_workspace_policies(&store, &auth_ctx.workspace_id).await {
                    Ok(v) => v,
                    Err(e) => return Envelope::err(e),
                };
            let decision = policy::evaluate(
                &policies,
                &policy::EvalInput {
                    identity_id: &auth_ctx.identity_id,
                    role: auth_ctx.role,
                    operation: policy::PolicyOperation::AuditActorActivity,
                    object_kind: None,
                    memory_key: None,
                    lifecycle_transition: None,
                },
            );
            if !decision.allowed {
                return Envelope::err(policy::deny_error_with_rule(
                    decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                    decision.policy_rule_id.as_deref(),
                    Some(serde_json::json!({"operation":"audit_actor_activity"})),
                ));
            }
            envelope_to_value(store.audit_actor_activity(req).await)
        }
        _ => env_err(
            ErrorCode::ObInvalidRequest,
            format!("unknown tool: {}", call.name),
            None,
        ),
    }
}
