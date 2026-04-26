# Multi-Tenant Agent Platform Demo Client Specification

## 1. Purpose

This document specifies the separate demo CLI/TUI client for the Multi-Tenant Agent Platform.

The demo client is a separate Rust binary that exercises the platform through public HTTP APIs. It is used for manual step-by-step validation, local development testing, Loom/demo recording, tenant isolation verification, idempotency verification, and agent loop trace inspection.

The demo client must not:

- Access PostgreSQL directly
- Call platform runtime internals
- Share process memory with the platform server
- Execute agent loops locally

---

## 2. Runtime Relationship

The platform server must be running before the demo client is used.

Start the platform server with Cargo:

```bash
cargo run --bin platform
```

or run the platform server through Docker Compose.

Then run demo commands from another terminal:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 <command>
```

The demo client talks to the platform only through HTTP APIs.

---

## 3. Demo Client Specification

The demo client must support manual step-by-step execution.

A one-command full scenario may exist, but manual mode is required.

### Required Manual Commands

The exact CLI names may vary, but the client must support these operations:

```text
tenants create
api-keys create
tools create
tools list
agents create
agents get
runs start
runs get
runs wait
runs trace
```

Example command shape:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 tenants create --name "Tenant A"

cargo run --bin demo -- --base-url http://127.0.0.1:3000 api-keys create --tenant-id <tenant_id>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 tools create \
  --api-key <api_key> \
  --name search_web \
  --schema-file examples/search_web.schema.json \
  --handler-file examples/search_web.handler.json

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

cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs wait \
  --api-key <api_key> \
  --run-id <run_id>

cargo run --bin demo -- --base-url http://127.0.0.1:3000 runs trace \
  --api-key <api_key> \
  --run-id <run_id>
```

### Required Verification Flows

The demo client must allow manual verification of:

1. Full agent loop:
   ```text
   LLM call -> tool exec -> LLM call -> final answer
   ```

2. Tenant isolation:
   ```text
   Tenant A cannot use Tenant B's tool.
   Tenant A cannot read Tenant B's run trace.
   ```

3. Idempotency:
   ```text
   Starting the same run twice with the same Idempotency-Key returns the same run_id.
   ```

4. Runtime tool authorization:
   ```text
   A forced invalid tool call is rejected during agent loop execution.
   ```

### Optional Full Scenario

Optional convenience command:

```bash
cargo run --bin demo -- --base-url http://127.0.0.1:3000 scenario full
```

This command may run the full happy-path and isolation checks automatically, but it must not replace manual commands.

---

## 4. Demo Test Requirements

### Manual CLI Operation

- The demo client can create a tenant with a manual command.
- The demo client can create an API key with a manual command.
- The demo client can register tools and agents with manual commands.
- The demo client can start, inspect, wait for, and trace a run with manual commands.

### Tenant Isolation Verification

- The demo client can show that Tenant A cannot use Tenant B's tool.
- The demo client can show that Tenant A cannot read Tenant B's run trace.
- The demo client prints expected failure reasons clearly.

### Idempotency Verification

- The demo client can start the same run twice with the same `Idempotency-Key`.
- The demo client clearly shows that both responses return the same `run_id`.

### Runtime Authorization Verification

- The demo client can trigger or simulate an invalid runtime tool call.
- The demo client can show that the platform rejects the call during agent loop execution.

### Platform Boundary

- The demo client can complete all flows using only public HTTP APIs.
- The demo client does not require database credentials.
- The demo client works against both a Cargo-run platform and a Dockerized platform.
