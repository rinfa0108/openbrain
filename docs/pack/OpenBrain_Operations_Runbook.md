# OpenBrain Operations Runbook

Status: FINAL

## Purpose and audience
This runbook is for partner evaluators and enterprise operations teams. OpenBrain is the durable state plane for governed memory, while model providers remain stateless compute adapters.

## Installation paths
### Primary: local onboarding with Docker Compose

```bash
docker compose up -d
```

This starts Postgres with pgvector and OpenBrain server on localhost.

### Secondary: enterprise "bring your Postgres"
Run OpenBrain against managed Postgres with pgvector enabled. Keep workspace boundaries, policy controls, and audit flows identical across environments.

## Bootstrap and auth
OpenBrain uses workspace-scoped roles (`owner`, `writer`, `reader`).

- HTTP auth: `Authorization: Bearer <token>`
- MCP auth: `auth_token` on initialize

Token hygiene:

- Server stores token hashes only.
- Demo/bootstrap token artifacts are written to `.openbrain/` and are gitignored.
- Rotate writer/reader tokens periodically and revoke compromised tokens.

## Running the system

```bash
docker compose up -d
pwsh scripts/demo.ps1
# or
bash scripts/demo.sh
```

`openbrain serve` remains available for non-compose local runs.

## Validate via Web Viewer
After startup, validate the environment visually using the read-only viewer.

1. Open `http://127.0.0.1:8080/viewer`.
2. Paste a token from `.openbrain/demo_tokens.json`.
3. Confirm `workspace info` and one `audit` query return expected records.

Viewer screenshots:

- `docs/pack/assets/viewer/viewer-01-connection-token.png`
- `docs/pack/assets/viewer/viewer-02-workspace-info.png`
- `docs/pack/assets/viewer/viewer-03-audit-object-timeline.png`
- `docs/pack/assets/viewer/viewer-08-deny-explainability.png`

## Operations procedures
### Migrations
Migrations run during normal startup/demo flow. Manual SQL steps are not required for standard onboarding.

### Backup and restore
Use normal Postgres backup strategy (`pg_dump` for logical backup, PITR/physical backup for production). After restore, verify `/v1/ping`, workspace info, and audit queries.

### Troubleshooting
- Docker not running: start Docker Desktop and rerun `docker compose up -d`.
- DB connection issues: verify `DATABASE_URL` and Postgres health.
- Missing pgvector: verify extension init path in Compose.
- Denied request:
  - `OB_UNAUTHENTICATED` = missing/invalid token.
  - `OB_FORBIDDEN` = policy denial with `reason_code` and `policy_rule_id`.

## Re-embed + coverage operations
Use this when moving retrieval to a new embedding provider/model/kind.

1. Measure baseline coverage in target space.
2. Run re-embed in dry-run mode.
3. Execute bounded batches with `--limit` and optional resume cursor.
4. Re-run coverage and verify completion.

Example:

```bash
openbrain embed coverage --workspace ws_demo --provider fake --model fake-v1 --kind semantic
openbrain embed reembed --workspace ws_demo --to-provider fake --to-model fake-v2 --to-kind semantic --dry-run --limit 100
openbrain embed reembed --workspace ws_demo --to-provider fake --to-model fake-v2 --to-kind semantic --limit 100
```

Re-embed obeys lifecycle defaults (accepted + not expired) and policy constraints.

## Governance operations
### Retention policy
Retention is workspace-owned policy-as-data via `kind="policy.retention"` with:

- `default_ttl_by_kind`
- `max_ttl_by_kind`
- `immutable_kinds`

### Audit queries
Audit views are bounded and workspace-scoped:

- object timeline
- memory_key timeline
- actor activity

### Explainability
Policy denials include deterministic metadata:

- `reason_code`
- `policy_rule_id`

## Memory pack principle (north-star)
Small-context agents retrieve a governed memory pack from a much larger durable store. The system does not rely on agents reading massive history directly; it enforces scoped, policy-checked retrieval for each interaction.

## Minimal command reference

```bash
openbrain workspace info
openbrain audit object <object_id>
openbrain audit key <memory_key>
openbrain audit actor <identity_id>
openbrain retention show
openbrain embed coverage --workspace <id> --provider <p> --model <m>
openbrain embed reembed --workspace <id> --to-provider <p> --to-model <m> --dry-run
```
