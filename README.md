# Multi-Tenant Agent Platform

Rust workspace implementing a local multi-tenant agent execution platform plus a separate demo CLI.

The platform lets tenants register mock tools, create agents bound to explicit tool sets, start asynchronous agent runs, and inspect ordered execution traces. Tenant identity is derived from API keys, and the demo client exercises the server only through public HTTP APIs.

## Workspace layout

- `server/` — Axum + Tokio platform server binary: `platform`
- `demo/` — Clap-based demo CLI binary: `demo`
- `crates/platform-core/` — shared API/domain request and response types
- `server/migrations/` — PostgreSQL schema migrations
- `demo/examples/` — example tool schema and mock handler JSON
- `spec/` — server and demo client specifications
- `architecture_decision_record.md` — architecture decisions for tenant isolation, async execution, and traces
- `demo_tenant_isolation.sh` — end-to-end tenant isolation demo script

## Architecture overview

```text
┌──────────────┐       HTTP        ┌────────────────────────────────────┐
│  demo CLI    │ ────────────────▶ │ platform server (Axum + Tokio)    │
│ external API │                   │                                    │
└──────────────┘                   │  routes/http ─▶ db/runtime/trace   │
        ▲                          │       │              │             │
        │                          │       ▼              ▼             │
        │                          │  auth/API keys   scheduler         │
        │                          │  tenant context  mock agent loop   │
        │                          └──────────┬───────────┬─────────────┘
        │                                     │           │
        │                  shared DTOs        ▼           ▼
        └────────────── crates/platform-core  PostgreSQL  run_traces
```

The repository is split into a deployable platform server, an external demo client, and a small shared core crate. The `server` binary owns HTTP routing, authentication, persistence, scheduling, runtime execution, and trace recording. The `demo` binary is intentionally kept as an outside client boundary: it exercises tenant, API-key, tool, agent, run, and trace workflows only through public HTTP APIs. `platform-core` contains shared DTOs and domain types so the server and demo agree on request and response shapes without coupling the demo to server internals.

The server follows a thin-handler architecture. `server/src/main.rs` loads configuration, connects to PostgreSQL, optionally runs migrations, builds shared state, and starts Axum. `server/src/app.rs` wires routes and scheduler state. `server/src/http.rs` authenticates requests, performs request-shape validation, delegates business behavior to the database/runtime layers, and maps results into API responses.

Tenant isolation is the primary system invariant. API keys authenticate into a tenant context, tenant-owned database operations receive an explicit `tenant_id`, and tenant identity comes from the bearer token rather than request bodies. The schema uses shared tables with `tenant_id` plus composite foreign keys to keep tools, agents, runs, and trace rows tenant-consistent.

Agent runs are persisted before scheduling. The runtime uses global and per-tenant concurrency semaphores, fast-fails runs when capacity is unavailable, and only starts execution after the database transitions a run from `pending` to `running`. The mock agent loop is deterministic for repeatable demos and tests: it calls bound tools until enough tool results exist, then produces a final answer.

Execution observability is exposed through ordered traces stored in `run_traces`. Trace writes are centralized, sequence numbers are monotonic per run, and user-visible events include run creation, scheduling decisions, runtime start, LLM calls, tool execution, and run completion.

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

Optional server configuration:

```bash
export BIND_ADDR=127.0.0.1:3000
export RUST_LOG=info,sqlx=warn
export GLOBAL_MAX_CONCURRENCY=32
export PER_TENANT_MAX_CONCURRENCY=8
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

Compose starts PostgreSQL 16 and the platform server, runs migrations at startup, and exposes the server on `http://127.0.0.1:3000`.

## Common development commands

```bash
cargo check
cargo test
cargo fmt --all
cargo clippy --all-targets --all-features
```

Run a focused test:

```bash
cargo test -p platform-server runtime::tests::execute_mock_handler_returns_static_response -- --nocapture
cargo test -p platform-server runtime::tests::validate_tool_arguments_returns_error_for_schema_violation -- --nocapture
```

