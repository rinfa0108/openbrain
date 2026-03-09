use clap::{Parser, Subcommand};

mod embed_cli;
mod governance_cli;
mod mcp;
mod shadow_cli;

use openbrain_embed::{
    EmbeddingProvider, FakeEmbeddingProvider, LocalHttpEmbeddingProvider, NoopEmbeddingProvider,
    OpenAIEmbeddingProvider,
};
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, policy, AppState};
use openbrain_store::{AuthStore, PgStore, PutObjectsRequest, Store};
use std::{io::Read as _, net::SocketAddr, sync::Arc};
use tracing::{info, warn};

#[derive(Debug, Parser)]
#[command(name = "openbrain")]
#[command(version = openbrain_core::SPEC_VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the localhost HTTP daemon
    Serve {
        #[arg(long, env = "OPENBRAIN_PORT", default_value_t = 7981)]
        port: u16,

        #[arg(long, env = "OPENBRAIN_BIND", default_value = "127.0.0.1")]
        bind: String,

        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,

        /// Embedding provider selection: "noop" (default), "fake" (dev/testing only), "openai", or "local"
        #[arg(long, env = "OPENBRAIN_EMBED_PROVIDER", default_value = "noop")]
        embed_provider: String,
    },

    /// Start an MCP server over stdio
    Mcp {
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,

        /// Embedding provider selection: "noop" (default), "fake" (dev/testing only), "openai", or "local"
        #[arg(long, env = "OPENBRAIN_EMBED_PROVIDER", default_value = "noop")]
        embed_provider: String,
    },

    /// Workspace inspection commands
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },

    /// Audit timeline inspection commands
    Audit {
        #[command(subcommand)]
        command: AuditCommand,
    },

    /// Retention policy inspection commands
    Retention {
        #[command(subcommand)]
        command: RetentionCommand,
    },

    /// Embedding operator commands
    Embed {
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,

        /// Auth token for workspace-scoped coverage/re-embed authorization checks
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,

        #[command(subcommand)]
        command: EmbedCommand,
    },

    /// Shadow mode extraction from transcript text/json (dry-run by default)
    Shadow {
        #[arg(long, env = "DATABASE_URL")]
        database_url: Option<String>,
        #[arg(long = "workspace", alias = "scope", default_value = "default")]
        workspace: String,
        /// Required for --mode write-scratch
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: Option<String>,
        #[arg(long, value_parser = shadow_cli::parse_mode, default_value = "dry-run")]
        mode: shadow_cli::ShadowMode,
        #[arg(long)]
        input: Option<String>,
        #[arg(long, default_value_t = false)]
        stdin: bool,
        #[arg(long, value_parser = shadow_cli::parse_input_format, default_value = "text")]
        format: shadow_cli::ShadowInputFormat,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        out: Option<String>,
        #[arg(long = "out-html")]
        out_html: Option<String>,
        #[arg(long)]
        actor: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum EmbedCommand {
    /// Report embedding coverage for a workspace + provider/model/kind
    Coverage {
        #[arg(long = "workspace", alias = "scope")]
        workspace: String,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "semantic")]
        kind: String,
        #[arg(long, value_parser = embed_cli::parse_lifecycle_state, default_value = "accepted")]
        state: openbrain_core::LifecycleState,
        #[arg(long, default_value_t = 10)]
        missing_sample: u32,
    },
    /// Re-embed missing objects into target provider/model/kind
    Reembed {
        #[arg(long = "workspace", alias = "scope")]
        workspace: String,
        #[arg(long = "to-provider")]
        to_provider: String,
        #[arg(long = "to-model")]
        to_model: String,
        #[arg(long = "to-kind", default_value = "semantic")]
        to_kind: String,
        #[arg(long, value_parser = embed_cli::parse_lifecycle_state, default_value = "accepted")]
        state: openbrain_core::LifecycleState,
        #[arg(long, default_value_t = 100)]
        limit: u32,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        max_bytes: Option<u64>,
        #[arg(long)]
        max_objects: Option<u32>,
        #[arg(long)]
        actor: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    /// Show workspace ownership and caller role
    Info {
        #[arg(
            long,
            env = "OPENBRAIN_BASE_URL",
            default_value = "http://127.0.0.1:7981"
        )]
        base_url: String,
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,
    },
}

