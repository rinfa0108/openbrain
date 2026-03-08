# OpenBrain Enterprise Architecture & Integration Specification

Status: FINAL

## Executive summary
OpenBrain is a provider-agnostic state plane for AI systems. It separates stateless model inference from durable, governed memory so teams can scale retrieval quality and governance without coupling to one model vendor.

## Reference architecture
### Trust boundaries
- Workspace is the tenant boundary.
- Identity + role are authenticated per request.
- Policy decisions execute before data access.
- Event history is append-only and workspace-scoped.

### Deployment topology
- Local evaluation: Docker Compose with Postgres + pgvector + OpenBrain server.
- Enterprise deployment: OpenBrain service with managed Postgres and platform-managed network/secret controls.

## Governed retrieval at scale
Small-context agents retrieve a governed memory pack from a much larger durable store. Retrieval is constrained by workspace boundaries, lifecycle defaults, retention boundaries, and policy checks before results are returned. This keeps context windows small while preserving auditability and deterministic behavior at scale.

## Interface contracts
### MCP contract groups
- Core memory operations: ping, write, read, structured search, embed generate, semantic search.
- Governance operations: workspace info and audit timelines.
- Optional utility operations: rerank and memory pack.

### HTTP contract groups
- Core `/v1/*` memory endpoints.
- Governance `/v1/workspace/info` and `/v1/audit/*` endpoints.
- SDK mirror behavior for TypeScript and Python clients.

### SDK usage patterns
- TypeScript and Python SDKs support HTTP client and MCP helper patterns.
- Service integrations typically use HTTP; agent runtimes use MCP.

## Security and governance model
- Workspace isolation with RBAC roles (`owner`, `writer`, `reader`).
- Fine-grained policy rules as workspace data (`policy.rule`).
- Deterministic denial payloads include `reason_code` and `policy_rule_id`.
- Token hashes only; no plaintext token persistence.

## Compliance, lifecycle, and retention
- Default retrieval: `accepted` + not expired.
- Lifecycle states govern retrieval eligibility and promotion flow.
- `policy.retention` defines workspace retention boundaries:
  - `default_ttl_by_kind`
  - `max_ttl_by_kind`
  - `immutable_kinds`
- Audit APIs provide bounded timelines for object, key, and actor investigations.

## Embedding migration workflow
Multi-embedding support allows parallel embedding spaces by provider/model/kind. Operators can migrate retrieval safely with:

1. Coverage measurement for target embedding space.
2. Dry-run re-embed planning.
3. Bounded re-embed execution with batch and resume controls.
4. Post-run coverage verification.

This avoids semantic blind spots when switching providers or models.

## Web Viewer
The `/viewer` surface is a read-only, localhost-first governance console.

- Token is pasted by operator and sent as `Authorization: Bearer <token>`.
- No write/update operations are exposed.
- Viewer calls existing APIs only (`/v1/workspace/info`, `/v1/audit/*`, `/v1/read`, `/v1/search/structured`).
- Forbidden responses surface `reason_code` and `policy_rule_id`.

Reference screenshots:

- `docs/pack/assets/viewer/viewer-02-workspace-info.png`
- `docs/pack/assets/viewer/viewer-03-audit-object-timeline.png`
- `docs/pack/assets/viewer/viewer-06-retention-policy.png`
- `docs/pack/assets/viewer/viewer-08-deny-explainability.png`

## Operational requirements
- Deterministic local gates: fmt, clippy, tests, cargo-deny, gitleaks.
- Backup/restore aligned with standard Postgres operations.
- Scale posture: monitor Postgres storage/index health and semantic query profile.

## Integration patterns
- Agent runtime pattern: MCP tools with governance enforcement.
- Application pattern: HTTP + SDKs for backend workflows.
- Operations pattern: CLI + viewer for inspection, audit APIs for investigation.

## Risks, assumptions, and mitigations
- Risk: policy misconfiguration blocks expected flows.
  - Mitigation: deterministic deny explainability plus audit timelines.
- Risk: retention settings too permissive or too strict.
  - Mitigation: versioned `policy.retention` objects with reviewable event history.
- Assumption: PostgreSQL with pgvector is available in target environments.
  - Mitigation: Compose path for local parity plus explicit enterprise deployment guidance.
