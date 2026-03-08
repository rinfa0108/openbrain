# OpenBrain Feature Inventory

Status: FINAL

## Purpose
This inventory summarizes current OpenBrain capabilities for partner validation and enterprise implementation.

## Capability matrix
| Capability area | What exists now | Why it matters | Interfaces |
|---|---|---|---|
| Memory model | Typed objects with version history | Durable, structured memory instead of transient context | MCP + HTTP + SDKs |
| Event model | Append-only events | Auditability and traceability of state changes | MCP + HTTP audit views |
| Structured retrieval | DSL-based filtering and scoped reads | Deterministic retrieval for application logic | MCP `read/search.structured`, HTTP `/v1/read` and `/v1/search/structured` |
| Semantic retrieval | pgvector search with embedding space selection (`provider/model/kind`) | Safe migration and comparison across embedding spaces | MCP `search.semantic`, HTTP `/v1/search/semantic` |
| Re-embed + coverage | Coverage reporting + controlled re-embed workflow | Operator-controlled embedding migration without semantic outages | CLI `openbrain embed coverage` / `openbrain embed reembed` |
| Workspace governance | Workspace boundary + RBAC (`owner/writer/reader`) | Tenant isolation and role safety | HTTP bearer auth + MCP auth token |
| Policy engine | Workspace-scoped `policy.rule` enforcement | Fine-grained deny/allow and request constraints | HTTP + MCP parity |
| Deny explainability | `reason_code` + `policy_rule_id` on forbidden | Fast operator debugging with deterministic denial reasons | HTTP + MCP errors + viewer |
| Lifecycle controls | `scratch`, `candidate`, `accepted`, `deprecated` | Default retrieval focuses on durable memory | Read/search defaults + override knobs |
| TTL and expiry | `expires_at` with defaults and policy boundaries | Prevent memory rot and stale recall | Store-level enforcement |
| Conflict tracking | `memory_key` + `value_hash` + resolution metadata | Detect and resolve contradictory memories | Retrieval metadata + updates |
| Retention boundaries | `policy.retention` policy-as-data | Workspace-owned retention controls enforced on write | Write/update enforcement |
| Audit views | Object, memory_key, and actor timelines | Incident response and compliance evidence | `/v1/audit/*` + `openbrain.audit.*` |
| Web Viewer (`/viewer`) | Localhost read-only governance viewer with token paste | Visual validation for partners and operators | `/viewer` using existing `/v1/*` APIs |
| SDKs | TypeScript and Python SDKs (HTTP + MCP helper) | Faster integration in app stacks | `sdk/typescript/openbrain-sdk`, `sdk/python/openbrain_sdk` |
| Governance UX | CLI inspect/explain commands | Operator-grade inspection from terminal | `openbrain workspace/audit/retention` |
| Quality and security | Deterministic local gates + deny + leak checks | Repeatable quality without paid CI reliance | `scripts/ci/quality-gates.ps1` and `.sh` |

## Interface surfaces
### MCP (primary for agents)
- Core: `openbrain.ping`, `openbrain.write`, `openbrain.read`, `openbrain.search.structured`, `openbrain.embed.generate`, `openbrain.search.semantic`
- Governance: `openbrain.workspace.info`, `openbrain.audit.object_timeline`, `openbrain.audit.memory_key_timeline`, `openbrain.audit.actor_activity`
- Optional: `openbrain.rerank`, `openbrain.memory.pack`

### HTTP (mirror/debug/SDK)
- Core: `POST /v1/ping`, `POST /v1/write`, `POST /v1/read`, `POST /v1/search/structured`, `POST /v1/embed/generate`, `POST /v1/search/semantic`
- Governance: `POST /v1/workspace/info`, `POST /v1/audit/object_timeline`, `POST /v1/audit/memory_key_timeline`, `POST /v1/audit/actor_activity`
- Optional: `POST /v1/rerank`, `POST /v1/memory/pack`

## Viewer evidence assets
- `docs/pack/assets/viewer/viewer-04-audit-memory-key.png`
- `docs/pack/assets/viewer/viewer-05-audit-actor-activity.png`
- `docs/pack/assets/viewer/viewer-06-retention-policy.png`
- `docs/pack/assets/viewer/viewer-07-object-inspector.png`

## Onboarding reality
- Local onboarding is Compose-first.
- Demo scripts bootstrap tokens and governance examples.
- Token artifacts are written to `.openbrain/` and ignored by git.
- Viewer provides read-only visual validation on localhost.
