# Changelog

## v1.0.0 - 2026-03-09

OpenBrain reaches v1.0 with governed durable memory as a stable platform surface.

### Storage and memory model
- Typed object storage with versioned updates.
- Append-only event model for immutable audit trail.
- Workspace-scoped isolation for memory and governance.

### Governance and control plane
- Workspace RBAC with token auth across HTTP and MCP.
- Policy engine with deterministic deny explainability (`reason_code`, `policy_rule_id`).
- Lifecycle controls with default retrieval (`accepted` + non-expired), TTL, and conflict metadata.
- Retention boundaries via `policy.retention` policy-as-data.
- Audit timeline queries by object, memory key, and actor.

### Retrieval and embeddings
- Structured search and semantic search with embedding space selectors (`provider`, `model`, `kind`).
- Re-embed and coverage tooling for deterministic embedding migration.

### Memory packs
- Deterministic memory pack assembly with strict budgeting (`ceil(chars/4)`).
- Conflict-aware surfacing, dedupe, and governed filtering.
- Deterministic hybrid ranking and diversity controls.

### Tooling and adoption
- MCP and HTTP parity on stable surfaces.
- TypeScript and Python SDKs (HTTP + MCP helpers).
- Read-only localhost web viewer at `/viewer`.
- Compose-first onboarding and deterministic demo kits.
- Shadow mode adoption wedge for transcript-to-scratch extraction.

### Security and quality
- Deterministic local quality gates (`fmt`, `clippy`, tests, `cargo-deny`, `gitleaks`).
- Live-provider tests remain explicit opt-in via `RUN_*_LIVE_TESTS=1`.
- Trademark/branding policy docs added without license changes.

### Migration notes
- v1.0.0 keeps existing stable `/v1/*` routes and MCP tool names.
- Contract evolution to date is additive; no endpoint/tool renames required for v0.1 users.

### Known limitations
- `sqlx-postgres` emits future-incompat warnings in current toolchain output; tracked for follow-up dependency hygiene.
- Local viewer is intentionally read-only and localhost-first.