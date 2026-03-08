# OpenBrain Governance Demo (IT10C)

This demo uses the IT10B governance endpoints through the OpenBrain CLI UX commands added in IT10C.

## Prerequisites

- OpenBrain server running locally (`openbrain serve`)
- Owner and reader tokens available
- Workspace/scope id (example uses `ws-default`)

Set env vars:

```powershell
$env:OPENBRAIN_BASE_URL = "http://127.0.0.1:7981"
$env:OPENBRAIN_SCOPE = "ws-default"
$env:OWNER_TOKEN = "<owner-token>"
$env:READER_TOKEN = "<reader-token>"
```

## 1) Inspect workspace ownership + caller role

```powershell
openbrain workspace info --token $env:OWNER_TOKEN
```

Expected: prints `workspace_id`, `owner_identity_id`, `caller_identity_id`, `caller_role`.

## 2) Write retention policy object (owner)

```powershell
$body = @{
  objects = @(@{
    type = "policy.retention"
    id = "policy.retention.default"
    scope = $env:OPENBRAIN_SCOPE
    status = "active"
    spec_version = "0.1"
    data = @{
      default_ttl_by_kind = @{ scratch = 7; candidate = 30; signal = 7 }
      max_ttl_by_kind = @{ pii = 30; credential = 7 }
      immutable_kinds = @("pii", "credential")
    }
    provenance = @{ actor = "demo-owner" }
    lifecycle_state = "accepted"
  })
} | ConvertTo-Json -Depth 8

Invoke-RestMethod -Method Post `
  -Uri "$env:OPENBRAIN_BASE_URL/v1/write" `
  -Headers @{ Authorization = "Bearer $env:OWNER_TOKEN" } `
  -ContentType "application/json" `
  -Body $body
```

## 3) Write a memory object (writer/owner)

```powershell
$memory = @{
  objects = @(@{
    type = "decision"
    id = "decision.db.01"
    scope = $env:OPENBRAIN_SCOPE
    status = "active"
    spec_version = "0.1"
    data = @{ choice = "postgres"; rationale = "consistency" }
    provenance = @{ actor = "demo-writer" }
    lifecycle_state = "accepted"
    memory_key = "decision:db"
  })
} | ConvertTo-Json -Depth 8

Invoke-RestMethod -Method Post `
  -Uri "$env:OPENBRAIN_BASE_URL/v1/write" `
  -Headers @{ Authorization = "Bearer $env:OWNER_TOKEN" } `
  -ContentType "application/json" `
  -Body $memory
```

## 4) Attempt forbidden action as reader (expect explainability)

```powershell
openbrain audit actor owner --scope $env:OPENBRAIN_SCOPE --token $env:READER_TOKEN
```

Expected on deny:

```text
DENIED: <reason_code> (rule: <policy_rule_id>)
```

## 5) Inspect audit timeline by object and actor

```powershell
openbrain audit object decision.db.01 --scope $env:OPENBRAIN_SCOPE --limit 20 --token $env:OWNER_TOKEN
openbrain audit actor owner --scope $env:OPENBRAIN_SCOPE --limit 20 --token $env:OWNER_TOKEN
```

Expected columns:

```text
timestamp | event_type | actor_id | object_id | version | summary
```

## 6) Inspect effective retention policy

```powershell
openbrain retention show --scope $env:OPENBRAIN_SCOPE --token $env:OWNER_TOKEN
```

Expected fields:

- `default_ttl_by_kind`
- `max_ttl_by_kind`
- `immutable_kinds`
