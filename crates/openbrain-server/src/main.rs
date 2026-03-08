use clap::{Parser, Subcommand};

mod embed_cli;
mod governance_cli;
mod mcp;

use openbrain_embed::{
    EmbeddingProvider, FakeEmbeddingProvider, LocalHttpEmbeddingProvider, NoopEmbeddingProvider,
    OpenAIEmbeddingProvider,
};
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::{AuthStore, PgStore};
use std::{net::SocketAddr, sync::Arc};
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

        /// Embedding provider used for generation during re-embed (fake|noop|openai|local)
        #[arg(long, env = "OPENBRAIN_EMBED_PROVIDER", default_value = "fake")]
        embed_provider: String,

        #[command(subcommand)]
        command: EmbedCommand,
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
            embed_provider,
            command,
        } => {
            let store = connect_store(database_url, embed_provider.clone()).await;
            if let Err(e) = run_embed_command(&store, &embed_provider, command).await {
                eprintln!("{}", e.user_message());
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
    Usage(String),
    Api(openbrain_core::ErrorEnvelope),
}

impl EmbedCommandError {
    fn user_message(&self) -> String {
        match self {
            Self::Usage(msg) => msg.clone(),
            Self::Api(err) => format!("{}: {}", err.code, err.message),
        }
    }
}

async fn run_embed_command(
    store: &PgStore,
    embed_provider: &str,
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
            if !to_provider.eq_ignore_ascii_case(embed_provider) {
                return Err(EmbedCommandError::Usage(format!(
                    "--to-provider ({to_provider}) must match --embed-provider ({embed_provider}) for deterministic re-embed execution"
                )));
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
            "--embed-provider",
            "fake",
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
}
