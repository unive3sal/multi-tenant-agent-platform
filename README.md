# Multi-Tenant Agent Platform

Rust workspace implementing a local multi-tenant agent execution platform plus a separate demo CLI.

## Workspace layout

- `server/` — Axum + Tokio platform server binary: `platform`
- `demo/` — Clap-based demo CLI binary: `demo`
- `crates/platform-core/` — shared API/domain types
- `server/migrations/` — PostgreSQL schema migrations
- `demo/examples/` — example tool schema and mock handler JSON

## Requirements

- Rust toolchain
- PostgreSQL

Optional:
- Docker / Docker Compose

## Run locally with Cargo

Start PostgreSQL first, then set:

```bash
export DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/platform
export RUN_MIGRATIONS=true
```

Start the server:

```bash
cargo run --bin platform
```

Health check:

```bash
curl http://127.0.0.1:3000/healthz
```

## Run with Docker Compose

```bash
docker compose up --build
```

## Demo CLI commands

Examples:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tenants create --name "Tenant A"

cargo run --bin demo -- --base-url http://127.0.0.1:3000 api-keys create --tenant-id <tenant_id>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create \
  --api-key <api_key> \
  --name search_web \
  --schema-file demo/examples/search_web.schema.json \
  --handler-file demo/examples/search_web.handler.json

cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools list --api-key <api_key>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 agents create \
  --api-key <api_key> \
  --name research_agent \
  --system-prompt "You are a research assistant." \
  --tool-id <tool_id>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs start \
  --api-key <api_key> \
  --agent-id <agent_id> \
  --message "Please search something." \
  --idempotency-key demo-run-1

cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs wait --api-key <api_key> --run-id <run_id>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs trace --api-key <api_key> --run-id <run_id>
```

## Minimal invalid tool-call walkthrough

This verifies runtime authorization failure while keeping the run request body spec-shaped (`messages` only).

### 1. Create a tenant

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tenants create --name "Tenant A"
```

Save the returned `tenant_id`.

### 2. Create an API key

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 api-keys create --tenant-id <tenant_id>
```

Save the returned `api_key`.

### 3. Register two mock tools under the same tenant

Create the first tool:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create \
  --api-key <api_key> \
  --name search_web \
  --schema-file demo/examples/search_web.schema.json \
  --handler-file demo/examples/search_web.handler.json
```

Create a second tool with the same schema/handler but a different name:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create \
  --api-key <api_key> \
  --name audit_log \
  --schema-file demo/examples/search_web.schema.json \
  --handler-file demo/examples/search_web.handler.json
```

Save both returned tool IDs.

### 4. Create an agent bound to only one tool

Bind only `search_web`:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 agents create \
  --api-key <api_key> \
  --name research_agent \
  --system-prompt "You are a research assistant." \
  --tool-id <search_web_tool_id>
```

Save the returned `agent_id`.

### 5. Start a run that requests the invalid-call verification flow

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs start \
  --api-key <api_key> \
  --agent-id <agent_id> \
  --message "Please search something." \
  --force-invalid-tool-call \
  --idempotency-key invalid-tool-demo-1
```

Save the returned `run_id`.

### 6. Wait for completion

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs wait \
  --api-key <api_key> \
  --run-id <run_id>
```

Expected result:

- `status` = `failed`
- `error_reason` = `tool_not_allowed_for_agent`

### 7. Inspect the trace

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs trace \
  --api-key <api_key> \
  --run-id <run_id>
```

You should see the run end in a failed authorization path after the runtime attempts a tool that belongs to the tenant but is not bound to the agent.

## Notes

- The demo client uses only public HTTP APIs.
- The demo client does not access PostgreSQL directly.
- API key plaintext is returned only once at creation time.
- The invalid tool-call flow is triggered through normal message content, so the run request body remains aligned with the written spec.
