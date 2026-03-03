use openbrain_embed::{EmbeddingProvider, OpenAIEmbeddingProvider};

#[tokio::test]
async fn openai_live_embedding_is_1536_dims() {
    let run_live = std::env::var("RUN_OPENAI_LIVE_TESTS").ok().as_deref() == Some("1");
    let has_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if !run_live || !has_key {
        eprintln!("skipping live OpenAI test: set RUN_OPENAI_LIVE_TESTS=1 and OPENAI_API_KEY");
        return;
    }

    let provider = OpenAIEmbeddingProvider::from_env();
    let embedding = provider
        .embed("default", "hello from openbrain")
        .await
        .unwrap();
    assert_eq!(embedding.len(), 1536);
}
