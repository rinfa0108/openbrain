# Memory Pack Demo Kit

## What this demonstrates

This demo proves that a small-context agent can operate over a much larger governed memory store by pulling deterministic memory packs under strict token budgets.

The flow is end-to-end and local:
- compose startup with OpenBrain + pgvector Postgres
- deterministic seed dataset (lifecycle states, conflicts, policy/retention)
- budgeted memory pack assembly at multiple budgets
- policy denial explainability (`reason_code` + `policy_rule_id`)
- embedding migration (`coverage` + `reembed`)

## Prerequisites

- Docker Desktop (or Docker Engine + Compose plugin) running
- `pwsh` for Windows script or `bash` + `jq` for Unix script
- OpenBrain CLI available as `openbrain`, or Rust toolchain to run fallback `cargo run -p openbrain-server -- ...`

## One-command run

Windows:

```powershell
pwsh scripts/demo_memory_pack.ps1
```

Unix:

```bash
bash scripts/demo_memory_pack.sh
```

The scripts perform a clean reset (`docker compose down -v`) before seeding so results are deterministic.

## Expected proof output

The runner prints:

1. Workspace bootstrap and token file path (`.openbrain/demo_tokens.json`)
2. Pack summaries for three budgets (`400`, `1200`, `1600`) with:
   - `budget_requested`
   - `budget_used`
   - `items_selected`
   - `truncated`
   - first 20 lines of `pack.text`
3. Conflict behavior:
   - default conflict alerts
   - `include_conflicts_detail=true` detail delta
4. Policy deny explainability:
   - `OB_FORBIDDEN`
   - `reason_code`
   - `policy_rule_id`
5. Embedding migration:
   - coverage before (`fake/fake-v2`)
   - dry-run reembed
   - execute reembed
   - coverage after

## Validation via Web Viewer

After the script completes:

1. Open `http://127.0.0.1:8080/viewer`
2. Paste token from `.openbrain/demo_tokens.json` (writer or owner)
3. Inspect:
   - workspace panel
   - audit timelines
   - retention panel
   - object inspector (`demo-conflict-a`, `demo-pack-context-01`, any `shadow:*` id)

## Safe rerun behavior

- Scripts are idempotent for demo purposes and begin from a clean local compose volume.
- Tokens/reports are written to `.openbrain/` (gitignored).
- Live external provider tests remain disabled (`RUN_OPENAI_LIVE_TESTS=0`, `RUN_ANTHROPIC_LIVE_TESTS=0`).
