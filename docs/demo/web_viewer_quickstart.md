# Web Viewer Quickstart (Read-only, Localhost-only)

This viewer is a thin local inspection surface for OpenBrain governance and memory state. It is read-only and calls existing `/v1/*` routes only.

## Start OpenBrain

```powershell
openbrain serve
```

Open the viewer:

- `http://127.0.0.1:8080/viewer`

## Token setup

Use a workspace token from the IT11A demo kit (`.openbrain/demo_tokens.json`) and paste it into the **Token (Bearer)** field.

- HTTP auth is `Authorization: Bearer <token>`.
- Viewer stores base URL + token in browser localStorage for localhost use.

## What you can inspect

1. Connection panel:
- Set base URL (default `http://127.0.0.1:8080`), save token, run ping.

2. Workspace panel:
- Load `workspace_id`, `owner_identity_id`, `caller_identity_id`, `caller_role`.

3. Audit panel:
- Object timeline via `/v1/audit/object_timeline`
- Memory key timeline via `/v1/audit/memory_key_timeline`
- Actor activity via `/v1/audit/actor_activity`

4. Retention panel:
- Loads latest `policy.retention` object by calling `/v1/search/structured` and `/v1/read`.

5. Object inspector:
- Reads one object by `scope` + `object_id` and shows lifecycle/conflict metadata.
- Data is shown as a collapsed JSON preview by default.

## Deny explainability

If policy blocks a request (`OB_FORBIDDEN`), the viewer surfaces:

- `reason_code`
- `policy_rule_id`

## Notes

- No write/edit/delete actions are available in this viewer.
- No new API endpoints are required; this is static content served under `/viewer`.
