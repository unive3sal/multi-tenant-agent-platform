# Architecture Decision Record

## ADR-001: Tenant Isolation, Agent Loop Scheduling, and Observability

## Status

Proposed

## Tenant Isolation

### Context

Tenant isolation must cover at least the following areas:

- Data isolation
- Tool isolation
- Agent loop lifecycle isolation
- Trace isolation

Every operation from resource creation, agent loop execution, to trace lookup must strictly access only the current tenant's own resources. Cross-tenant access is not allowed.

The platform must never trust the client to identify itself. Access to all agent resources must be authenticated by API key, and tenant identity must be resolved from that API key.

All tenant-owned resources should include a `tenant_id` field.

Tenant matching must be checked across CRUD operations, configuration, and runtime execution.

The agent loop lifecycle itself must also be isolated. Tool schemas exposed to the LLM must be isolated by tenant. Trace lookup must also require `tenant_id` scoping.

`Idempotency-Key` must be generated and checked per tenant.

Different tasks under the same tenant must have isolated runtime context, but may share read-only resources.

### Decision

- Each tenant has a globally unique `tenant_id`.
- Every tenant-owned resource table includes a `tenant_id` field.
- Tenant identity is resolved from the API key, not from client-provided request data.
- CRUD operations must filter and validate resources by the resolved `tenant_id`.
- Agent configuration and tool binding must be validated against the resolved `tenant_id`.
- Agent loop scheduling and execution must enforce tenant ownership.
- Tool schemas passed to the LLM must only include tools visible to the current tenant and bound to the current agent.
- Trace lookup must require API-key authentication and tenant ownership validation.
- Idempotency keys are scoped per tenant.
- Runtime context is isolated per task, even when tasks belong to the same tenant.

### Trade-offs

Pros:

- Provides safer tenant resource isolation than partial validation or no validation.
- Makes tenant ownership explicit through `tenant_id`.
- Keeps CRUD, configuration, runtime, tool exposure, idempotency, and trace lookup aligned around one isolation boundary.

Cons:

- Does not solve cross-region tenant placement.
- Does not solve multi-database sharding.
- Does not solve resource redundancy.
- Leaked API keys cannot currently be revoked.

## Agent Loop

### Context

Clients should not manually drive the agent loop. The platform is responsible for managing the full lifecycle of a run.

Agent execution must be asynchronous. This introduces concurrency concerns, resource contention, and scheduling fairness concerns.

### Decision

- Use a task pool, similar to a thread pool, to schedule agent loop tasks when execution capacity is available.
- When the pool is full, fail fast instead of queueing indefinitely.
- Apply a maximum concurrency limit per tenant to prevent one tenant from monopolizing resources.
- Apply two-level concurrency control:
  - Global maximum concurrency
  - Per-tenant maximum concurrency
- Each task must have isolated runtime context and must not interfere with other tasks.
- Use a task state machine to prevent the same task from being executed more than once.
- Each task has a `max_retry` setting, with a default value of `3`, to handle temporary failures.

Example task lifecycle:

```text
pending -> running -> succeeded
pending -> running -> failed
pending -> running -> retrying -> running
```

### Trade-offs

Pros:

- Supports asynchronous execution.
- Reuses execution resources through a task pool instead of repeatedly creating execution capacity.
- Limits per-tenant concurrency to reduce malicious or accidental resource contention.
- Uses task state transitions to reduce duplicate execution risk.

Cons:

- Crash recovery during task execution is not fully addressed.
- Task priority is not addressed.
- A more elastic resource platform should allow controlled resource overbooking.
- Idle task pool capacity could be dynamically assigned to high-concurrency tenants in a more advanced design.
- Better hardware utilization, overbooking ratio, and SLA commitments are future concerns.

## Observability

### Context

Observability has two levels:

- Tenant-facing business data: each tenant can query its own business execution data, including the full task call chain, bottleneck analysis, and task monitoring.
- Platform-facing operational data: the platform can monitor stability, capacity, and failures.

These two kinds of observability must be separated.

### Decision

- Provide tenant-facing business traces.
- Provide platform logs for platform operation and debugging.
- Provide aggregated metrics for platform monitoring and alerting.
- Business trace events use JSON payloads for flexibility.
- Business trace lookup requires API-key authentication and tenant ownership validation.

Example business trace event:

```json
{
  "tenant_id": "tenant_123",
  "run_id": "run_456",
  "event_type": "tool_exec",
  "payload": {
    "tool_name": "search_web",
    "status": "succeeded"
  }
}
```

### Trade-offs

Pros:

- Tenant trace lookup uses the same API-key-based tenant validation model.
- JSON payloads keep business trace events flexible and easy to evolve.
- Separating business traces from platform logs and metrics keeps tenant-facing observability distinct from platform operations.

Cons:

- Tenant traces should be treated as private tenant assets; ideally, even the platform operator should not be able to view them directly.
- Platform-side metrics and traces are still simple in the current implementation.
