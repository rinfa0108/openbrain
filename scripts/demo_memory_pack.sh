#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

export RUN_OPENAI_LIVE_TESTS=0
export RUN_ANTHROPIC_LIVE_TESTS=0

base_url="http://127.0.0.1:8080"
db_url="postgres://postgres:postgres@127.0.0.1:5432/openbrain"
fixture_path="docs/demo/fixtures/transcript_demo.txt"
state_dir=".openbrain"
token_file="$state_dir/demo_tokens.json"
shadow_json="$state_dir/shadow_report.json"
shadow_html="$state_dir/shadow_report.html"

mkdir -p "$state_dir"

require() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

docker_preflight() {
  require docker
  require curl
  require jq
  if ! docker info >/dev/null 2>&1; then
    echo "docker is not running; start Docker Desktop/Engine and re-run" >&2
    exit 1
  fi
}

post_json() {
  local path="$1"
  local token="$2"
  local body="$3"
  if [[ -n "$token" ]]; then
    curl -fsS -X POST "${base_url}${path}" \
      -H "Authorization: Bearer ${token}" \
      -H "Content-Type: application/json" \
      -d "$body"
  else
    curl -fsS -X POST "${base_url}${path}" \
      -H "Content-Type: application/json" \
      -d "$body"
  fi
}

wait_ping() {
  for _ in $(seq 1 90); do
    if post_json "/v1/ping" "" '{}' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "openbrain server did not become ready at ${base_url}" >&2
  exit 1
}

bootstrap_token() {
  for _ in $(seq 1 90); do
    local line
    line="$(docker compose logs openbrain --no-color 2>/dev/null | sed -n 's/.*bootstrap owner token (workspace=\([^)]*\)): \([^ ]*\).*/\1 \2/p' | tail -n1)"
    if [[ -n "$line" ]]; then
      echo "$line"
      return 0
    fi
    sleep 1
  done
  echo "could not find bootstrap owner token in container logs" >&2
  exit 1
}

openbrain_cli() {
  if command -v openbrain >/dev/null 2>&1; then
    openbrain "$@"
  else
    cargo run -q -p openbrain-server -- "$@"
  fi
}

demo_text() {
  local seed="$1"
  printf 'Decision context for %s in OpenBrain memory. This block is intentionally verbose for deterministic budget pressure. Keep governance, lifecycle, and retention constraints visible in the final pack. Use this memory to answer rollout and migration questions. Decision context for %s in OpenBrain memory. This block is intentionally verbose for deterministic budget pressure. Keep governance, lifecycle, and retention constraints visible in the final pack. Use this memory to answer rollout and migration questions.' "$seed" "$seed"
}

show_pack_summary() {
  local resp="$1"
  local label="$2"
  echo
  echo "== Pack (${label}) =="
  echo "budget_requested=$(echo "$resp" | jq -r '.pack.budget_requested') budget_used=$(echo "$resp" | jq -r '.pack.budget_used') items_selected=$(echo "$resp" | jq -r '.pack.items_selected') truncated=$(echo "$resp" | jq -r '.pack.truncated')"
  echo "pack.items count=$(echo "$resp" | jq -r '.pack.items | length')"
  echo "conflict_alerts=$(echo "$resp" | jq -r '.pack.conflict_alerts | length')"
  if [[ "$(echo "$resp" | jq -r '.pack.conflict_alerts | length')" != "0" ]]; then
    echo "first conflict: key=$(echo "$resp" | jq -r '.pack.conflict_alerts[0].memory_key') count=$(echo "$resp" | jq -r '.pack.conflict_alerts[0].conflicting_count') status=$(echo "$resp" | jq -r '.pack.conflict_alerts[0].conflict_status')"
  fi
  echo "--- first 20 lines of pack.text ---"
  echo "$resp" | jq -r '.pack.text' | head -n 20
}

echo "== OpenBrain IT12C Memory Pack Demo =="
echo "Preflight: docker"
docker_preflight

