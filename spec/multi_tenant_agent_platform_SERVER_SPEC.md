# Multi-Tenant Agent Platform Server Specification

## 1. Purpose

This document specifies a local, deployable multi-tenant agent execution platform.

The platform allows each tenant to register tools and agents, run agents asynchronously, execute mock tool calls, and inspect full execution traces. The implementation target is:

- Rust + Axum + Tokio
- SQLx + PostgreSQL
- Docker-packaged platform server

The platform server is the deployable backend service. It owns the HTTP API, database access, scheduler, agent loop runtime, tool execution, and trace writing. Demo/client behavior is specified separately.

---

## 2. Runtime Component

### 2.1 Platform Server

The platform server is a long-running backend process.

Responsibilities:

- Expose HTTP APIs.
- Authenticate API keys.
- Derive tenant context from API keys.
- Enforce tenant isolation.
- Manage tools, agents, runs, and traces.
- Run the bounded async scheduler.
- Execute agent loops.
- Validate tool call arguments.
- Execute mock tool handlers.
- Write structured traces and logs.

Local command:

```bash
cargo run --bin platform
```

Docker deployment must also be supported.

### 2.2 External Client Boundary

External clients, including the demo client, interact with the platform only through public HTTP APIs. They must not access the database directly, call platform internals, share process memory with the platform server, or execute agent loops locally.

---

## 3. Core Concepts

### Tenant

A tenant represents an enterprise customer.

Tenant identity is `tenant_id`. Tenant names are display fields and may be duplicated.

### API Key

An API key authenticates requests for a tenant.

Plaintext API keys are shown only once at creation time and must not be stored, logged, or written to traces.

### Tool

A tool is a tenant-owned callable capability.

A tool has:

- `name`
- `input_schema`
- `mock_handler`

`input_schema` is a JSON Schema used to validate tool call arguments.

`mock_handler` is a JSON configuration describing the mock result returned when the tool is executed.

### Agent

An agent is a tenant-owned reusable definition.

An agent has:

- `system_prompt`
- `max_iterations`
- explicitly bound tools

An agent is not execution state.

### Run

A run is one execution instance of an agent.

A run has its own isolated execution context:

- messages
- iteration count
- retry count
- trace sequence
- tool results
- final answer
- error reason
- status

`run_id` is also the task/execution ID.

### Trace

A trace is a structured, ordered record of a run lifecycle.

Trace events are tenant-owned data and must be queried with tenant scope.

---

## 4. Tenant Isolation Model

The platform uses shared database + shared tables + `tenant_id`.

All tenant-owned resources include `tenant_id`.

All access to tenant-owned resources must be tenant-scoped.

Incorrect:

```sql
SELECT * FROM tools WHERE id = $1;
```

Correct:

```sql
SELECT * FROM tools
WHERE tenant_id = $1
  AND id = $2;
```

Tenant identity comes from API key authentication, not request bodies.

Isolation is enforced by:

1. API key authentication deriving `tenant_id`.
2. `tenant_id` columns on tenant-owned tables.
3. Tenant-scoped repository queries.
4. Composite foreign keys for tenant consistency.
5. Agent-tool bindings.
6. Runtime tool authorization before tool execution.
7. Tenant-scoped trace reads.

Permission model:

```text
Tenant boundary:
  An agent cannot call tools from another tenant.

Agent capability boundary:
  An agent can only call tools explicitly bound to that agent.
```

---

## 5. Docker Packaging Requirements

The platform server must be buildable as a Docker image.

Docker goals:

- Package the platform server as an independent deployable image.
- Keep PostgreSQL in a separate container.
- Use the official PostgreSQL image in Docker Compose.
- Support both `linux/amd64` and `linux/arm64` platform images.
- Support local compose-based startup for platform + PostgreSQL.
- Support running the demo client against the Dockerized platform.
- Keep the demo client separate from the platform image unless an optional separate demo image is added.

The platform image must:

- Start only the platform server.
- Read configuration from environment variables.
- Connect to PostgreSQL through `DATABASE_URL`.
- Expose the HTTP API.
- Not embed PostgreSQL.
- Not require direct demo client access.
- Be buildable with Docker Buildx for multi-architecture publishing.

Recommended runtime configuration:

```text
DATABASE_URL
BIND_ADDR
RUST_LOG
GLOBAL_MAX_CONCURRENCY
PER_TENANT_MAX_CONCURRENCY
RUN_MIGRATIONS
```

Migration behavior must be explicit. Either:

1. The platform runs migrations at startup when enabled, or
2. Migrations are run as a separate deployment step before starting the platform.

A lightweight health endpoint should be provided:

```http
GET /healthz
```

---

## 6. Database Schema

