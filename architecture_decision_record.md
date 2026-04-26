# Architecture Decision Record

## ADR-001: Tenant Isolation, Agent Loop Scheduling, and Observability

## Status

Proposed

## Tenant Isolation

### Context

Tenant isolation is the core security boundary of the platform. If the platform cannot consistently identify and enforce tenant ownership across data, tools, runtime execution, and trace lookup, a client may access another tenant's agents, tools, runs, or traces.

This is a decision point because tenant identity can be handled in multiple ways: the platform could trust tenant identifiers provided by clients, validate ownership only in selected API handlers, or make tenant identity an authenticated server-side concern enforced across every resource boundary. The platform chooses the stricter model because partial validation leaves cross-tenant access paths open.

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

### Trade-offs

Options considered:

1. Trust client-provided tenant identifiers.
   - Lower implementation cost, but insecure because clients can claim another tenant identity.
2. Validate tenant ownership only in HTTP handlers.
   - Simple at the API boundary, but insufficient because runtime execution, tool calls, trace lookup, and internal DB access can still become cross-tenant paths.
3. Resolve tenant identity from API keys and enforce `tenant_id` ownership across persistence, configuration, runtime, tools, idempotency, and trace lookup.
   - Higher implementation cost, but provides one consistent isolation boundary.

Pros of the chosen option:

- Provides safer tenant resource isolation than partial validation or no validation.
- Makes tenant ownership explicit through `tenant_id`.
- Avoids trusting client-provided identity fields.
- Keeps CRUD, configuration, runtime, tool exposure, idempotency, and trace lookup aligned around one isolation boundary.

Cons of the chosen option:

- Does not solve cross-region tenant placement.
- Does not solve multi-database sharding.
- Does not solve resource redundancy.
- Leaked API keys cannot currently be revoked.
- Every tenant-owned query and write path must explicitly carry and validate `tenant_id`, increasing implementation complexity.

### Decision

- Each tenant has a globally unique `tenant_id`.
- Tenant identity is resolved from the API key, not from client-provided request data.
- Every tenant-owned resource table includes a `tenant_id` field.
- CRUD operations must filter and validate resources by the resolved `tenant_id`.
- Agent configuration and tool binding must be validated against the resolved `tenant_id`.
- Agent loop scheduling and execution must enforce tenant ownership.
- Tool schemas passed to the LLM must only include tools visible to the current tenant and bound to the current agent.
- Trace lookup must require API-key authentication and tenant ownership validation.
- Idempotency keys are scoped per tenant.
- Runtime context is isolated per task, even when tasks belong to the same tenant.

This option is selected because it creates a single, explicit tenant boundary that can be applied consistently at storage, API, runtime, tool, idempotency, and trace layers. The additional implementation cost is acceptable because tenant isolation is a platform security invariant.

## Agent Loop

### Context

Clients should not manually drive the agent loop. The platform is responsible for managing the full lifecycle of a run.

Agent execution must be asynchronous. This introduces concurrency concerns, resource contention, scheduling fairness concerns, duplicate execution risk, and failure recovery questions.

This is a decision point because the platform could execute runs synchronously in request handlers, enqueue all runs for later processing, or use bounded execution capacity with fast failure. The platform needs predictable resource usage and tenant fairness more than unbounded acceptance of work.

### Trade-offs

Options considered:

1. Synchronous execution in the request path.
   - Simple lifecycle, but blocks clients, makes request latency unpredictable, and couples HTTP availability to long-running agent work.
2. Unbounded queueing.
   - Improves acceptance rate, but can accumulate uncontrolled backlog and hide capacity problems until recovery becomes harder.
3. Bounded task pool with global and per-tenant concurrency limits, using fast failure when capacity is full.
   - Provides predictable resource limits and tenant fairness, but rejects work during saturation.

Pros of the chosen option:

- Supports asynchronous execution.
- Reuses execution resources through a task pool instead of repeatedly creating execution capacity.
- Limits per-tenant concurrency to reduce malicious or accidental resource contention.
- Uses task state transitions to reduce duplicate execution risk.
- Fast failure prevents unbounded backlog growth.

Cons of the chosen option:

- Crash recovery during task execution is not fully addressed.
- Task priority is not addressed.
- A more elastic resource platform should allow controlled resource overbooking.
- Idle task pool capacity could be dynamically assigned to high-concurrency tenants in a more advanced design.
- Better hardware utilization, overbooking ratio, and SLA commitments are future concerns.
- Fast failure shifts some retry responsibility to clients.

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

This option is selected because it gives the platform predictable resource ceilings and clearer lifecycle ownership while still supporting asynchronous execution. The design intentionally favors bounded capacity and fairness over accepting unlimited work.

## Observability

### Context

Observability has two levels:

- Tenant-facing business data: each tenant can query its own business execution data, including the full task call chain, bottleneck analysis, and task monitoring.
- Platform-facing operational data: the platform can monitor stability, capacity, and failures.

These two kinds of observability must be separated.

This is a decision point because business traces contain tenant-private execution details, while platform logs and metrics are operational signals for service reliability. Mixing them would make access control unclear and could expose tenant data to platform-level workflows that do not need it.

### Trade-offs

Options considered:

1. Store only platform logs and derive tenant visibility from them.
   - Simpler implementation, but weak tenant-facing observability and unclear privacy boundary.
2. Store only tenant business traces.
   - Good for tenant debugging, but insufficient for platform stability, alerting, and capacity management.
3. Separate tenant-facing business traces from platform logs and aggregated metrics.
   - More moving parts, but clearer ownership, permissions, and operational usage.

Pros of the chosen option:

- Tenant trace lookup uses the same API-key-based tenant validation model.
- JSON payloads keep business trace events flexible and easy to evolve.
- Separating business traces from platform logs and metrics keeps tenant-facing observability distinct from platform operations.
- Aggregated platform metrics can support monitoring and alerting without exposing tenant trace details.

Cons of the chosen option:

- Tenant traces should be treated as private tenant assets; ideally, even the platform operator should not be able to view them directly.
- Platform-side metrics and traces are still simple in the current implementation.
- JSON payloads are flexible but have weaker schema guarantees, which may make long-term querying and analysis harder.

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

This option is selected because tenant-facing observability and platform-facing operations have different audiences, permissions, and retention needs. Keeping them separate preserves tenant privacy while still giving the platform enough aggregate signal for reliability work.
