# Architecture Decision Record

## ADR-001: Tenant Isolation, Agent Loop Scheduling, and Observability

## Status

Proposed

## Context

This project implements a multi-tenant agent platform. The platform is responsible for creating agents, running agent loops, exposing tenant-specific tools, and allowing tenants to query task execution traces.

The core architectural concern is tenant isolation. A tenant must only be able to access its own resources throughout the full lifecycle of agent usage.

Tenant isolation must cover at least the following areas:

- Data isolation
- Tool schema isolation
- Agent loop lifecycle isolation
- Task context isolation
- Trace lookup isolation
- Idempotency isolation

The platform should not trust tenant identity directly provided by the client. All access to tenant-owned resources must be authenticated and resolved through an API key.

## Decision

### 1. Use a Global Tenant ID as the Isolation Boundary

Each tenant has a globally unique `tenant_id`.

All tenant-owned resources must include a `tenant_id` field, including but not limited to:

- Agents
- Tools
- Runs
- Tasks
- Trace events
- Idempotency records

The `tenant_id` is the primary logical boundary for resource ownership.

### 2. Resolve Tenant Identity from API Key

The client must not provide `tenant_id` as a trusted input.

Instead, the platform resolves the tenant identity from the API key attached to the request.

The resolved `tenant_id` is then used by the platform to:

- Filter resource queries
- Validate resource ownership
- Create new tenant-owned resources
- Select visible tool schemas
- Scope idempotency keys
- Scope trace lookup

### 3. Enforce Tenant Checks in CRUD, Configuration, and Runtime

Tenant ownership must be checked in all major platform operations:

- CRUD operations
- Agent configuration access
- Agent loop scheduling
- Tool schema exposure
- Tool execution
- Trace lookup
- Idempotency handling

This means tenant isolation is not only a database concern. It must also be enforced at the application logic layer.

### 4. Isolate Agent Loop Lifecycle per Tenant

The platform manages the full lifecycle of each agent run.

Clients should not manually drive the agent loop step by step. Instead, the client submits a task or run request, and the platform owns the execution lifecycle.

Each run or task belongs to exactly one tenant.

A task from one tenant must not be able to:

- Access another tenant's agent
- Use another tenant's tools
- Read another tenant's trace
- Share mutable runtime context with another tenant's task

Tasks from the same tenant should still have isolated runtime contexts, but they may share read-only tenant-owned resources when appropriate.

### 5. Scope Tool Schema by Tenant

The tools exposed to the LLM must be filtered by tenant.

A tenant should only see tool schemas that belong to that tenant or are explicitly allowed for that tenant.

This prevents the LLM from being given tool definitions that the current tenant should not know about or invoke.

### 6. Scope Trace Lookup by Tenant

Trace events are tenant-owned resources.

Trace lookup must be authenticated by API key and filtered by the resolved `tenant_id`.

A tenant can only query trace events belonging to its own runs and tasks.

Business trace data should be treated as private tenant data.

### 7. Scope Idempotency Key by Tenant

`Idempotency-Key` must be scoped per tenant.

The same idempotency key value from different tenants should not conflict.

The logical uniqueness should be based on a pair similar to:

```text
(tenant_id, idempotency_key)
```

This prevents one tenant's request idempotency record from affecting another tenant's request.

## Agent Loop Scheduling

## Context

The platform should support asynchronous agent execution.

Running an agent loop may involve multiple steps, including model calls, tool calls, trace generation, state transitions, retries, and final result persistence.

If the platform directly executes all tasks without control, one tenant may consume too many resources and affect other tenants.

Therefore, the agent loop needs a scheduling mechanism.

## Decision

### 1. Use a Task Pool for Agent Runs

The platform uses a task pool to schedule agent loop execution.

The task pool behaves similarly to a thread pool:

- When capacity is available, a task can be scheduled.
- When the pool is full, the platform may fail fast.
- The platform does not need to provide a complex queue in the demo version.

### 2. Use Two-Level Concurrency Limits

The platform applies two levels of concurrency control:

- Global maximum concurrency
- Per-tenant maximum concurrency

The global limit protects the whole platform.

The per-tenant limit prevents a single tenant from consuming all execution resources.

Example configuration:

```text
GLOBAL_MAX_CONCURRENCY = 32
PER_TENANT_MAX_CONCURRENCY = 8
```

### 3. Use Task State Machine to Prevent Duplicate Execution

Each task should have a clear lifecycle state.

Example states:

