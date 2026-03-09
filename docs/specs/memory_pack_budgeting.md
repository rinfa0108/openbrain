# Memory Pack Budgeting

OpenBrain memory packs let a small-context agent operate over a large durable store by assembling a deterministic, policy-filtered subset of memory.

## What a memory pack contains

A pack is returned by existing surfaces:
- HTTP: `POST /v1/memory/pack`
- MCP: `openbrain.memory.pack`

The response includes:
- `pack.text`: deterministic prompt-ready text blocks
- `pack.items`: structured selected items with metadata
- `budget_requested`, `budget_used`, `items_selected`, `truncated`
- `conflict_alerts` for unresolved key collisions

## Request shape (deterministic mode)

Key request fields:
- `scope`
- `task_hint`
- `query` (optional)
- `structured_filter` (optional)
- `semantic` (optional)
- `embedding_provider`, `embedding_model`, `embedding_kind` (optional semantic space selector)
- `budget_tokens` (default `1200`)
- `top_k` (candidate cap)
- `max_per_key` (default `1`)
- `include_states`, `include_expired`, `now` (override lifecycle defaults)
- `include_conflicts_detail` (default `false`)

## Retrieval and governance behavior

Default retrieval is governed and deterministic:
- accepted + not expired by default
- workspace boundary enforced
- policy engine enforced before pack build
- policy top-k clamps are applied

Candidate retrieval is hybrid:
- structured search path
- semantic search path (if enabled)
- union + stable rank + dedupe by `memory_key`

## Budgeting and truncation

Budget estimation is deterministic and local:
- `approx_tokens = ceil(char_count / 4)`

Builder behavior:
- hard budget cap is enforced
- selection order is stable
- if an item would exceed budget, assembly stops
- `truncated=true` and `constraints` include `pack_truncated_to_budget`

## Conflict handling

For unresolved conflicts:
- default includes one best item per key (`max_per_key=1`)
- `conflict_alerts` summarize conflicting memory keys
- detailed conflicting IDs are optional via `include_conflicts_detail=true`

## Example (HTTP)

```bash
curl -sS http://127.0.0.1:7981/v1/memory/pack \
  -H "Authorization: Bearer $OPENBRAIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "scope": "ws-default",
    "task_hint": "Answer current DB decision question",
    "query": "database decision",
    "semantic": true,
    "embedding_provider": "fake",
    "embedding_model": "fake-v1",
    "embedding_kind": "semantic",
    "budget_tokens": 1200,
    "top_k": 20,
    "max_per_key": 1
  }'
```

## Example (MCP tool call)

```json
{
  "name": "openbrain.memory.pack",
  "arguments": {
    "scope": "ws-default",
    "task_hint": "Prepare response context",
    "query": "customer id format",
    "semantic": true,
    "budget_tokens": 1000
  }
}
```

## Recommended agent prompt pattern

1. Call memory-pack for the current task with explicit budget.
2. Feed only `pack.text` and selected metadata to the model.
3. If `truncated=true`, ask a follow-up narrow query and rebuild the pack.
