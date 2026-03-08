use openbrain_core::{ErrorEnvelope, LifecycleState};
use openbrain_store::{
    EmbeddingCoverageRequest, EmbeddingCoverageResponse, EmbeddingReembedRequest,
    EmbeddingReembedResponse, PgStore,
};
use std::fmt::Write as _;

pub fn parse_lifecycle_state(raw: &str) -> Result<LifecycleState, String> {
    use std::str::FromStr;
    LifecycleState::from_str(raw).map_err(|_| {
        format!("invalid --state value: {raw} (expected scratch|candidate|accepted|deprecated)")
    })
}

pub async fn run_embed_coverage(
    store: &PgStore,
    req: EmbeddingCoverageRequest,
) -> Result<String, ErrorEnvelope> {
    let data = store.embedding_coverage(req).await?;
    Ok(format_coverage(data))
}

pub async fn run_embed_reembed(
    store: &PgStore,
    req: EmbeddingReembedRequest,
) -> Result<String, ErrorEnvelope> {
    let data = store.reembed_missing(req).await?;
    Ok(format_reembed(data))
}

pub fn format_coverage(data: EmbeddingCoverageResponse) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "total_eligible: {}", data.total_eligible);
    let _ = writeln!(out, "with_embeddings: {}", data.with_embeddings);
    let _ = writeln!(out, "missing: {}", data.missing);
    let _ = writeln!(out, "percent_coverage: {:.2}", data.percent_coverage);
    if !data.missing_refs.is_empty() {
        let _ = writeln!(out, "missing_refs: {}", data.missing_refs.join(","));
    }
    out
}

pub fn format_reembed(data: EmbeddingReembedResponse) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "dry_run: {}", data.dry_run);
    let _ = writeln!(out, "scanned: {}", data.scanned);
    let _ = writeln!(out, "processed: {}", data.processed);
    let _ = writeln!(out, "bytes_processed: {}", data.bytes_processed);
    if let Some(cursor) = data.next_cursor {
        let _ = writeln!(out, "next_cursor: {}", cursor);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_accepts_accepted() {
        assert_eq!(
            parse_lifecycle_state("accepted").expect("state"),
            LifecycleState::Accepted
        );
    }

    #[test]
    fn parse_state_rejects_unknown() {
        let err = parse_lifecycle_state("active").expect_err("must fail");
        assert!(err.contains("invalid --state value"));
    }

    #[test]
    fn format_coverage_prints_percent() {
        let out = format_coverage(EmbeddingCoverageResponse {
            total_eligible: 10,
            with_embeddings: 7,
            missing: 3,
            percent_coverage: 70.0,
            missing_refs: vec!["a".to_string()],
        });
        assert!(out.contains("percent_coverage: 70.00"));
        assert!(out.contains("missing_refs: a"));
    }
}