#[derive(Debug, Subcommand)]
enum RetentionCommand {
    /// Show effective policy.retention for a scope
    Show {
        #[arg(
            long,
            env = "OPENBRAIN_BASE_URL",
            default_value = "http://127.0.0.1:7981"
        )]
        base_url: String,
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,
        #[arg(long)]
        scope: String,
    },
}

#[derive(Debug, Subcommand)]
enum AuditCommand {
    /// Timeline of events for a specific object id
    Object {
        object_id: String,
        #[arg(
            long,
            env = "OPENBRAIN_BASE_URL",
            default_value = "http://127.0.0.1:7981"
        )]
        base_url: String,
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Timeline of events for a memory key
    Key {
        memory_key: String,
        #[arg(
            long,
            env = "OPENBRAIN_BASE_URL",
            default_value = "http://127.0.0.1:7981"
        )]
        base_url: String,
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Timeline of events for an actor identity
    Actor {
        actor_identity_id: String,
        #[arg(
            long,
            env = "OPENBRAIN_BASE_URL",
            default_value = "http://127.0.0.1:7981"
        )]
        base_url: String,
        #[arg(long, env = "OPENBRAIN_TOKEN")]
        token: String,
        #[arg(long)]
        scope: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn select_embedder(name: &str) -> Arc<dyn EmbeddingProvider> {
    match name.trim().to_ascii_lowercase().as_str() {
        "fake" => {
            warn!("using FakeEmbeddingProvider (dev/testing only)");
            Arc::new(FakeEmbeddingProvider)
        }
        "openai" => Arc::new(OpenAIEmbeddingProvider::from_env()),
        "local" => Arc::new(LocalHttpEmbeddingProvider::from_env()),
        _ => Arc::new(NoopEmbeddingProvider),
    }
}

async fn connect_store(database_url: Option<String>, embed_provider: String) -> PgStore {
    let database_url = database_url.unwrap_or_else(|| {
        eprintln!("DATABASE_URL is required (set env DATABASE_URL)");
        std::process::exit(2);
    });

    let embedder = select_embedder(&embed_provider);

    match PgStore::connect_with_embedder_and_provider(&database_url, embedder, &embed_provider)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to connect to postgres: {e}");
            std::process::exit(2);
        }
    }
}

async fn bootstrap_default_workspace(store: &PgStore) {
    match store.bootstrap_default_workspace().await {
        Ok(Some(token)) => {
            println!(
                "bootstrap owner token (workspace={}): {}",
                token.workspace_id, token.token
            );
        }
        Ok(None) => {}
        Err(e) => {
            eprintln!("bootstrap failed: {} ({})", e.message, e.code);
        }
    }
}