### 6.1 Extension

```sql
CREATE EXTENSION IF NOT EXISTS pgcrypto;
```

### 6.2 `tenants`

```sql
CREATE TABLE tenants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

`name` is not unique.

### 6.3 `api_keys`

```sql
CREATE TABLE api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,

    key_prefix TEXT NOT NULL,
    key_hash TEXT NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ NULL
);

CREATE UNIQUE INDEX uq_api_keys_key_prefix
ON api_keys (key_prefix);

CREATE INDEX idx_api_keys_tenant_id
ON api_keys (tenant_id);

CREATE INDEX idx_api_keys_active_prefix
ON api_keys (key_prefix)
WHERE revoked_at IS NULL;
```

### 6.4 `tools`

```sql
CREATE TABLE tools (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,

    name TEXT NOT NULL,
    input_schema JSONB NOT NULL,
    mock_handler JSONB NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ NULL,

    UNIQUE (tenant_id, id),

    CHECK (jsonb_typeof(input_schema) = 'object'),
    CHECK (jsonb_typeof(mock_handler) = 'object')
);

CREATE UNIQUE INDEX uq_tools_tenant_name_active
ON tools (tenant_id, name)
WHERE deleted_at IS NULL;

CREATE INDEX idx_tools_tenant_active
ON tools (tenant_id)
WHERE deleted_at IS NULL;
```

### 6.5 `agents`

```sql
CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id UUID NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,

    name TEXT NULL,
    system_prompt TEXT NOT NULL,
    max_iterations INTEGER NOT NULL DEFAULT 10,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ NULL,

    UNIQUE (tenant_id, id),

    CHECK (max_iterations > 0),
    CHECK (max_iterations <= 50)
);

CREATE UNIQUE INDEX uq_agents_tenant_name_active
ON agents (tenant_id, name)
WHERE deleted_at IS NULL AND name IS NOT NULL;

CREATE INDEX idx_agents_tenant_active
ON agents (tenant_id)
WHERE deleted_at IS NULL;
```

### 6.6 `agent_tools`

```sql
CREATE TABLE agent_tools (
    tenant_id UUID NOT NULL,
    agent_id UUID NOT NULL,
    tool_id UUID NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (tenant_id, agent_id, tool_id),

    FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents(tenant_id, id)
        ON DELETE CASCADE,

    FOREIGN KEY (tenant_id, tool_id)
        REFERENCES tools(tenant_id, id)
        ON DELETE CASCADE
);
```

### 6.7 `runs`

```sql
CREATE TABLE runs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    tenant_id UUID NOT NULL,
    agent_id UUID NOT NULL,

    status TEXT NOT NULL DEFAULT 'pending',

    input_messages JSONB NOT NULL,
    final_answer TEXT NULL,
    error_reason TEXT NULL,

    idempotency_key TEXT NULL,

    max_retries INTEGER NOT NULL DEFAULT 3,
    retry_count INTEGER NOT NULL DEFAULT 0,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ NULL,
    finished_at TIMESTAMPTZ NULL,

    UNIQUE (tenant_id, id),

    FOREIGN KEY (tenant_id, agent_id)
        REFERENCES agents(tenant_id, id)
        ON DELETE RESTRICT,

    CHECK (jsonb_typeof(input_messages) = 'array'),
    CHECK (status IN ('pending', 'running', 'success', 'failed')),
    CHECK (max_retries >= 0),
    CHECK (max_retries <= 10),
    CHECK (retry_count >= 0)
);

CREATE UNIQUE INDEX uq_runs_idempotency
ON runs (tenant_id, agent_id, idempotency_key)
WHERE idempotency_key IS NOT NULL;

CREATE INDEX idx_runs_tenant
ON runs (tenant_id);

CREATE INDEX idx_runs_tenant_agent
ON runs (tenant_id, agent_id);

CREATE INDEX idx_runs_tenant_status
ON runs (tenant_id, status);
```

### 6.8 `run_traces`

```sql
CREATE TABLE run_traces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    tenant_id UUID NOT NULL,
    run_id UUID NOT NULL,

    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    payload JSONB NOT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    FOREIGN KEY (tenant_id, run_id)
        REFERENCES runs(tenant_id, id)
        ON DELETE CASCADE,

    UNIQUE (tenant_id, run_id, seq),

    CHECK (seq > 0),

    CHECK (
        type IN (
            'run_created',
            'scheduler_decision',
            'run_started',
            'llm_call',
            'tool_exec',
            'retry',
            'run_end'
        )
    ),

    CHECK (jsonb_typeof(payload) = 'object')
);

