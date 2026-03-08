## Local Compose Quickstart

OpenBrain can run locally with one command using Docker Compose.

### Prerequisites
- Docker Desktop (or Docker Engine + Compose plugin)

### Start services
```bash
docker compose up -d
```

This starts:
- `postgres` (pgvector-enabled, persistent volume)
- `openbrain` HTTP server on `http://127.0.0.1:8080`

### Stop services
```bash
docker compose down
```

### Verify server
```bash
curl -sS -X POST http://127.0.0.1:8080/v1/ping -H "Content-Type: application/json" -d '{}'
```

### Run the demo kit
Windows (full deterministic flow):
```powershell
pwsh scripts/demo.ps1
```

Unix (full deterministic flow):
```bash
bash scripts/demo.sh
```

### Token handling
The demo stores tokens at:
- `.openbrain/demo_tokens.json`

That file is gitignored. Tokens are printed once by the script and then persisted only in that local file.

### Auth and first requests
HTTP uses bearer auth:
```bash
curl -sS -X POST http://127.0.0.1:8080/v1/workspace/info \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{}'
```

Example audit query:
```bash
curl -sS -X POST http://127.0.0.1:8080/v1/audit/object_timeline \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"scope":"default","object_id":"demo-accepted","limit":20}'
```