## Demo CLI commands

The demo CLI is an external client boundary: it talks to the platform over HTTP and does not access PostgreSQL or server internals.

All examples assume the server is running at `http://127.0.0.1:3000`.

### Create a tenant

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tenants create --name "Tenant A"
```

Save the returned `tenant_id`.

### Create an API key

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 api-keys create --tenant-id <tenant_id>
```

Save the returned `api_key`. The plaintext API key is returned only once.

### Register a tool

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create \
  --api-key <api_key> \
  --name search_web \
  --schema-file demo/examples/search_web.schema.json \
  --handler-file demo/examples/search_web.handler.json
```

Save the returned `tool_id`.

### List tools

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools list --api-key <api_key>
```

### Delete a tool

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools delete \
  --api-key <api_key> \
  --tool-id <tool_id>
```

### Create an agent

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 agents create \
  --api-key <api_key> \
  --name research_agent \
  --system-prompt "You are a research assistant." \
  --tool-id <tool_id>
```

`--name` is optional, `--tool-id` can be repeated, and `--max-iterations` defaults to `10`.

Save the returned `agent_id`.

### Get an agent

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 agents get \
  --api-key <api_key> \
  --agent-id <agent_id>
```

### Start a run

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs start \
  --api-key <api_key> \
  --agent-id <agent_id> \
  --message "Please search something." \
  --idempotency-key demo-run-1
```

`--idempotency-key` is optional. Reusing the same key for the same agent returns the same `run_id`.

Save the returned `run_id`.

### Get a run

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs get \
  --api-key <api_key> \
  --run-id <run_id>
```

### Wait for a run

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs wait \
  --api-key <api_key> \
  --run-id <run_id>
```

Optional wait flags:

```bash
--timeout-secs 30 --poll-secs 1
```

### Inspect a trace

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs trace \
  --api-key <api_key> \
  --run-id <run_id>
```

Trace events include `run_created`, `scheduler_decision`, `run_started`, `llm_call`, `tool_exec`, and `run_end`.

## Happy-path walkthrough

1. Create a tenant.
2. Create an API key for that tenant.
3. Register `search_web` with the example schema and handler.
4. Create an agent bound to the returned tool ID.
5. Start a run for that agent.
6. Wait for completion.
7. Inspect the trace.

Expected run result:

- `status` = `success`
- `final_answer` is present
- trace shows the deterministic mock loop: LLM call, tool execution, LLM call, final answer

## Idempotency walkthrough

Start a run with an idempotency key:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs start \
  --api-key <api_key> \
  --agent-id <agent_id> \
  --message "Please search something." \
  --idempotency-key demo-run-1
```

Run the same command again with the same `--idempotency-key`.

Expected result:

- both responses return the same `run_id`

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

## Tenant isolation checks

Tenant isolation is enforced by API key authentication and tenant-scoped resource access.

Manual checks:

- Create Tenant A and Tenant B.
- Register tools separately under each tenant's API key.
- Try to create a Tenant A agent with a Tenant B tool ID; the request should fail.
- Try to read a Tenant A run or trace with Tenant B's API key; the request should fail.

## Tenant isolation demo script

`demo_tenant_isolation.sh` runs an end-to-end isolation walkthrough against a running platform server. It creates two tenants, registers separate tools and agents, starts runs for each tenant, prints their traces, and verifies that cross-tenant trace reads fail.

```bash
./demo_tenant_isolation.sh
```

To target a different server URL, pass it as the first argument or set `BASE_URL`:

```bash
BASE_URL=http://127.0.0.1:3000 ./demo_tenant_isolation.sh
```

## Notes

- The demo client uses only public HTTP APIs.
- The demo client does not access PostgreSQL directly.
- API key plaintext is returned only once at creation time.
- Tenant identity comes from the bearer token, not request bodies for tenant-owned resources.
- Agents can only call tools explicitly bound to them.
- The invalid tool-call flow is triggered through normal message content, so the run request body remains aligned with the written spec.