echo "Resetting compose environment to a clean state"
docker compose down -v --remove-orphans >/dev/null
docker compose up -d >/dev/null

echo "Waiting for server readiness"
wait_ping

read -r workspace_id owner_token < <(bootstrap_token)

writer_resp="$(post_json "/v1/workspace/token/create" "$owner_token" '{"role":"writer","label":"demo-pack-writer","display_name":"Demo Pack Writer"}')"
reader_resp="$(post_json "/v1/workspace/token/create" "$owner_token" '{"role":"reader","label":"demo-pack-reader","display_name":"Demo Pack Reader"}')"

writer_token="$(echo "$writer_resp" | jq -r '.token')"
reader_token="$(echo "$reader_resp" | jq -r '.token')"

jq -n \
  --arg ws "$workspace_id" \
  --arg owner "$owner_token" \
  --arg writer "$writer_token" \
  --arg reader "$reader_token" \
  --arg created_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  '{workspace_id:$ws,owner_token:$owner,writer_token:$writer,reader_token:$reader,created_at:$created_at}' > "$token_file"

echo "workspace=${workspace_id}"
echo "tokens saved: ${token_file}"

echo
echo "Installing retention policy and policy deny rule for reader memory packs"
post_json "/v1/write" "$owner_token" "$(jq -nc --arg ws "$workspace_id" '
{
  objects: [
    {
      type: "policy.retention",
      id: "policy-retention-it12c",
      scope: $ws,
      status: "canonical",
      spec_version: "0.1",
      tags: ["demo","it12c"],
      data: {
        default_ttl_by_kind: {scratch: 7, candidate: 30},
        max_ttl_by_kind: {pii: 30},
        immutable_kinds: ["pii","credential"]
      },
      provenance: {actor: "owner"},
      lifecycle_state: "accepted"
    },
    {
      type: "policy.rule",
      id: "policy-deny-reader-memory-pack-it12c",
      scope: $ws,
      status: "canonical",
      spec_version: "0.1",
      tags: ["demo","it12c","policy"],
      data: {
        id: "rule-deny-reader-memory-pack-it12c",
        effect: "deny",
        operations: ["memory_pack"],
        roles: ["reader"],
        reason: "OB_POLICY_DENY_MEMORY_PACK_READER_DEMO"
      },
      provenance: {actor: "owner"},
      lifecycle_state: "accepted"
    }
  ]
}')" >/dev/null

echo
echo "Seeding transcript via shadow mode (write-scratch)"
openbrain_cli shadow \
  --database-url "$db_url" \
  --workspace "$workspace_id" \
  --token "$writer_token" \
  --mode write-scratch \
  --input "$fixture_path" \
  --format text \
  --limit 50 \
  --actor shadow-demo \
  --out "$shadow_json" \
  --out-html "$shadow_html"

mapfile -t shadow_refs < <(jq -r '.written_refs[]' "$shadow_json")
if [[ "${#shadow_refs[@]}" -lt 3 ]]; then
  echo "expected at least 3 shadow refs, got ${#shadow_refs[@]}" >&2
  exit 1
fi

refs_json="$(printf '%s\n' "${shadow_refs[@]}" | jq -R . | jq -s .)"
shadow_read="$(post_json "/v1/read" "$writer_token" "$(jq -nc --arg ws "$workspace_id" --argjson refs "$refs_json" '{scope:$ws,refs:$refs,include_states:["scratch","candidate","accepted"]}')")"

