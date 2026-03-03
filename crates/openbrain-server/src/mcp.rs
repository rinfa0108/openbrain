use openbrain_core::{Envelope, ErrorCode, ErrorEnvelope, SPEC_VERSION};
use openbrain_store::{
    EmbedGenerateRequest, GetObjectsRequest, PutObjectsRequest, SearchSemanticRequest,
    SearchStructuredRequest, Store,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
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

fn validate_scope(scope: &str) -> Result<(), ErrorEnvelope> {
    if scope.trim().is_empty() {
        return Err(ErrorEnvelope::new(
            ErrorCode::ObScopeRequired,
            "scope is required",
            None,
        ));
    }
    Ok(())
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
}

pub async fn run_mcp_stdio<S>(store: S) -> std::io::Result<()>
where
    S: Store + Clone + 'static,
{
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let mut reader = BufReader::new(stdin).lines();
    let mut out = tokio::io::BufWriter::new(stdout);

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
            Ok(r) => handle_request(store.clone(), r).await,
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

async fn handle_request<S>(store: S, req: RpcRequest) -> RpcResponse
where
    S: Store + Clone + 'static,
{
    let id = req.id.clone();

    match req.method.as_str() {
        "initialize" => {
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
                    {"name":"openbrain.search.semantic","description":"Semantic search","inputSchema": {"type":"object"}}
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

            let result = dispatch_tool(store, call).await;
            RpcResponse::result(id, serde_json::to_value(result).unwrap())
        }
        _ => RpcResponse::error(id, -32601, "method not found", None),
    }
}

async fn dispatch_tool<S>(store: S, call: ToolsCallParams) -> Envelope<Value>
where
    S: Store + Clone + 'static,
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
            let args = call.arguments.unwrap_or(Value::Null);
            let req: PutObjectsRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };
            envelope_to_value(store.put_objects(req).await)
        }
        "openbrain.read" => {
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

            if let Err(e) = validate_scope(&req.scope) {
                return Envelope::err(e);
            }

            let resp = store
                .get_objects(GetObjectsRequest { refs: req.refs })
                .await;

            match resp {
                Envelope::Ok { ok: _, data } => {
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
            let args = call.arguments.unwrap_or(Value::Null);
            let req: SearchStructuredRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = validate_scope(&req.scope) {
                return Envelope::err(e);
            }

            envelope_to_value(store.search_structured(req).await)
        }
        "openbrain.embed.generate" => {
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

            if let Err(e) = validate_scope(&req.scope) {
                return Envelope::err(e);
            }

            envelope_to_value(store.embed_generate(req).await)
        }
        "openbrain.search.semantic" => {
            let args = call.arguments.unwrap_or(Value::Null);
            let req: SearchSemanticRequest = match serde_json::from_value(args) {
                Ok(v) => v,
                Err(e) => {
                    return env_err(
                        ErrorCode::ObInvalidRequest,
                        "invalid JSON",
                        Some(serde_json::json!({"error": e.to_string()})),
                    )
                }
            };

            if let Err(e) = validate_scope(&req.scope) {
                return Envelope::err(e);
            }

            envelope_to_value(store.search_semantic(req).await)
        }
        _ => env_err(
            ErrorCode::ObInvalidRequest,
            format!("unknown tool: {}", call.name),
            None,
        ),
    }
}
