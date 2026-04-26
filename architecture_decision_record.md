# Architecture Decision Record: Multi-Tenant Agent Platform

## Decision 1: Enforce tenant isolation through tenant-scoped resources and server-derived identity

### Context

The platform allows multiple tenants to register tools, create agents, start agent runs, and inspect execution traces. The core risk is accidental or malicious cross-tenant access: a tenant must not be able to read another tenant's agents, bind another tenant's tools, execute another tenant's resources, or inspect another tenant's traces.

Client-provided tenant identifiers cannot be trusted for tenant-owned operations. Tenant identity must be derived from authentication, and the same tenant boundary must be enforced consistently during CRUD operations, agent configuration, runtime tool execution, idempotency handling, and trace lookup.

### Tradeoff

One option is to use globally unique resource IDs only and check ownership in application code. This keeps the schema simple, but it makes isolation dependent on every query being written correctly.

Another option is to make `tenant_id` part of every tenant-owned resource and use composite uniqueness and foreign keys such as `(tenant_id, agent_id)` and `(tenant_id, tool_id)`. This adds some schema verbosity, but it makes cross-tenant joins and invalid bindings harder to express accidentally.

A stricter production option would be PostgreSQL Row-Level Security. That would provide stronger database-level isolation, but it adds operational complexity and is unnecessary for this local take-home implementation.

### Decision

Use application-level authentication plus tenant-scoped database modeling. API keys are stored as hashes, and the server derives the tenant from the bearer token rather than trusting request bodies. Tenant-owned tables include `tenant_id`, and relationships such as agent-tool bindings, runs, and traces are constrained by tenant-aware foreign keys.

This decision keeps the implementation explainable and lightweight while still making tenant isolation explicit in the data model, service layer, and runtime agent loop. It also supports per-tenant idempotency by scoping `Idempotency-Key` uniqueness to `(tenant_id, agent_id, idempotency_key)`.

---

## Decision 2: Execute agent runs asynchronously with bounded concurrency and explicit run state transitions

### Context

Starting an agent run should not require the client to drive the loop step by step. After `POST /agents/{id}/run`, the platform is responsible for running the loop asynchronously: call the mock LLM, execute allowed tools, append tool results, stop on `final_answer`, and fail on `max_iterations`.

However, asynchronous execution introduces concurrency risks. Multiple tenants may submit runs at the same time, one tenant could consume all runtime capacity, and the same run must not be executed twice because of retries or duplicate requests.

### Tradeoff

One option is to execute the full agent loop synchronously inside the HTTP request. This is simple, but it ties client latency to the whole run and makes timeout behavior worse.

Another option is to use a durable external queue such as Redis, Kafka, or a managed job system. That would improve reliability and recovery, but it is too heavy for a four-hour local take-home project.

A middle-ground option is an in-process Tokio-based scheduler with bounded global concurrency, per-tenant concurrency limits, and database-backed run states. This is less durable than an external queue, but it is simple, local, and enough to demonstrate scheduling, fairness, and isolation.

### Decision

Use an in-process asynchronous scheduler. A run is persisted first with status `pending`, then the scheduler attempts to acquire global and per-tenant capacity before transitioning it to `running`. If capacity is unavailable, the platform can fail fast rather than maintaining an unbounded queue.

Run state transitions are explicit: `pending -> running -> success` or `pending/running -> failed`. Each run has its own message context, retry metadata, timestamps, and trace sequence. `max_iterations` is enforced by the runtime to prevent infinite loops. This design keeps the agent loop non-blocking for clients while limiting resource contention and making duplicate execution easier to prevent.

---

## Decision 3: Separate tenant-facing execution traces from platform logs and aggregate metrics

### Context

The platform needs observability for two different audiences. Tenants need a business-level trace of their own agent runs so they can understand what happened: LLM call, tool execution, final answer, or failure reason. The platform operator also needs implementation-level logs and aggregate metrics for debugging, capacity planning, and alerting.

These concerns should not be mixed. Tenant-facing traces must be scoped by tenant and safe to expose through the public API. Platform logs may contain operational details and should not become part of the tenant API contract.

### Tradeoff

One option is to rely only on platform logs. This is easy to implement, but logs are hard for tenants to query safely and do not provide a stable API-level view of a run.

Another option is to store every low-level internal event in the database. That provides maximum detail, but it can expose unnecessary internals and increase schema complexity.

A balanced option is to persist a compact, ordered business trace per run, while keeping platform logs and metrics separate. The trace table records stable event types such as `llm_call`, `tool_exec`, and `run_end`, with a monotonic sequence number and JSON payload.

### Decision

Persist tenant-scoped run traces as first-class data. Each trace event belongs to `(tenant_id, run_id)`, has a unique sequence number, and is returned only after API-key authentication confirms the caller belongs to the same tenant.

Use platform logs for internal troubleshooting and aggregate metrics for monitoring scheduler behavior, run outcomes, latency, and failure counts. This keeps tenant-visible observability simple and safe while leaving room to add production-grade monitoring later without changing the public trace API.
