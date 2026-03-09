# OpenBrain v1.0.0 - Provider-Agnostic Structured Memory for AI Agents

OpenBrain v1.0.0 formalizes the state-plane model for agent systems: OpenBrain is durable, governed memory; LLMs are stateless compute adapters. This release delivers deterministic, policy-filtered retrieval and budgeted memory packs so small-context agents can operate safely over large memory stores.

## Highlights

- Typed memory core with objects, immutable events, and versioned updates.
- Workspace isolation with RBAC, fine-grained policy controls, and deterministic deny explainability.
- Lifecycle governance with default accepted-only retrieval, TTL handling, and conflict detection/resolution metadata.
- Multi-embedding retrieval across provider/model/kind, plus operator-controlled coverage and re-embed workflows.
- Deterministic memory packs with strict budgeting (`ceil(chars/4)`), dedupe, conflict-aware surfacing, and stable ordering.
- Practical adoption tooling: localhost read-only Web Viewer, compose demo kits, and Shadow Mode extraction wedge.
- SDK support in TypeScript and Python for both HTTP and MCP integration paths.
- Enforced local quality/security gates including rustfmt, clippy, tests, cargo-deny, and gitleaks.

## What is included

### Memory plane core

- Typed object storage with append-only event history.
- Version-aware updates and reproducible state transitions.
- Workspace-scoped state boundaries for multi-tenant/team operation.

### Retrieval (structured, semantic, multi-embed)

- Structured search and semantic search over shared memory.
- Embedding-space selection by `provider` / `model` / `kind`.
- Deterministic retrieval defaults with optional explicit overrides.

### Governance and auditability

- Workspace ownership and role-based access control.
- Policy engine enforcement across HTTP and MCP.
- Explainable denies with `reason_code` and `policy_rule_id`.
- Audit timeline access by object, memory key, and actor.

### Lifecycle and conflicts

- Lifecycle states (`scratch`, `candidate`, `accepted`, `deprecated`).
- Default retrieval behavior: `accepted` + not expired.
- Conflict detection via `memory_key` + value hash, with resolution metadata.

### Memory packs

- Stable memory-pack contract (`pack.text`, `pack.items`, `budget_requested`, `budget_used`, `items_selected`, `truncated`, optional `conflict_alerts`).
- Deterministic candidate selection, ranking, and budgeting.
- Safe conflict surfacing by default, with optional detail mode.

### Tooling and onboarding

- Docker Compose local onboarding and deterministic demo scripts.
- Read-only Web Viewer at `/viewer` for governance inspection.
- Shadow Mode CLI for transcript-to-scratch memory capture.
- Re-embed and coverage commands for embedding-space migration.

### Quality and security gates

- Single local gate entrypoint via `scripts/ci/quality-gates.ps1` / `.sh`.
- Enforced checks: formatting, linting, tests, dependency policy/advisories, and leak scanning.
- Live provider tests are opt-in only.

## Compatibility guarantees (v1.0)

OpenBrain v1.0 compatibility commitments are documented in `docs/release/COMPATIBILITY.md`.

- Stable surfaces: `/v1/*` HTTP routes, MCP tools, and memory-pack contract fields listed in that document.
- SemVer policy: additive optional fields may be introduced in minor releases; breaking changes are reserved for major versions.

## Known issues and follow-ups

- Follow-up tracked: `IT13B` / `IT9C.2` to upgrade `sqlx-postgres` and remove current future-incompat warnings.

## Quickstart

```bash
# 1) Start local stack

docker compose up -d

# 2) Validate local onboarding
# See: docs/demo/local_compose_quickstart.md

# 3) Open read-only viewer
# http://127.0.0.1:8080/viewer
# See: docs/demo/web_viewer_quickstart.md

# 4) Run end-to-end memory pack demo
# See: docs/demo/memory_pack_demo_kit.md
```

Additional references:

- `docs/demo/local_compose_quickstart.md`
- `docs/demo/web_viewer_quickstart.md`
- `docs/demo/memory_pack_demo_kit.md`