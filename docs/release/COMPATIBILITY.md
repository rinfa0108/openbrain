# Compatibility Guarantees (v1.0)

This document defines the compatibility contract for OpenBrain v1.0.0.

## Stable Surfaces

### HTTP `/v1/*`
- `POST /v1/ping`
- `POST /v1/write`
- `POST /v1/read`
- `POST /v1/search/structured`
- `POST /v1/embed/generate`
- `POST /v1/search/semantic`
- `POST /v1/rerank`
- `POST /v1/memory/pack`
- `POST /v1/workspace/info`
- `POST /v1/audit/object_timeline`
- `POST /v1/audit/memory_key_timeline`
- `POST /v1/audit/actor_activity`

### MCP tools
- `openbrain.ping`
- `openbrain.write`
- `openbrain.read`
- `openbrain.search.structured`
- `openbrain.embed.generate`
- `openbrain.search.semantic`
- `openbrain.rerank`
- `openbrain.memory.pack`
- `openbrain.workspace.info`
- `openbrain.audit.object_timeline`
- `openbrain.audit.memory_key_timeline`
- `openbrain.audit.actor_activity`

### Memory pack contract fields
- `pack.text`
- `pack.items`
- `budget_requested`
- `budget_used`
- `items_selected`
- `truncated`
- `conflict_alerts` (when applicable)

## Experimental / Subject to Change
- CLI presentation formatting (tables, spacing, human-readable labels).
- Optional additive request/response diagnostics fields.
- Internal scoring weights used by deterministic pack ranking (documented, but tunable in minors).

## SemVer Commitments
- Breaking API/tool contract changes are introduced only in major versions.
- Minor versions may add optional request/response fields and new additive commands/tools.
- Patch versions are for fixes/security/compatibility without contract breakage.

## Deprecation Policy
- Deprecations are documented in `CHANGELOG.md` with migration guidance.
- Deprecated fields/surfaces are retained for at least one minor release before removal in a major.