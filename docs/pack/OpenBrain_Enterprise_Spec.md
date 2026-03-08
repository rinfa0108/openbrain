# OpenBrain Enterprise Architecture & Integration Specification

Status: DRAFT (screenshots and final polish will be added after IT11A.1)

## Executive summary
OpenBrain is a provider-agnostic state plane for AI systems. It separates stateless model inference from durable, governed memory. This enables teams to keep one shared memory substrate while switching or combining compute providers.

## Reference architecture
### Trust boundaries
- Workspace is the primary tenant boundary.
- Identity + role are authenticated per request.
- Policy engine decisions are enforced before data access.
- Audit history is append-only and workspace-scoped.

### Deployment topology
- Local evaluation: Docker Compose with Postgres + pgvector + OpenBrain server.
- Enterprise deployment: OpenBrain service with managed Postgres and existing network/secret controls.

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
- TypeScript and Python SDKs support HTTP client usage and MCP helper workflows.
- Integrations can use HTTP for services and MCP for agent runtimes.

## Security model
- Workspace isolation with RBAC roles: owner, writer, reader.
- Fine-grained policy rules as workspace data (`policy.rule`).
- Deterministic denial payloads include `reason_code` and `policy_rule_id`.
- No token plaintext storage in server persistence.

## Compliance and retention
- Lifecycle model controls retrieval eligibility (`accepted` + not expired by default).
- TTL controls support explicit expiry and policy-driven defaults.
- `policy.retention` defines workspace retention boundaries:
  - `default_ttl_by_kind`
  - `max_ttl_by_kind`
  - `immutable_kinds`
- Audit endpoints expose timeline evidence without returning secret material.

## Operational requirements
- Deterministic local quality gates: fmt, clippy, tests, cargo-deny, gitleaks.
- Backup/restore follows standard Postgres operational practices.
- Capacity planning focuses on Postgres storage/index health and semantic search query profile.

## Integration patterns
- Agent-native pattern: MCP tools for write/read/search with governance checks.
- Application pattern: HTTP + SDKs for backend services and automation.
- Governance pattern: CLI inspect commands plus audit APIs for operator workflows.

## Risks, assumptions, and mitigations
- Risk: policy misconfiguration can block expected flows.
  - Mitigation: explainability fields on deny responses and audit timelines.
- Risk: retention policies too permissive or too strict.
  - Mitigation: policy-as-data with versioned updates and reviewable event history.
- Assumption: PostgreSQL with pgvector is available in all target environments.
  - Mitigation: Compose path for local parity and explicit enterprise deployment guidance.