CREATE INDEX idx_run_traces_tenant_run_seq
ON run_traces (tenant_id, run_id, seq);
```

---

## 7. HTTP API

### Public APIs

```http
POST /tenants
POST /tenants/{tenant_id}/api-keys
GET  /healthz
```

`POST /tenants`

Request:

```json
{
  "name": "Acme"
}
```

Response:

```json
{
  "tenant_id": "uuid",
  "name": "Acme"
}
```

`POST /tenants/{tenant_id}/api-keys`

Response:

```json
{
  "api_key": "map_live_xxx_yyy",
  "warning": "This key is shown only once. Store it securely."
}
```

### Authenticated APIs

All authenticated APIs require:

```http
Authorization: Bearer <api_key>
```

#### Tools

```http
POST   /tools
GET    /tools
DELETE /tools/{tool_id}
```

`POST /tools`

```json
{
  "name": "search_web",
  "input_schema": {
    "type": "object",
    "properties": {
      "query": { "type": "string" }
    },
    "required": ["query"]
  },
  "mock_handler": {
    "type": "static",
    "response": {
      "result": "mock search result"
    }
  }
}
```

Rules:

- Client must not send `tenant_id`.
- Tool is created under the authenticated tenant.
- Tool name is unique only within the same tenant among active tools.

#### Agents

```http
POST /agents
GET  /agents/{agent_id}
```

`POST /agents`

```json
{
  "name": "research_agent",
  "system_prompt": "You are a research assistant.",
  "tool_ids": ["uuid"],
  "max_iterations": 10
}
```

Rules:

- Client must not send `tenant_id`.
- Every tool ID must reference an active tool under the authenticated tenant.
- Agent-tool bindings are written to `agent_tools`.

#### Runs

```http
POST /agents/{agent_id}/run
GET  /runs/{run_id}
GET  /runs/{run_id}/trace
```

`POST /agents/{agent_id}/run`

Headers:

```http
Authorization: Bearer <api_key>
Idempotency-Key: optional-key
```

Body:

```json
{
  "messages": [
    {
      "role": "user",
      "content": "Please search something."
    }
  ]
}
```

Response:

```json
{
  "run_id": "uuid",
  "status": "pending"
}
```

Rules:

- Agent must belong to the authenticated tenant.
- If `Idempotency-Key` matches an existing run in the same tenant and agent scope, return the existing run.
- Otherwise create a new run and submit it to the scheduler.

---

## 8. Authentication

Authentication flow:

1. Read `Authorization: Bearer <api_key>`.
2. Extract key prefix.
3. Query active API key by prefix.
4. Verify plaintext key against stored hash.
5. Build `TenantContext { tenant_id }`.
6. Use `TenantContext` for all tenant-owned operations.

Clients must not provide `tenant_id` for tenant-owned resources.

---

## 9. Idempotency

Idempotency scope:

```text
tenant_id + agent_id + idempotency_key
```

Behavior:

- Repeated requests with the same scope return the same `run_id`.
- The agent loop must not execute twice for the same idempotency key.
- If the first result failed, repeated requests still return that same failed run.

Concurrent create-or-get must rely on the database unique index:

```sql
CREATE UNIQUE INDEX uq_runs_idempotency
ON runs (tenant_id, agent_id, idempotency_key)
WHERE idempotency_key IS NOT NULL;
```

Implementation requirement:

1. Try to insert the run in a transaction.
2. If insert succeeds, return the new run.
3. If unique conflict occurs, query and return the existing run.

---

## 10. Scheduler

The scheduler controls asynchronous run execution.

Requirements:

- Enforce global maximum concurrency.
- Enforce per-tenant maximum concurrency.
- Do not queue runs in the current version.
- Fast-fail when capacity is unavailable.

Capacity failures:

```text
scheduler_capacity_exceeded
tenant_concurrency_limit_exceeded
```

Run statuses:

```text
pending
running
success
failed
```

Valid transitions:

```text
pending -> running -> success
pending -> running -> failed
pending -> failed
```

Before executing a run, the runtime must atomically transition it:

```sql
UPDATE runs
SET status = 'running',
    started_at = now()
WHERE id = $1
  AND tenant_id = $2
  AND status = 'pending';
```

If no row is updated, the task must exit without executing the run.

---

## 11. Agent Loop

Execution flow:

```text
1. Load run.
2. Recover tenant_id from run.
3. Transition pending -> running.
4. Load agent by tenant_id + agent_id.
5. Load bound tools by tenant_id + agent_id.
6. Create run-local context.
7. Append run_started trace.
8. Repeat until final answer or max_iterations:
   a. Call mock LLM.
   b. Append llm_call trace.
   c. If final_answer:
      - mark run success
      - append run_end trace
      - stop
   d. If tool_call:
      - authorize tool call
      - validate tool arguments
      - execute mock handler
      - append tool_exec trace
      - append tool result to local messages