```text
pending -> running -> succeeded
pending -> running -> failed
pending -> running -> retrying -> running
```

The state machine prevents the same task from being executed repeatedly by mistake.

### 4. Support Retry with a Maximum Retry Count

Each task may have a `max_retry` value.

The default value can be:

```text
max_retry = 3
```

Retries are used to handle temporary failures, such as transient tool errors or model call failures.

Retry should not violate idempotency or tenant isolation.

## Observability

## Context

Observability has two different audiences:

1. Tenant-facing observability
2. Platform-facing observability

Tenant-facing observability allows tenants to inspect their own business execution data.

Platform-facing observability helps the platform operator understand system health and stability.

These two types of observability should be separated.

## Decision

### 1. Provide Tenant-Facing Business Trace

The platform records business trace events for each task or run.

A tenant can query its own trace data to understand:

- Agent execution steps
- Tool calls
- Model call results
- Errors
- Retry behavior
- Final task result

Trace records should include `tenant_id`.

Trace lookup must be authenticated and filtered by tenant.

### 2. Use JSON Payload for Trace Events

Business trace events can use a JSON payload.

This keeps the trace format flexible and easy to extend in the demo version.

Example trace event shape:

```json
{
  "tenant_id": "tenant_123",
  "task_id": "task_456",
  "event_type": "tool_call_started",
  "payload": {
    "tool_name": "search_docs",
    "input": {
      "query": "example"
    }
  }
}
```

### 3. Keep Platform Logs and Metrics Separate

The platform may also produce internal logs and metrics.

Platform logs and metrics are used for:

- Debugging
- Monitoring
- Alerting
- Capacity analysis
- Error diagnosis

They are not the same as tenant-facing business traces.

In the demo version, platform-side metrics and tracing can remain simple.

## Trade-offs

### Benefits

This design provides a clear and consistent tenant isolation model.

The main benefits are:

- Tenant-owned resources are easy to identify through `tenant_id`.
- Access control is enforced consistently through API-key-based tenant resolution.
- CRUD, runtime execution, tool exposure, and trace lookup share the same isolation boundary.
- Per-tenant concurrency limits reduce the risk of one tenant monopolizing platform resources.
- Business traces are treated as tenant-owned private data.

### Costs

This design requires every tenant-owned resource and query path to handle `tenant_id` carefully.

The implementation must avoid mistakes such as:

- Creating tenant-owned resources without `tenant_id`
- Querying resources without tenant filtering
- Trusting client-provided tenant identity
- Exposing all tool schemas to every tenant
- Looking up traces only by `task_id` without checking tenant ownership

### Boundary

This ADR does not solve all possible production-grade multi-tenant platform problems.

The current design does not fully address:

- Cross-region tenant placement
- Multi-database tenant sharding
- Resource redundancy
- Dynamic resource overbooking
- SLA-based scheduling
- API key revocation after leakage
- Fine-grained role-based access control within the same tenant
- Strong database-level isolation such as PostgreSQL Row-Level Security

These topics can be addressed in future ADRs if the platform evolves beyond the demo scope.

## Alternatives Considered

### 1. Trust Client-Provided Tenant ID

The platform could allow the client to send `tenant_id` directly in each request.

This option is rejected.

Reason:

- The client-provided tenant ID cannot be trusted.
- A malicious or buggy client could access another tenant's resources by changing the tenant ID.
- It makes authorization logic fragile.

### 2. No Per-Tenant Concurrency Limit

The platform could only use a global concurrency limit.

This option is rejected for the current design.

Reason:

- A single tenant could consume all available execution capacity.
- Other tenants could be starved.
- The platform would have weaker fairness guarantees.

### 3. Fully Dynamic Resource Overbooking

The platform could dynamically overbook resources and allocate idle capacity to high-traffic tenants.

This option is not selected for the demo version.

Reason:

- It adds scheduling complexity.
- It requires better metrics and capacity planning.
- It is more suitable for a production-grade platform with SLA requirements.

The demo version uses simpler global and per-tenant limits.

## Consequences

The implementation should ensure that tenant isolation is visible in the code structure.

Important implementation expectations:

- Request authentication resolves `tenant_id` from API key.
- Repository/database queries include tenant filters.
- Task scheduling checks both global and per-tenant concurrency limits.
- Tool schemas are selected based on tenant ownership.
- Trace lookup requires tenant ownership validation.
- Idempotency keys are unique per tenant.
- Runtime task context is isolated per task.

This keeps the platform simple enough for a demo while still demonstrating the most important multi-tenant platform design principles.