#[tokio::main]
async fn main() {
    init_tracing();

    let cli = Cli::parse();

    match cli.command {
        Command::Serve {
            port,
            bind,
            database_url,
            embed_provider,
        } => {
            let store = connect_store(database_url, embed_provider).await;
            bootstrap_default_workspace(&store).await;

            let addr: SocketAddr = format!("{}:{}", bind, port).parse().unwrap_or_else(|_| {
                eprintln!("invalid bind/port: {bind}:{port}");
                std::process::exit(2);
            });

            let llm = AnthropicClient::from_env();
            let app = build_router(AppState { store, llm });

            info!("OpenBrain server (spec v{})", openbrain_core::SPEC_VERSION);
            info!("listening on http://{}", addr);

            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("failed to bind {addr}: {e}");
                    std::process::exit(2);
                });

            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .unwrap_or_else(|e| {
                    eprintln!("server error: {e}");
                    std::process::exit(1);
                });
        }

        Command::Mcp {
            database_url,
            embed_provider,
        } => {
            let store = connect_store(database_url, embed_provider).await;
            bootstrap_default_workspace(&store).await;

            info!(
                "OpenBrain MCP stdio (spec v{})",
                openbrain_core::SPEC_VERSION
            );

            let llm = AnthropicClient::from_env();
            if let Err(e) = mcp::run_mcp_stdio(store, llm).await {
                eprintln!("mcp error: {e}");
                std::process::exit(1);
            }
        }
        Command::Workspace { command } => {
            if let Err(e) = run_workspace_command(command).await {
                eprintln!("{}", e.user_message());
                std::process::exit(1);
            }
        }
        Command::Audit { command } => {
            if let Err(e) = run_audit_command(command).await {
                eprintln!("{}", e.user_message());
                std::process::exit(1);
            }
        }
        Command::Retention { command } => {
            if let Err(e) = run_retention_command(command).await {
                eprintln!("{}", e.user_message());
                std::process::exit(1);
            }
        }
        Command::Embed {
            database_url,
            token,
            command,
        } => {
            let selected_provider = match &command {
                EmbedCommand::Reembed { to_provider, .. } => to_provider.clone(),
                EmbedCommand::Coverage { .. } => "noop".to_string(),
            };
            let store = connect_store(database_url, selected_provider).await;
            if let Err(e) = run_embed_command(&store, &token, command).await {
                eprintln!("{}", e.user_message());
                std::process::exit(1);
            }
        }
        Command::Shadow {
            database_url,
            workspace,
            token,
            mode,
            input,
            stdin,
            format,
            limit,
            out,
            out_html,
            actor,
        } => {
            if let Err(e) = run_shadow_command(ShadowCommandArgs {
                database_url,
                workspace,
                token,
                mode,
                input,
                stdin,
                input_format: format,
                limit,
                out,
                out_html,
                actor,
            })
            .await
            {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}

async fn run_workspace_command(command: WorkspaceCommand) -> Result<(), governance_cli::CliError> {
    match command {
        WorkspaceCommand::Info { base_url, token } => {
            let transport = governance_cli::ReqwestTransport::new(base_url.clone())?;
            let args = governance_cli::HttpArgs {
                token,
                scope: None,
                from: None,
                to: None,
                limit: None,
            };
            let out = governance_cli::run_workspace_info(&transport, &args).await?;
            print!("{out}");
        }
    }
    Ok(())
}

async fn run_audit_command(command: AuditCommand) -> Result<(), governance_cli::CliError> {
    match command {
        AuditCommand::Object {
            object_id,
            base_url,
            token,
            scope,
            from,
            to,
            limit,
        } => {
            let transport = governance_cli::ReqwestTransport::new(base_url.clone())?;
            let args = governance_cli::HttpArgs {
                token,
                scope: Some(scope),
                from,
                to,
                limit,
            };
            let out = governance_cli::run_audit_object(&transport, &args, object_id).await?;
            print!("{out}");
        }
        AuditCommand::Key {
            memory_key,
            base_url,
            token,
            scope,
            from,
            to,
            limit,
        } => {
            let transport = governance_cli::ReqwestTransport::new(base_url.clone())?;
            let args = governance_cli::HttpArgs {
                token,
                scope: Some(scope),
                from,
                to,
                limit,
            };
            let out = governance_cli::run_audit_key(&transport, &args, memory_key).await?;
            print!("{out}");
        }
        AuditCommand::Actor {
            actor_identity_id,
            base_url,
            token,
            scope,
            from,
            to,
            limit,
        } => {
            let transport = governance_cli::ReqwestTransport::new(base_url.clone())?;
            let args = governance_cli::HttpArgs {
                token,
                scope: Some(scope),
                from,
                to,
                limit,
            };
            let out = governance_cli::run_audit_actor(&transport, &args, actor_identity_id).await?;
            print!("{out}");
        }
    }
    Ok(())
}

async fn run_retention_command(command: RetentionCommand) -> Result<(), governance_cli::CliError> {
    match command {
        RetentionCommand::Show {
            base_url,
            token,
            scope,
        } => {
            let transport = governance_cli::ReqwestTransport::new(base_url.clone())?;
            let args = governance_cli::HttpArgs {
                token,
                scope: Some(scope),
                from: None,
                to: None,
                limit: None,
            };
            let out = governance_cli::run_retention_show(&transport, &args).await?;
            print!("{out}");
        }
    }
    Ok(())
}

#[derive(Debug)]
enum EmbedCommandError {
    Api(openbrain_core::ErrorEnvelope),
}

impl EmbedCommandError {
    fn user_message(&self) -> String {
        match self {
            Self::Api(err) => format!("{}: {}", err.code, err.message),
        }
    }
}

async fn run_embed_command(
    store: &PgStore,
    token: &str,
    command: EmbedCommand,
) -> Result<(), EmbedCommandError> {
    match command {
        EmbedCommand::Coverage {
            workspace,
            provider,
            model,
            kind,
            state,
            missing_sample,
        } => {
            let auth = store
                .auth_from_token(token)
                .await
                .map_err(EmbedCommandError::Api)?;
            if auth.workspace_id != workspace {
                return Err(EmbedCommandError::Api(openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObForbidden,
                    "token does not grant access to requested workspace",
                    None,
                )));
            }
            if !auth.role.can_read() {
                return Err(EmbedCommandError::Api(openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObForbidden,
                    "coverage requires read permission",
                    None,
                )));
            }
            let output = embed_cli::run_embed_coverage(
                store,
                openbrain_store::EmbeddingCoverageRequest {
                    scope: workspace,
                    provider,
                    model,
                    kind,
                    state,
                    missing_sample_limit: Some(missing_sample),
                },
            )
            .await
            .map_err(EmbedCommandError::Api)?;
            print!("{output}");
        }
        EmbedCommand::Reembed {
            workspace,
            to_provider,
            to_model,
            to_kind,
            state,
            limit,
            cursor,
            dry_run,
            max_bytes,
            max_objects,
            actor,
        } => {
            let auth = store
                .auth_from_token(token)
                .await
                .map_err(EmbedCommandError::Api)?;
            if auth.workspace_id != workspace {
                return Err(EmbedCommandError::Api(openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObForbidden,
                    "token does not grant access to requested workspace",
                    None,
                )));
            }
            if !auth.role.can_write() {
                return Err(EmbedCommandError::Api(openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObForbidden,
                    "re-embed requires writer or owner role",
                    None,
                )));
            }

            // Preflight policy check to avoid bypassing kind-based embed restrictions in CLI path.
            let rules = policy::load_workspace_policies(store, &workspace)
                .await
                .map_err(EmbedCommandError::Api)?;
            let scope_for_query = workspace.as_str();
            let now = chrono::Utc::now();
            let rows: Vec<(String, Option<String>)> = sqlx::query_as(
                r#"SELECT DISTINCT o.type, o.memory_key
                   FROM ob_objects o
                   WHERE o.scope = $1
                     AND o.lifecycle_state = $2
                     AND (o.expires_at IS NULL OR o.expires_at > $3)
                     AND ($4::text IS NULL OR o.id > $4)
                     AND NOT EXISTS (
                       SELECT 1 FROM ob_embeddings e
                       WHERE e.scope = o.scope
                         AND e.object_id = o.id
                         AND e.provider = $5
                         AND e.model = $6
                         AND e.kind = $7
                     )
                   ORDER BY o.type ASC
                   LIMIT $8"#,
            )
            .bind(scope_for_query)
            .bind(state.as_str())
            .bind(now)
            .bind(cursor.as_deref())
            .bind(to_provider.as_str())
            .bind(to_model.as_str())
            .bind(to_kind.as_str())
            .bind(limit as i64)
            .fetch_all(store.pool())
            .await
            .map_err(|e| {
                EmbedCommandError::Api(openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObStorageError,
                    format!("re-embed policy preflight query failed: {e}"),
                    None,
                ))
            })?;

            for (object_kind, memory_key) in rows {
                let decision = policy::evaluate(
                    &rules,
                    &policy::EvalInput {
                        role: auth.role,
                        identity_id: &auth.identity_id,
                        operation: policy::PolicyOperation::EmbedGenerate,
                        object_kind: Some(object_kind.as_str()),
                        memory_key: memory_key.as_deref(),
                        lifecycle_transition: None,
                    },
                );
                if !decision.allowed {
                    return Err(EmbedCommandError::Api(policy::deny_error_with_rule(
                        decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                        decision.policy_rule_id.as_deref(),
                        None,
                    )));
                }
            }

            let output = embed_cli::run_embed_reembed(
                store,
                openbrain_store::EmbeddingReembedRequest {
                    scope: workspace,
                    to_provider,
                    to_model,
                    to_kind,
                    state,
                    limit: Some(limit),
                    after: cursor,
                    dry_run,
                    max_bytes,
                    max_objects,
                    actor,
                },
            )
            .await
            .map_err(EmbedCommandError::Api)?;
            print!("{output}");
        }
    }
    Ok(())
}

