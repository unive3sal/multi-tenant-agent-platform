#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-${1:-http://127.0.0.1:3000}}"
RUN_TAG="tenant-isolation-$(date +%s)"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT

json_get() {
  python3 -c 'import json, sys; print(json.load(sys.stdin)[sys.argv[1]])' "$1"
}

run_demo() {
  cargo run --quiet --bin demo -- --base-url "$BASE_URL" "$@"
}

print_step() {
  printf '\n==> %s\n' "$1"
}

write_tool_files() {
  local name="$1"
  local response="$2"

  cat > "$TEMP_DIR/$name.schema.json" <<'JSON'
{
  "type": "object",
  "properties": {
    "query": { "type": "string" }
  },
  "required": ["query"]
}
JSON

  cat > "$TEMP_DIR/$name.handler.json" <<JSON
{
  "type": "static",
  "response": {
    "result": "$response"
  }
}
JSON
}

if ! curl -fsS "$BASE_URL/healthz" >/dev/null; then
  printf 'Platform server is not reachable at %s. Start it with DATABASE_URL set, then run this script again.\n' "$BASE_URL" >&2
  exit 1
fi

cd "$ROOT_DIR"
write_tool_files "tenant_a_search" "tenant A private search result"
write_tool_files "tenant_b_lookup" "tenant B private lookup result"

print_step "Create two tenants"
tenant_a_json="$(run_demo tenants create --name "Tenant A $RUN_TAG")"
tenant_b_json="$(run_demo tenants create --name "Tenant B $RUN_TAG")"
printf 'Tenant A:\n%s\n' "$tenant_a_json"
printf 'Tenant B:\n%s\n' "$tenant_b_json"
tenant_a_id="$(printf '%s' "$tenant_a_json" | json_get tenant_id)"
tenant_b_id="$(printf '%s' "$tenant_b_json" | json_get tenant_id)"

print_step "Create one API key per tenant"
api_key_a_json="$(run_demo api-keys create --tenant-id "$tenant_a_id")"
api_key_b_json="$(run_demo api-keys create --tenant-id "$tenant_b_id")"
api_key_a="$(printf '%s' "$api_key_a_json" | json_get api_key)"
api_key_b="$(printf '%s' "$api_key_b_json" | json_get api_key)"
printf 'Tenant A API key created.\nTenant B API key created.\n'

print_step "Register different tools in each tenant"
tool_a_json="$(run_demo tools create --api-key "$api_key_a" --name tenant_a_search --schema-file "$TEMP_DIR/tenant_a_search.schema.json" --handler-file "$TEMP_DIR/tenant_a_search.handler.json")"
tool_b_json="$(run_demo tools create --api-key "$api_key_b" --name tenant_b_lookup --schema-file "$TEMP_DIR/tenant_b_lookup.schema.json" --handler-file "$TEMP_DIR/tenant_b_lookup.handler.json")"
printf 'Tenant A tool:\n%s\n' "$tool_a_json"
printf 'Tenant B tool:\n%s\n' "$tool_b_json"
tool_a_id="$(printf '%s' "$tool_a_json" | json_get tool_id)"
tool_b_id="$(printf '%s' "$tool_b_json" | json_get tool_id)"

print_step "List tools from each tenant to show separate catalogs"
printf 'Tools visible with Tenant A key:\n'
run_demo tools list --api-key "$api_key_a"
printf 'Tools visible with Tenant B key:\n'
run_demo tools list --api-key "$api_key_b"

print_step "Create one agent per tenant, bound only to that tenant's tool"
agent_a_json="$(run_demo agents create --api-key "$api_key_a" --name tenant_a_agent --system-prompt "You are Tenant A's isolated agent." --tool-id "$tool_a_id")"
agent_b_json="$(run_demo agents create --api-key "$api_key_b" --name tenant_b_agent --system-prompt "You are Tenant B's isolated agent." --tool-id "$tool_b_id")"
printf 'Tenant A agent:\n%s\n' "$agent_a_json"
printf 'Tenant B agent:\n%s\n' "$agent_b_json"
agent_a_id="$(printf '%s' "$agent_a_json" | json_get agent_id)"
agent_b_id="$(printf '%s' "$agent_b_json" | json_get agent_id)"

print_step "Start independent runs in both tenants"
run_a_json="$(run_demo runs start --api-key "$api_key_a" --agent-id "$agent_a_id" --message "Tenant A task: search for private tenant A context." --idempotency-key "$RUN_TAG-a")"
run_b_json="$(run_demo runs start --api-key "$api_key_b" --agent-id "$agent_b_id" --message "Tenant B task: lookup private tenant B context." --idempotency-key "$RUN_TAG-b")"
printf 'Tenant A run started:\n%s\n' "$run_a_json"
printf 'Tenant B run started:\n%s\n' "$run_b_json"
run_a_id="$(printf '%s' "$run_a_json" | json_get run_id)"
run_b_id="$(printf '%s' "$run_b_json" | json_get run_id)"

print_step "Wait for both tenant runs"
printf 'Tenant A final run:\n'
run_demo runs wait --api-key "$api_key_a" --run-id "$run_a_id" --timeout-secs 30 --poll-secs 1
printf 'Tenant B final run:\n'
run_demo runs wait --api-key "$api_key_b" --run-id "$run_b_id" --timeout-secs 30 --poll-secs 1

print_step "Print traces for each tenant"
printf 'Tenant A trace:\n'
run_demo runs trace --api-key "$api_key_a" --run-id "$run_a_id"
printf 'Tenant B trace:\n'
run_demo runs trace --api-key "$api_key_b" --run-id "$run_b_id"

print_step "Tenant isolation checks: cross-tenant trace reads should fail"
set +e
cross_a_output="$(run_demo runs trace --api-key "$api_key_b" --run-id "$run_a_id" 2>&1)"
cross_a_status=$?
cross_b_output="$(run_demo runs trace --api-key "$api_key_a" --run-id "$run_b_id" 2>&1)"
cross_b_status=$?
set -e

if [[ "$cross_a_status" -eq 0 || "$cross_b_status" -eq 0 ]]; then
  printf 'Isolation check failed: a cross-tenant trace read unexpectedly succeeded.\n' >&2
  printf 'Tenant B reading Tenant A run output:\n%s\n' "$cross_a_output" >&2
  printf 'Tenant A reading Tenant B run output:\n%s\n' "$cross_b_output" >&2
  exit 1
fi

printf 'Tenant B cannot read Tenant A trace, as expected:\n%s\n' "$cross_a_output"
printf 'Tenant A cannot read Tenant B trace, as expected:\n%s\n' "$cross_b_output"
printf '\nTenant isolation demo completed successfully.\n'
