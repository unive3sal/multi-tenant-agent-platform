# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Common commands

- Build/check the whole workspace:
  - `cargo check`
- Run tests:
  - `cargo test`
- Run a single test:
  - `cargo test -p platform-server runtime::tests::execute_mock_handler_returns_static_response -- --nocapture`
  - `cargo test -p platform-server runtime::tests::validate_tool_arguments_returns_error_for_schema_violation -- --nocapture`
- Format:
  - `cargo fmt --all`
- Lint:
  - `cargo clippy --all-targets --all-features`

## Running the system

- Start PostgreSQL and set env vars:
  - `export DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/platform`
  - `export RUN_MIGRATIONS=true`
- Run the platform server:
  - `cargo run --bin platform`
- Health check:
  - `curl http://127.0.0.1:3000/healthz`
- Run with Docker Compose:
  - `docker compose up --build`

## Demo CLI

The demo client is a separate binary and must only talk to the server over HTTP.

- Create a tenant:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 tenants create --name "Tenant A"`
- Create an API key:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 api-keys create --tenant-id <tenant_id>`
- Register a tool:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create --api-key <api_key> --name search_web --schema-file demo/examples/search_web.schema.json --handler-file demo/examples/search_web.handler.json`
- Create an agent:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 agents create --api-key <api_key> --name research_agent --system-prompt "You are a research assistant." --tool-id <tool_id>`
- Start a run:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs start --api-key <api_key> --agent-id <agent_id> --message "Please search something." --idempotency-key demo-run-1`
- Wait for a run:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs wait --api-key <api_key> --run-id <run_id>`
- Inspect a trace:
  - `cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs trace --api-key <api_key> --run-id <run_id>`

## High-level architecture

This is a Rust workspace with three main parts:

- `server/`: deployable Axum/Tokio backend binary (`platform`)
- `demo/`: separate Clap-based CLI (`demo`) used for manual workflow verification against public HTTP APIs
- `crates/platform-core/`: shared request/response DTOs and small domain types used by both binaries

### Server request flow

- `server/src/main.rs` loads env config, opens the Postgres pool, optionally runs migrations from `server/migrations`, builds shared app state, and starts Axum.
- `server/src/app.rs` wires the HTTP routes and stores runtime-wide scheduler state in `AppState`.
- `server/src/http.rs` keeps handlers thin. It authenticates the request, validates basic request shape, delegates DB/runtime work, and maps results into API DTOs.

### Tenant isolation model

Tenant isolation is a core invariant of the codebase.

- API keys authenticate into `TenantContext` in `server/src/auth.rs`.
- Tenant identity always comes from the bearer token, never from request bodies for tenant-owned resources.
- Repository functions in `server/src/db.rs` are expected to take `tenant_id` explicitly for tenant-owned lookups and writes.
- The schema in `server/migrations/0001_init.sql` uses shared tables with `tenant_id`, plus composite foreign keys to preserve tenant consistency across `agent_tools`, `runs`, and `run_traces`.

When editing API or DB code, preserve tenant scoping first; it is more important than convenience.

### Persistence and schema

The database schema is intentionally close to the spec:

- `tenants`, `api_keys`, `tools`, `agents`, `agent_tools`, `runs`, `run_traces`
- `tools` and `agents` are soft-deleted via `deleted_at`
- run idempotency is enforced by a unique partial index on `(tenant_id, agent_id, idempotency_key)`
- traces are ordered by `(tenant_id, run_id, seq)`

Most business behavior is implemented with plain SQL in `server/src/db.rs`, not through an ORM abstraction.

### Run scheduling and execution

The interesting runtime behavior lives in `server/src/runtime.rs`.

- `start_run` persists the run first, then calls `schedule_run`.
- `schedule_run` uses a global semaphore plus a per-tenant semaphore map from `AppState`.
- Capacity is fast-fail: if a permit cannot be acquired, the run is marked failed immediately and trace events are written.
- Actual execution starts only after the DB transition from `pending -> running` succeeds.

The mock agent loop is deterministic:

- if tool result count is below 2, it returns a tool call for the first bound tool
- otherwise it returns a final answer

That behavior is important for repeatable manual verification and focused tests.

### Tool authorization and invalid-call verification

Tool authorization is enforced at runtime, not just when creating the agent.

- The runtime checks that the tool exists for the tenant, is not deleted, and is explicitly bound to the agent before execution.
- The demo’s invalid tool-call verification relies on registering extra tools under the same tenant and then creating an agent bound to only a subset of them.
- The `--force-invalid-tool-call` demo flag does **not** add fields to the run request body. Instead it embeds a sentinel in normal message content, and `server/src/runtime.rs` detects that to intentionally attempt an unbound active tool.

Keep the public run request body spec-shaped: `messages` only.

### Traces and observability

- User-visible execution traces are stored in `run_traces`.
- `server/src/trace.rs` centralizes trace appends and assigns monotonically increasing `seq` values per run.
- The server writes `run_created`, `scheduler_decision`, `run_started`, `llm_call`, `tool_exec`, and `run_end` events.

If you change run behavior, also check whether the trace sequence and final `run_end` payload still match the stored run status.

### Error handling

- `server/src/error.rs` is the central HTTP error mapping layer.
- Server code uses `thiserror`-style typed errors and converts them into spec-style error codes.
- `demo/src/client.rs` treats any non-2xx response as an error and prints status + body, so API error shapes are part of the effective CLI UX.

## Repository-specific guidance

- Keep public API request/response shapes strictly aligned with the written spec unless the user explicitly approves a deviation.
- Do not make the demo CLI depend on database access or server internals; it is an external client boundary by design.
- Prefer keeping HTTP handlers thin and pushing tenant-scoped business logic into `db.rs`, `runtime.rs`, and `trace.rs`.