fn load_shadow_input(
    input: Option<String>,
    use_stdin: bool,
) -> Result<String, openbrain_core::ErrorEnvelope> {
    if let Some(path) = input {
        return std::fs::read_to_string(&path).map_err(|e| {
            openbrain_core::ErrorEnvelope::new(
                openbrain_core::ErrorCode::ObInvalidRequest,
                format!("failed to read --input file: {e}"),
                Some(serde_json::json!({"path": path})),
            )
        });
    }
    if use_stdin {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            openbrain_core::ErrorEnvelope::new(
                openbrain_core::ErrorCode::ObInvalidRequest,
                format!("failed to read --stdin: {e}"),
                None,
            )
        })?;
        return Ok(buf);
    }
    Err(openbrain_core::ErrorEnvelope::new(
        openbrain_core::ErrorCode::ObInvalidRequest,
        "provide either --input <file> or --stdin",
        None,
    ))
}

async fn run_shadow_command(args: ShadowCommandArgs) -> Result<(), String> {
    let raw =
        load_shadow_input(args.input, args.stdin).map_err(|e| shadow_cli::format_user_error(&e))?;
    let candidates = shadow_cli::extract_candidates(&raw, args.input_format, args.limit as usize)
        .map_err(|e| shadow_cli::format_user_error(&e))?;
    let mut objects = shadow_cli::candidates_to_objects(&args.workspace, &candidates);

    let written_refs = match args.mode {
        shadow_cli::ShadowMode::DryRun => Vec::new(),
        shadow_cli::ShadowMode::WriteScratch => {
            let token = args.token.as_ref().ok_or_else(|| {
                shadow_cli::format_user_error(&openbrain_core::ErrorEnvelope::new(
                    openbrain_core::ErrorCode::ObInvalidRequest,
                    "--token is required in --mode write-scratch",
                    None,
                ))
            })?;
            let store = connect_store(args.database_url, "noop".to_string()).await;
            let auth = store
                .auth_from_token(token)
                .await
                .map_err(|e| shadow_cli::format_user_error(&e))?;
            if auth.workspace_id != args.workspace {
                return Err(shadow_cli::format_user_error(
                    &openbrain_core::ErrorEnvelope::new(
                        openbrain_core::ErrorCode::ObForbidden,
                        "token does not grant access to requested workspace",
                        None,
                    ),
                ));
            }
            if !auth.role.can_write() {
                return Err(shadow_cli::format_user_error(
                    &openbrain_core::ErrorEnvelope::new(
                        openbrain_core::ErrorCode::ObForbidden,
                        "shadow write requires writer or owner role",
                        None,
                    ),
                ));
            }
            let rules = policy::load_workspace_policies(&store, &args.workspace)
                .await
                .map_err(|e| shadow_cli::format_user_error(&e))?;
            for obj in &objects {
                let decision = policy::evaluate(
                    &rules,
                    &policy::EvalInput {
                        role: auth.role,
                        identity_id: &auth.identity_id,
                        operation: policy::PolicyOperation::Write,
                        object_kind: obj.object_type.as_deref(),
                        memory_key: obj.memory_key.as_deref(),
                        lifecycle_transition: None,
                    },
                );
                if !decision.allowed {
                    return Err(shadow_cli::format_user_error(
                        &policy::deny_error_with_rule(
                            decision.reason_code.as_deref().unwrap_or("OB_POLICY_DENY"),
                            decision.policy_rule_id.as_deref(),
                            None,
                        ),
                    ));
                }
            }
            let retention = policy::load_workspace_retention_policy(&store, &args.workspace)
                .await
                .map_err(|e| shadow_cli::format_user_error(&e))?;
            policy::apply_retention_policy_to_objects(
                retention.as_ref(),
                &mut objects,
                chrono::Utc::now(),
            )
            .map_err(|e| shadow_cli::format_user_error(&e))?;
            let resp = store
                .put_objects(PutObjectsRequest {
                    objects: objects.clone(),
                    actor: args.actor.clone(),
                    idempotency_key: None,
                })
                .await;
            let results = match resp {
                openbrain_core::Envelope::Ok { data, .. } => data.results,
                openbrain_core::Envelope::Err { error, .. } => {
                    return Err(shadow_cli::format_user_error(&error));
                }
            };
            store
                .append_event(
                    &args.workspace,
                    "shadow.batch",
                    args.actor.as_deref().unwrap_or(&auth.identity_id),
                    serde_json::json!({
                        "mode": "write-scratch",
                        "count": results.len(),
                        "kinds": candidates.iter().map(|c| c.kind.clone()).collect::<Vec<_>>(),
                    }),
                )
                .await;
            results.into_iter().map(|r| r.r#ref).collect()
        }
    };

    let report = shadow_cli::ShadowReport {
        mode: match args.mode {
            shadow_cli::ShadowMode::DryRun => "dry-run".to_string(),
            shadow_cli::ShadowMode::WriteScratch => "write-scratch".to_string(),
        },
        workspace: args.workspace.clone(),
        extracted_count: candidates.len(),
        written_count: written_refs.len(),
        extracted: candidates.clone(),
        written_refs: written_refs.clone(),
    };

    match args.input_format {
        shadow_cli::ShadowInputFormat::Json => {
            let rendered = serde_json::to_string_pretty(&report)
                .map_err(|e| format!("failed to render report json: {e}"))?;
            println!("{rendered}");
        }
        shadow_cli::ShadowInputFormat::Text => {
            println!("{}", shadow_cli::format_candidates_text(&candidates));
            println!("mode: {}", report.mode);
            println!("workspace: {}", report.workspace);
            println!("extracted_count: {}", report.extracted_count);
            println!("written_count: {}", report.written_count);
            if !report.written_refs.is_empty() {
                println!("written_refs: {}", report.written_refs.join(","));
            }
        }
    }

    if let Some(path) = args.out {
        let rendered = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("failed to render report json: {e}"))?;
        std::fs::write(&path, rendered).map_err(|e| format!("failed to write --out file: {e}"))?;
        println!("report_json: {path}");
    }
    if let Some(path) = args.out_html {
        let html = shadow_cli::render_html_report(&report);
        std::fs::write(&path, html).map_err(|e| format!("failed to write --out-html file: {e}"))?;
        println!("report_html: {path}");
    }

    Ok(())
}