updates_json="$(echo "$shadow_read" | jq -c --arg ws "$workspace_id" '
  .objects
  | to_entries
  | map(
      .value as $o
      | {
          type: ($o.type // "note"),
          id: $o.ref,
          scope: $ws,
          status: (if .key < 2 then "canonical" elif .key == 2 then "candidate" else "active" end),
          spec_version: "0.1",
          tags: ["demo","shadow","it12c"],
          data: $o.data,
          provenance: {actor: "writer", source: "shadow-promote"},
          lifecycle_state: (if .key < 2 then "accepted" elif .key == 2 then "candidate" else "scratch" end),
          memory_key: $o.memory_key
      }
    )')"

context_json="$(for n in $(seq -w 1 8); do
  id="demo-pack-context-${n}"
  text="$(demo_text "$id")"
  jq -nc --arg ws "$workspace_id" --arg id "$id" --arg text "$text" '
  {
    type: "context",
    id: $id,
    scope: $ws,
    status: "canonical",
    spec_version: "0.1",
    tags: ["demo","it12c","pack"],
    data: {text: $text},
    provenance: {actor: "writer"},
    lifecycle_state: "accepted",
    memory_key: ("context:" + $id)
  }'
done | jq -s -c '.')"

extras_json="$(jq -nc --arg ws "$workspace_id" '
[
  {
    type: "note",
    id: "demo-hidden-scratch",
    scope: $ws,
    status: "draft",
    spec_version: "0.1",
    tags: ["demo","it12c"],
    data: {text: "scratch object should be excluded by default pack retrieval"},
    provenance: {actor: "writer"},
    lifecycle_state: "scratch",
    memory_key: "note:hidden-scratch"
  },
  {
    type: "note",
    id: "demo-hidden-candidate",
    scope: $ws,
    status: "candidate",
    spec_version: "0.1",
    tags: ["demo","it12c"],
    data: {text: "candidate object should be excluded by default pack retrieval"},
    provenance: {actor: "writer"},
    lifecycle_state: "candidate",
    memory_key: "note:hidden-candidate"
  },
  {
    type: "decision",
    id: "demo-conflict-a",
    scope: $ws,
    status: "canonical",
    spec_version: "0.1",
    tags: ["demo","it12c","conflict"],
    data: {choice: "postgres", rationale: "operational simplicity"},
    provenance: {actor: "writer"},
    lifecycle_state: "accepted",
    memory_key: "decision:db",
    conflict_status: "unresolved"
  },
  {
    type: "decision",
    id: "demo-conflict-b",
    scope: $ws,
    status: "canonical",
    spec_version: "0.1",
    tags: ["demo","it12c","conflict"],
    data: {choice: "sqlite", rationale: "single file dev speed"},
    provenance: {actor: "writer"},
    lifecycle_state: "accepted",
    memory_key: "decision:db",
    conflict_status: "unresolved"
  }
]')"

all_objects="$(jq -nc --argjson a "$updates_json" --argjson b "$context_json" --argjson c "$extras_json" '{objects: ($a + $b + $c)}')"
post_json "/v1/write" "$writer_token" "$all_objects" >/dev/null
echo "seeded deterministic objects: $(echo "$all_objects" | jq '.objects | length')"

echo
echo "Generating initial embedding in legacy space (fake/fake-v1)"
post_json "/v1/embed/generate" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '{scope:$ws,target:{ref:"demo-pack-context-01"},model:"fake-v1"}')" >/dev/null