9. If max_iterations is exceeded:
   - mark run failed
   - error_reason = max_iterations_exceeded
   - append run_end trace
```

### Mock LLM

For demo stability, mock LLM should be deterministic:

```text
If tool result count < 2:
  return a tool_call using the first bound tool.
Else:
  return final_answer.
```

The mock LLM must not use global mutable counters.

### Runtime Tool Authorization

Before executing a tool, verify:

1. Tool belongs to the run's tenant.
2. Tool is active.
3. Tool is bound to the run's agent.

If authorization fails:

```text
tool_not_allowed_for_agent
cross_tenant_tool_call_rejected
```

### Tool Argument Validation

Before executing a mock handler:

```text
tool_call.arguments must conform to tool.input_schema
```

If validation fails:

```text
invalid_tool_arguments
```

This error is not retryable.

### Mock Handler

First supported handler:

```json
{
  "type": "static",
  "response": {
    "result": "mock search result"
  }
}
```

Execution returns `mock_handler.response`.

---

## 12. Retry

`max_iterations` and `max_retries` are separate.

```text
max_iterations:
  Limits agent loop iterations.

max_retries:
  Handles transient runtime failures.
```

Retryable examples:

```text
llm_timeout
mock_handler_transient_error
db_transient_error
timeout
```

Non-retryable examples:

```text
cross_tenant_tool_call_rejected
tool_not_allowed_for_agent
invalid_tool_arguments
max_iterations_exceeded
agent_not_found
tool_deleted
```

Retries must be visible in trace:

```json
{
  "type": "retry",
  "payload": {
    "operation": "llm_call",
    "attempt": 2,
    "reason": "llm_timeout",
    "backoff_ms": 200
  }
}
```

If retry budget is exhausted:

```text
retry_exhausted
```

---

## 13. Observability

### Business Trace

`run_traces` is the user-facing trace mechanism.

Trace event types:

```text
run_created
scheduler_decision
run_started
llm_call
tool_exec
retry
run_end
```

Every trace event includes:

```text
tenant_id
run_id
seq
type
payload
created_at
```

Trace reads must be tenant-scoped:

```sql
SELECT seq, type, payload, created_at
FROM run_traces
WHERE tenant_id = $1
  AND run_id = $2
ORDER BY seq;
```

Final run status and final `run_end` trace must agree.

### Trace Safety

Trace payloads must not contain:

- API keys
- secrets
- credentials
- unredacted sensitive payloads

### Structured Logs

The platform should emit structured logs with:

```text
tenant_id
run_id
agent_id
event
status
error_reason
duration_ms
```

---

## 14. Error Reasons

Run error reasons:

```text
scheduler_capacity_exceeded
tenant_concurrency_limit_exceeded
agent_not_found
tool_not_allowed_for_agent
cross_tenant_tool_call_rejected
invalid_tool_arguments
tool_deleted
mock_handler_failed
llm_failed
retry_exhausted
max_iterations_exceeded
internal_error
```

Suggested HTTP errors:

```text
401 invalid_api_key
403 cross_tenant_access_denied
404 resource_not_found
409 duplicate_name
422 invalid_tool_arguments
429 tenant_concurrency_limit_exceeded
503 scheduler_capacity_exceeded
```

---

## 15. Test Requirements

### Tenant Isolation

- Tenant A cannot list Tenant B's tools.
- Tenant A cannot fetch Tenant B's agent.
- Tenant A cannot fetch Tenant B's run.
- Tenant A cannot fetch Tenant B's trace.
- Tenant A cannot bind Tenant B's tool to its agent.

### Agent Capability

- An agent can call a bound tool.
- An agent cannot call an unbound tool.
- Deleted tools cannot be executed.

### Idempotency

- Repeated requests with the same `Idempotency-Key` return the same `run_id`.
- Concurrent requests with the same key create only one run.
- Different tenants can use the same key without conflict.

### Agent Loop

- Mock LLM behavior is deterministic.
- Run succeeds when final answer is returned.
- Run fails with `max_iterations_exceeded` when the limit is reached.
- Run fails with `invalid_tool_arguments` for schema violations.

### Observability

- Trace events are ordered.
- Final run status matches final `run_end`.
- Scheduler fast-fail decisions are traced.
- Retry attempts are traced.
- Trace queries are tenant-scoped.

### External Client Boundary

- External clients can complete workflows using only public HTTP APIs.
- External clients do not require direct database access.
- Platform server remains running while multiple client commands are executed.

### Docker Packaging

- Platform image builds successfully.
- Platform can run with official PostgreSQL image.
- Platform image supports both `linux/amd64` and `linux/arm64`.