struct ShadowCommandArgs {
    database_url: Option<String>,
    workspace: String,
    token: Option<String>,
    mode: shadow_cli::ShadowMode,
    input: Option<String>,
    stdin: bool,
    input_format: shadow_cli::ShadowInputFormat,
    limit: u32,
    out: Option<String>,
    out_html: Option<String>,
    actor: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_workspace_info_command() {
        let cli = Cli::try_parse_from(["openbrain", "workspace", "info", "--token", "tok"])
            .expect("parse");
        assert!(matches!(
            cli.command,
            Command::Workspace {
                command: WorkspaceCommand::Info { .. }
            }
        ));
    }

    #[test]
    fn parses_audit_object_command() {
        let cli = Cli::try_parse_from([
            "openbrain",
            "audit",
            "object",
            "obj-1",
            "--token",
            "tok",
            "--scope",
            "ws-default",
            "--limit",
            "10",
        ])
        .expect("parse");

        match cli.command {
            Command::Audit { command } => match command {
                AuditCommand::Object {
                    object_id,
                    scope,
                    limit,
                    ..
                } => {
                    assert_eq!(object_id, "obj-1");
                    assert_eq!(scope, "ws-default");
                    assert_eq!(limit, Some(10));
                }
                _ => panic!("wrong audit command"),
            },
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_embed_coverage_command() {
        let cli = Cli::try_parse_from([
            "openbrain",
            "embed",
            "--token",
            "ob_demo",
            "coverage",
            "--workspace",
            "ws-default",
            "--provider",
            "fake",
            "--model",
            "fake-v1",
        ])
        .expect("parse");

        assert!(matches!(
            cli.command,
            Command::Embed {
                command: EmbedCommand::Coverage { .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_embed_reembed_command() {
        let cli = Cli::try_parse_from([
            "openbrain",
            "embed",
            "--token",
            "ob_demo",
            "reembed",
            "--workspace",
            "ws-default",
            "--to-provider",
            "fake",
            "--to-model",
            "fake-v1",
            "--limit",
            "25",
            "--dry-run",
        ])
        .expect("parse");

        assert!(matches!(
            cli.command,
            Command::Embed {
                command: EmbedCommand::Reembed { .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_shadow_dry_run_command() {
        let cli = Cli::try_parse_from([
            "openbrain",
            "shadow",
            "--workspace",
            "ws-default",
            "--stdin",
            "--mode",
            "dry-run",
        ])
        .expect("parse");
        assert!(matches!(cli.command, Command::Shadow { .. }));
    }
}
