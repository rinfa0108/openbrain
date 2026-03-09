# OpenBrain (v0.1) — Provider-Agnostic Structured Memory for AI Agents

OpenBrain is an open-source **machine-readable memory layer** for agentic systems.  
It stores **typed, versioned memory objects** (claims, decisions, tasks, artifacts, entities, relations, thought summaries), supports **structured queries + semantic search**, and exposes a **standard plug-in protocol via MCP** (and a **mirror HTTP API**).

**Goal:** stop context switching / context rot by moving memory out of provider silos (ChatGPT/Codex/Claude/Gemini/local models) into an agent-owned infrastructure plane.

> **NOTE (Do not edit without explicit instruction):**  
> The **header + hero lines** above and the **License/footer** at the end of this README are considered **project identity text**.  
> Please **refrain from changing** them unless explicitly instructed by the project owner.
> Contributors: Richard Infantado (richard.infantado@gmail.com), Robert Leroux (rl.isapience@gmail.com)
---

## Why OpenBrain exists

Most agent stacks still keep memory trapped inside provider-specific context windows. That makes state fragile, hard to audit, and difficult to share across models. OpenBrain exists to separate memory from model runtime: the model becomes a stateless compute adapter, while OpenBrain is the durable state plane with typed records, deterministic retrieval, and governance boundaries.

## Core concepts

OpenBrain organizes memory by workspace (scope), which is the top-level isolation boundary for data and policy. Each workspace has ownership semantics and role-based access controls.

Objects are typed memory records with versioned updates. Events are append-only facts about how those objects changed, who changed them, and when. Embeddings are stored in separate spaces keyed by provider, model, and kind, so the same object can be represented in multiple semantic spaces without changing vector dimensions.

Lifecycle and conflict metadata are first-class. Objects can move through `scratch`, `candidate`, `accepted`, and `deprecated`, with TTL defaults and explicit expiry. Keyed memories use `memory_key` plus deterministic `value_hash` to mark conflicting values and capture resolution metadata.

## How retrieval works (deterministic + governed)

Default retrieval is strict: only `accepted` and non-expired objects are returned. This applies to scoped reads, structured search, and semantic search.

Clients can opt in to broader views using optional request fields:
- `include_states`
- `include_expired`
- `now` (for deterministic evaluation)

Semantic search can target a specific embedding space with `embedding_provider`, `embedding_model`, and `embedding_kind`. Governance still applies at read time: policy and role checks can deny or clamp requests even when the client asks for broader access.

## Governance model (ownership, audit, retention, explainability)

Workspaces are owned and governed. Ownership controls administrative actions such as token and policy management, while writer/reader roles control day-to-day memory operations.

Auditability is built on immutable event history and timeline queries. Retention boundaries are policy-driven through `policy.retention` objects that define default TTLs, maximum TTL caps, and immutable kinds. These boundaries are enforced on write/update so retention decisions are deterministic and workspace-owned.

When access is denied, responses include explainability fields (`reason_code` and `policy_rule_id`) so operators can understand which policy blocked the action without exposing sensitive data.

## Interfaces

### MCP (primary for agents)

MCP stdio is the primary integration path for agent runtimes.

Core capabilities:
- `openbrain.ping`
- `openbrain.write`
- `openbrain.read`
- `openbrain.search.structured`
- `openbrain.embed.generate`
- `openbrain.search.semantic`

Governance capabilities:
- `openbrain.workspace.info`
- `openbrain.audit.object_timeline`
- `openbrain.audit.memory_key_timeline`
- `openbrain.audit.actor_activity`

Optional enrichment capabilities:
- `openbrain.rerank`
- `openbrain.memory.pack`

### HTTP (mirror/debug/SDK)

HTTP mirrors the MCP surface and is designed for local debugging, service composition, and SDK usage.

Core endpoints:
- `/v1/ping`
- `/v1/write`
- `/v1/read`
- `/v1/search/structured`
- `/v1/embed/generate`
- `/v1/search/semantic`

Governance endpoints:
- `/v1/workspace/info`
- `/v1/audit/object_timeline`
- `/v1/audit/memory_key_timeline`
- `/v1/audit/actor_activity`

Optional enrichment endpoints:
- `/v1/rerank`
- `/v1/memory/pack`

## Quickstart

1. Start Postgres with `pgvector`, then set `DATABASE_URL`.
2. Start OpenBrain HTTP locally:
```bash
openbrain serve
```
3. Authentication:
- HTTP uses `Authorization: Bearer <token>`
- MCP passes `auth_token` during initialize

## SDKs

OpenBrain ships TypeScript and Python SDKs with both an HTTP client and an MCP helper:
- `sdk/typescript/openbrain-sdk`
- `sdk/python/openbrain_sdk`

TypeScript:
```ts
import { OpenBrainHttpClient } from "@openbrain/openbrain-sdk";

const client = new OpenBrainHttpClient({ baseUrl: "http://127.0.0.1:7981" });
await client.ping();
const matches = await client.searchSemantic({ scope: "workspace:demo", query: "release policy", top_k: 3 });
console.log(matches.matches.length);
```

Python:
```python
from openbrain_sdk import OpenBrainHttpClient
from openbrain_sdk.models import SearchSemanticRequest

client = OpenBrainHttpClient(base_url="http://127.0.0.1:7981")
client.ping()
result = client.search_semantic(SearchSemanticRequest(scope="workspace:demo", query="release policy", top_k=3))
print(len(result.matches))
```

## Governance UX (CLI/TUI)

The terminal UX focuses on inspectability and fast policy debugging. `openbrain workspace info` shows ownership and current caller role, `openbrain audit object|key|actor` provides bounded timelines, and `openbrain retention show` displays the effective retention policy. When a command is denied, CLI output surfaces explainability directly as `reason_code` plus `policy_rule_id`.

## Quality and security checks

OpenBrain keeps local quality gates deterministic with a single entrypoint:
- `scripts/ci/quality-gates.ps1` (Windows)
- `scripts/ci/quality-gates.sh` (Unix)

The gate runs formatting, clippy, tests, `cargo deny`, and gitleaks checks. Live-network tests are opt-in only via explicit `RUN_*` flags and are forced off in the quality gate flow.

## Whats next

OpenBrain is now governed and auditable in terminal-first workflows; next work is about visibility and compliance ergonomics on top of the same policy engine and event trail.
- Read-only web governance console
- Compliance pack with tamper-evident event exports and redaction policy tooling
- MCP-over-HTTP transport for broader deployment patterns

---

## License
Apache-2.0 (recommended for wide adoption) or MIT.

Digilabs Company Australia © NYEX AI Platform. All rights reserved. AIC Pty Ltd (ACN 082 378 256)
