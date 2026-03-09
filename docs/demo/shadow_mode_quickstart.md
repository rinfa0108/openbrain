# Shadow Mode Quickstart

## Why Shadow Mode exists

Shadow mode lets teams prove memory value without refactoring existing agent flows. It extracts deterministic memory candidates from transcripts, previews them, and can optionally write them as `scratch` memories only.

## Dry-run from stdin (default-safe)

```powershell
@'
decision: Use local pgvector for dev
Alex likes short release notes
todo: prepare customer migration guide
The pipeline must not expose API keys
'@ | openbrain shadow --workspace default --stdin --mode dry-run --format text
```

Behavior:
- No writes are performed in `dry-run`.
- Output is deterministic for the same input.

## Dry-run from file with JSON report

```powershell
openbrain shadow `
  --workspace default `
  --input .\transcript.txt `
  --mode dry-run `
  --out .\shadow-report.json `
  --out-html .\shadow-report.html
```

## Write scratch-only candidates

```powershell
openbrain shadow `
  --workspace default `
  --token $env:OPENBRAIN_TOKEN `
  --input .\transcript.txt `
  --mode write-scratch `
  --limit 50 `
  --actor shadow-cli
```

Write mode guarantees:
- Every created object is forced to `lifecycle_state=scratch`.
- Workspace token + RBAC + policy checks are enforced before write.
- Policy denials print explainability: `reason_code` + `policy_rule_id`.

## Validate with the Web Viewer

After write mode, inspect captured memories in the viewer:

1. Start server: `openbrain serve`
2. Open `http://127.0.0.1:7981/viewer`
3. Paste a workspace token
4. Use Object Inspector or Audit views to verify shadow captures and event trail
