use clap::{Parser, Subcommand};

mod mcp;

use openbrain_embed::{
    EmbeddingProvider, FakeEmbeddingProvider, LocalHttpEmbeddingProvider, NoopEmbeddingProvider,
    OpenAIEmbeddingProvider,
};
use openbrain_llm::AnthropicClient;
use openbrain_server::{build_router, AppState};
use openbrain_store::PgStore;
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

    match PgStore::connect_with_embedder(&database_url, embedder).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to connect to postgres: {e}");
            std::process::exit(2);
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
    }
}
