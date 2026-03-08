# OpenBrain Operations Runbook

Status: DRAFT (screenshots and final polish will be added after IT11A.1)

## Purpose and audience
This runbook is for two audiences:
- Partners and technical evaluators who need to stand up OpenBrain quickly and validate value.
- Enterprise security and operations teams who need governance, auditability, and retention controls.

OpenBrain is the durable state plane for agent memory. Model providers remain stateless compute adapters.

## Installation paths
### Primary: local onboarding with Docker Compose
Use Docker Desktop and run:

```bash
docker compose up -d
```

This starts Postgres with pgvector and OpenBrain server for local evaluation.

### Secondary: enterprise "bring your Postgres"
Run OpenBrain against managed Postgres with pgvector enabled. Keep workspace boundaries and policy controls identical across environments.

## Bootstrap and auth
OpenBrain uses workspace-scoped identities and roles.
- Roles: `owner`, `writer`, `reader`
- HTTP auth: `Authorization: Bearer <token>`
- MCP auth: `auth_token` on initialize

Token hygiene (MVP):
- Tokens are treated as secrets.
- Server stores token hashes, not plaintext tokens.
- Demo/bootstrap token artifacts are saved locally under `.openbrain/` and gitignored.

## Running the system
### Start
```bash
docker compose up -d
```

### Demo kit
```powershell
pwsh scripts/demo.ps1
```

```bash
bash scripts/demo.sh
```

### Optional direct server start
If running outside Compose:
```bash
openbrain serve
```

## Operations procedures
### Migrations
Database migrations run as part of normal startup/demo workflow. Manual SQL steps are not required for standard onboarding.

### Backup and restore basics
- Use standard Postgres backups (`pg_dump` for logical backups, physical backups/PITR for production).
- After restore: verify `POST /v1/ping`, then run workspace and audit queries.

### Troubleshooting
- Docker not running: start Docker Desktop, then rerun `docker compose up -d`.
- DB connection failures: verify `DATABASE_URL` and Postgres container health.
- `pgvector` missing: verify extension creation in startup/init path.
- Token denied:
  - `OB_UNAUTHENTICATED` means missing/invalid token.
  - `OB_FORBIDDEN` includes `reason_code` and `policy_rule_id` for explainability.

## Governance operations
### Retention policy
Retention is workspace-owned policy-as-data via `kind="policy.retention"`, including:
- `default_ttl_by_kind`
- `max_ttl_by_kind`
- `immutable_kinds`

Enforcement happens server-side on write and update.

### Audit queries
Audit views are bounded and workspace-scoped:
- object timeline
- memory_key timeline
- actor activity

### Explainability
Policy denials include deterministic metadata:
- `reason_code`
- `policy_rule_id`

## Minimal command reference
```bash
openbrain workspace info
openbrain audit object <object_id>
openbrain audit key <memory_key>
openbrain audit actor <identity_id>
openbrain retention show
```
