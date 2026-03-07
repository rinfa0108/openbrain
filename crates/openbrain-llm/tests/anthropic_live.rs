use openbrain_llm::AnthropicClient;

#[tokio::test]
async fn anthropic_live_rerank_is_json() {
    let run_live = std::env::var("RUN_ANTHROPIC_LIVE_TESTS").ok().as_deref() == Some("1");
    if !run_live {
        eprintln!("skipping live Anthropic test: set RUN_ANTHROPIC_LIVE_TESTS=1");
        return;
    }

    let has_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if !has_key {
        eprintln!("skipping live Anthropic test: set ANTHROPIC_API_KEY");
        return;
    }

    let client = AnthropicClient::from_env();
    let prompt = "Return JSON: {\"ranked_refs\":[\"a\"],\"rationale_short\":[{\"ref\":\"a\",\"why\":\"test\"}]}";
    let out = client.complete_json(prompt).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&out.text).unwrap();
    assert!(v.get("ranked_refs").is_some());
}
