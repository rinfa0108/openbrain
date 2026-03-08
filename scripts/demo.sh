#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export RUN_OPENAI_LIVE_TESTS=0
export RUN_ANTHROPIC_LIVE_TESTS=0

require() {
  command -v "$1" >/dev/null 2>&1 || { echo "Missing required command: $1" >&2; exit 1; }
}

require docker
require curl
require jq

mkdir -p .openbrain
token_file=".openbrain/demo_tokens.json"

post_json() {
  local path="$1"
  local token="${2:-}"
  local body="$3"
  if [[ -n "$token" ]]; then
    curl -fsS -X POST "http://127.0.0.1:8080${path}" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      -d "$body"
  else
    curl -fsS -X POST "http://127.0.0.1:8080${path}" \
      -H "Content-Type: application/json" \
      -d "$body"
  fi
}

wait_ping() {
  for _ in $(seq 1 60); do
    if post_json "/v1/ping" "" '{}' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "openbrain server did not become ready on http://127.0.0.1:8080" >&2
  exit 1
}

bootstrap_token() {
  for _ in $(seq 1 60); do
    local line
    line="$(docker compose logs openbrain --no-color 2>/dev/null | sed -n 's/.*bootstrap owner token (workspace=\([^)]*\)): \([^ ]*\).*/\1 \2/p' | tail -n1)"
    if [[ -n "$line" ]]; then
      echo "$line"
      return 0
    fi
    sleep 1
  done
  echo "could not find bootstrap owner token in openbrain logs" >&2
  exit 1
}

echo "== OpenBrain IT11A Demo =="
docker compose up -d >/dev/null
wait_ping

read -r workspace_id owner_token < <(bootstrap_token)

writer_resp="$(post_json "/v1/workspace/token/create" "$owner_token" '{"role":"writer","label":"demo-writer","display_name":"Demo Writer"}')"
reader_resp="$(post_json "/v1/workspace/token/create" "$owner_token" '{"role":"reader","label":"demo-reader","display_name":"Demo Reader"}')"

writer_token="$(echo "$writer_resp" | jq -r '.token')"
reader_token="$(echo "$reader_resp" | jq -r '.token')"

jq -n \
  --arg ws "$workspace_id" \
  --arg owner "$owner_token" \
  --arg writer "$writer_token" \
  --arg reader "$reader_token" \
  --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{workspace_id:$ws,owner_token:$owner,writer_token:$writer,reader_token:$reader,created_at:$created_at}' > "$token_file"

echo "workspace: $workspace_id"
echo "owner token (printed once): $owner_token"
echo "writer token (printed once): $writer_token"
echo "reader token (printed once): $reader_token"
echo "tokens saved: $token_file"

echo
echo "== Workspace Info =="
post_json "/v1/workspace/info" "$owner_token" '{}' | jq

post_json "/v1/write" "$owner_token" "$(jq -nc --arg ws "$workspace_id" '{objects:[{type:"policy.retention",id:"policy-retention-demo",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo"],data:{default_ttl_by_kind:{scratch:7,candidate:30},max_ttl_by_kind:{pii:30},immutable_kinds:["pii","credential"]},provenance:{actor:"owner"},lifecycle_state:"accepted"}]}')" >/dev/null

echo "Installed policy.retention object: policy-retention-demo"

post_json "/v1/write" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{objects:[{type:"note",id:"demo-scratch",scope:$ws,status:"draft",spec_version:"0.1",tags:["demo","lifecycle"],data:{text:"scratch memory"},provenance:{actor:"writer"},lifecycle_state:"scratch"},{type:"note",id:"demo-candidate",scope:$ws,status:"candidate",spec_version:"0.1",tags:["demo","lifecycle"],data:{text:"candidate memory"},provenance:{actor:"writer"},lifecycle_state:"candidate"},{type:"note",id:"demo-accepted",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo","lifecycle"],data:{text:"accepted memory"},provenance:{actor:"writer"},lifecycle_state:"accepted"}]}')" >/dev/null

echo
echo "== Lifecycle Retrieval =="
default_read="$(post_json "/v1/read" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,refs:["demo-scratch","demo-candidate","demo-accepted"]}')")"
echo "default read count (accepted + not expired): $(echo "$default_read" | jq '.objects|length')"

override_read="$(post_json "/v1/read" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,refs:["demo-scratch","demo-candidate","demo-accepted"],include_states:["scratch","candidate","accepted"],include_expired:true}')")"
echo "override read count (scratch/candidate/accepted): $(echo "$override_read" | jq '.objects|length')"

post_json "/v1/embed/generate" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,target:{ref:"demo-accepted"},model:"fake-v1"}')" >/dev/null
semantic="$(post_json "/v1/search/semantic" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,query:"accepted memory",top_k:5,embedding_provider:"fake",embedding_model:"fake-v1",embedding_kind:"semantic"}')")"
echo "semantic matches: $(echo "$semantic" | jq '.matches|length') (provider=fake, model=fake-v1, kind=semantic)"