for budget in 400 1200 1600; do
  pack_resp="$(post_json "/v1/memory/pack" "$writer_token" "$(jq -nc --arg ws "$workspace_id" --argjson b "$budget" '
  {
    scope: $ws,
    task_hint: "Prepare response context for database and rollout decisions",
    query: "database decision rollout context",
    semantic: false,
    budget_tokens: $b,
    top_k: 30,
    max_per_key: 1,
    include_conflicts: true
  }')")"
  show_pack_summary "$pack_resp" "${budget} tokens"
done

echo
echo "== Conflict Detail Toggle =="
pack_default_conflict="$(post_json "/v1/memory/pack" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '
{
  scope: $ws,
  task_hint: "Compare unresolved conflict behavior",
  query: "decision db conflict",
  semantic: false,
  budget_tokens: 1200,
  structured_filter: "memory_key == \"decision:db\"",
  include_conflicts: true,
  include_conflicts_detail: false
}')")"

pack_detail_conflict="$(post_json "/v1/memory/pack" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '
{
  scope: $ws,
  task_hint: "Compare unresolved conflict behavior",
  query: "decision db conflict",
  semantic: false,
  budget_tokens: 1200,
  structured_filter: "memory_key == \"decision:db\"",
  include_conflicts: true,
  include_conflicts_detail: true
}')")"

echo "default detail: conflicting_object_ids populated=$(echo "$pack_default_conflict" | jq -r '.pack.conflict_alerts[0].conflicting_object_ids != null')"
echo "include_conflicts_detail=true: conflicting_object_ids count=$(echo "$pack_detail_conflict" | jq -r '.pack.conflict_alerts[0].conflicting_object_ids | length')"

echo
echo "== Policy Deny Explainability =="
denied="$(post_json "/v1/memory/pack" "$reader_token" "$(jq -nc --arg ws "$workspace_id" '
{
  scope: $ws,
  task_hint: "reader should be denied by policy",
  query: "database decision",
  semantic: false,
  budget_tokens: 400
}')")"

if [[ "$(echo "$denied" | jq -r '.ok')" != "false" ]]; then
  echo "expected reader memory pack to be denied by policy" >&2
  exit 1
fi
echo "OB_FORBIDDEN reason_code=$(echo "$denied" | jq -r '.error.details.reason_code') policy_rule_id=$(echo "$denied" | jq -r '.error.details.policy_rule_id')"

echo
echo "== Embedding Migration (Coverage + Reembed) =="
echo "Coverage before target space fake/fake-v2:"
openbrain_cli embed \
  --database-url "$db_url" \
  --token "$writer_token" \
  coverage \
  --workspace "$workspace_id" \
  --provider fake \
  --model fake-v2 \
  --kind semantic \
  --state accepted \
  --missing-sample 5

echo
echo "Reembed dry-run:"
openbrain_cli embed \
  --database-url "$db_url" \
  --token "$writer_token" \
  reembed \
  --workspace "$workspace_id" \
  --to-provider fake \
  --to-model fake-v2 \
  --to-kind semantic \
  --state accepted \
  --limit 20 \
  --dry-run

echo
echo "Reembed execute:"
openbrain_cli embed \
  --database-url "$db_url" \
  --token "$writer_token" \
  reembed \
  --workspace "$workspace_id" \
  --to-provider fake \
  --to-model fake-v2 \
  --to-kind semantic \
  --state accepted \
  --limit 20 \
  --max-objects 20 \
  --max-bytes 262144 \
  --actor demo-reembed

echo
echo "Coverage after target space fake/fake-v2:"
openbrain_cli embed \
  --database-url "$db_url" \
  --token "$writer_token" \
  coverage \
  --workspace "$workspace_id" \
  --provider fake \
  --model fake-v2 \
  --kind semantic \
  --state accepted \
  --missing-sample 5

pack_semantic="$(post_json "/v1/memory/pack" "$writer_token" "$(jq -nc --arg ws "$workspace_id" '
{
  scope: $ws,
  task_hint: "semantic pack in migrated embedding space",
  query: "database decision",
  semantic: true,
  embedding_provider: "fake",
  embedding_model: "fake-v2",
  embedding_kind: "semantic",
  budget_tokens: 800,
  top_k: 20
}')")"
echo
echo "Semantic pack (fake-v2) items_selected=$(echo "$pack_semantic" | jq -r '.pack.items_selected') budget_used=$(echo "$pack_semantic" | jq -r '.pack.budget_used')"

echo
echo "Demo complete."
echo "- Workspace seeded with lifecycle states and conflicts"
echo "- Memory pack shown at 400 / 1200 / 1600 budgets"
echo "- Policy deny explainability confirmed (reason_code + policy_rule_id)"
echo "- Coverage before/after and reembed workflow executed"
echo "- Tokens/report files:"
echo "  ${token_file}"
echo "  ${shadow_json}"
echo "  ${shadow_html}"
echo "- Next step: open ${base_url}/viewer and inspect workspace/audit/object panels."