post_json "/v1/write" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{objects:[{type:"decision",id:"demo-conflict-a",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo","conflict"],data:{choice:"postgres"},provenance:{actor:"writer"},lifecycle_state:"accepted",memory_key:"decision:db",conflict_status:"unresolved"},{type:"decision",id:"demo-conflict-b",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo","conflict"],data:{choice:"sqlite"},provenance:{actor:"writer"},lifecycle_state:"accepted",memory_key:"decision:db",conflict_status:"unresolved"}]}')" >/dev/null

conflict_before="$(post_json "/v1/search/structured" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,where_expr:"memory_key == \"decision:db\"",include_conflicts:true,limit:20,offset:0}')")"
echo "conflict before resolution: conflict=$(echo "$conflict_before" | jq -r '.results[0].conflict') count=$(echo "$conflict_before" | jq -r '.results[0].conflict_count')"

now_utc="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
post_json "/v1/write" "$writer_token" "$(jq -nc --arg ws "$workspace_id" --arg now "$now_utc" '{objects:[{type:"decision",id:"demo-conflict-a",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo","conflict"],data:{choice:"postgres"},provenance:{actor:"writer"},lifecycle_state:"accepted",memory_key:"decision:db",conflict_status:"resolved",resolved_by_object_id:"demo-conflict-a",resolved_at:$now,resolution_note:"demo winner"},{type:"decision",id:"demo-conflict-b",scope:$ws,status:"deprecated",spec_version:"0.1",tags:["demo","conflict"],data:{choice:"sqlite"},provenance:{actor:"writer"},lifecycle_state:"deprecated",memory_key:"decision:db",conflict_status:"resolved",resolved_by_object_id:"demo-conflict-a",resolved_at:$now,resolution_note:"deprecated after resolution"}]}')" >/dev/null

conflict_after="$(post_json "/v1/search/structured" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,where_expr:"memory_key == \"decision:db\"",include_conflicts:true,include_states:["accepted","deprecated"],limit:20,offset:0}')")"
echo "conflict after resolution (winner): status=$(echo "$conflict_after" | jq -r '.results[] | select(.ref=="demo-conflict-a") | .conflict_status') resolved_by=$(echo "$conflict_after" | jq -r '.results[] | select(.ref=="demo-conflict-a") | .resolved_by_object_id')"

post_json "/v1/write" "$owner_token" "$(jq -nc --arg ws "$workspace_id" '{objects:[{type:"policy.rule",id:"demo-policy-deny-reader-audit",scope:$ws,status:"canonical",spec_version:"0.1",tags:["demo","policy"],data:{id:"demo-deny-reader-audit",effect:"deny",operations:["audit_object_timeline"],roles:["reader"],reason:"OB_POLICY_DENY_AUDIT_READ_DEMO"},provenance:{actor:"owner"},lifecycle_state:"accepted"}]}')" >/dev/null

reader_denied="$(post_json "/v1/audit/object_timeline" "$reader_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,object_id:"demo-accepted",limit:20}')")"
echo
if [[ "$(echo "$reader_denied" | jq -r '.ok')" == "false" ]]; then
  echo "DENIED: $(echo "$reader_denied" | jq -r '.error.details.reason_code') (rule: $(echo "$reader_denied" | jq -r '.error.details.policy_rule_id'))"
fi

echo
echo "== Audit Timelines =="
audit_obj="$(post_json "/v1/audit/object_timeline" "$owner_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,object_id:"demo-conflict-a",limit:20}')")"
echo "object timeline events: $(echo "$audit_obj" | jq '.events|length')"
audit_actor="$(post_json "/v1/audit/actor_activity" "$owner_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,actor_identity_id:"writer",limit:20}')")"
echo "actor timeline events (writer): $(echo "$audit_actor" | jq '.events|length')"

echo
echo "Demo complete."
echo "- Workspace info shown"
echo "- Tokens created and saved"
echo "- policy.retention installed"
echo "- Lifecycle defaults + override shown"
echo "- Semantic search (fake embeddings) shown"
echo "- Conflict detection + resolution shown"
echo "- Deny explainability shown"
echo "- Audit timelines shown"