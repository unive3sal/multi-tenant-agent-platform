# 架构决策记录：多租户 Agent 平台

## 决策 1：通过租户作用域资源和服务端派生身份保证租户隔离

### 背景

这个平台允许多个 tenant 注册 tools、创建 agents、启动 agent runs，并查询执行 traces。核心风险是意外或恶意的跨租户访问：一个 tenant 不能读取其他 tenant 的 agents，不能绑定其他 tenant 的 tools，不能执行其他 tenant 的资源，也不能查看其他 tenant 的 traces。

客户端传入的 tenant 标识不能被信任。对于所有租户资源操作，tenant 身份都必须由服务端通过认证结果推导出来。同一条租户边界也必须一致地应用在 CRUD、agent 配置、运行时 tool 执行、幂等处理以及 trace 查询中。

### 权衡

一种方案是只使用全局唯一的资源 ID，并在应用层代码里检查资源归属。这会让 schema 更简单，但隔离正确性依赖每一条查询都写对。

另一种方案是在所有租户持有的资源中显式加入 `tenant_id`，并使用组合唯一约束和外键，例如 `(tenant_id, agent_id)` 和 `(tenant_id, tool_id)`。这会让 schema 稍微冗长一些，但可以让跨租户 join 和非法绑定更难被意外写出来。

更严格的生产级方案是使用 PostgreSQL Row-Level Security。它可以提供更强的数据库层隔离，但会增加配置和运维复杂度。对于这个本地 take-home 项目来说，这个复杂度没有必要。

### 决策

采用应用层认证加租户作用域数据建模。API key 只存储 hash，服务端根据 bearer token 推导 tenant，而不是相信请求体中的 tenant 信息。所有租户资源表都包含 `tenant_id`，agent-tool 绑定、runs、traces 等关系都通过带 tenant 的外键进行约束。

这个决策让实现保持轻量、可解释，同时在数据模型、service 层和运行时 agent loop 中都明确体现 tenant isolation。它也支持 per-tenant idempotency，因为 `Idempotency-Key` 的唯一性被限定在 `(tenant_id, agent_id, idempotency_key)` 范围内。

---

## 决策 2：使用有界并发的异步执行模型和显式 run 状态机

### 背景

启动一次 agent run 不应该要求客户端一步一步驱动 agent loop。在 `POST /agents/{id}/run` 之后，平台应该异步负责后续执行：调用 mock LLM、执行允许的 tools、追加 tool results、在得到 `final_answer` 时结束，并在超过 `max_iterations` 时失败。

但是异步执行会带来并发风险。多个 tenants 可能同时提交 runs；单个 tenant 可能占用所有运行资源；同一个 run 也不能因为重试或重复请求而被执行两次。

### 权衡

一种方案是在 HTTP 请求中同步执行完整 agent loop。这个方案简单，但会把客户端延迟和整个 run 的执行时间绑定在一起，也更容易遇到请求超时问题。

另一种方案是使用 Redis、Kafka 或托管任务系统等外部持久化队列。这会提升可靠性和恢复能力，但对于四小时本地 take-home 项目来说过重。

折中方案是使用基于 Tokio 的进程内 scheduler，配合全局有界并发、per-tenant 并发限制，以及数据库中的 run 状态。它没有外部队列那么强的持久化能力，但实现简单、本地可运行，也足以展示调度、公平性和隔离。

### 决策

使用进程内异步 scheduler。run 会先以 `pending` 状态持久化，然后 scheduler 尝试获取全局和 per-tenant 的执行容量，成功后再将其转换为 `running`。如果当前容量不足，平台可以快速失败，而不是维护一个无限增长的队列。

run 的状态转换是显式的：`pending -> running -> success`，或者 `pending/running -> failed`。每个 run 拥有独立的 message context、retry metadata、timestamps 和 trace sequence。运行时会强制执行 `max_iterations`，防止无限循环。这个设计让 agent loop 不阻塞客户端，同时限制资源竞争，并降低重复执行的风险。

---

## 决策 3：区分租户可见的执行 trace、平台日志和聚合指标

### 背景

平台需要面向两类不同对象提供可观测性。tenant 需要查看自己 agent run 的业务级 trace，以理解执行过程中发生了什么：LLM call、tool execution、final answer 或失败原因。平台维护者则需要实现层面的 logs 和聚合 metrics，用于 debugging、容量规划和告警。

这两类需求不应该混在一起。tenant-facing trace 必须按 tenant 隔离，并且可以安全地通过公开 API 返回。platform logs 可能包含内部实现细节，不应该成为 tenant API contract 的一部分。

### 权衡

一种方案是只依赖平台日志。这个方案最容易实现，但日志很难安全地开放给 tenant 查询，也不能提供稳定的 API 级 run 视图。

另一种方案是把所有底层内部事件都存入数据库。这可以提供最大细节，但可能暴露不必要的内部实现，并增加 schema 复杂度。

更平衡的方案是为每个 run 持久化一份紧凑、有序的业务 trace，同时把 platform logs 和 metrics 分开处理。trace 表只记录稳定事件类型，例如 `llm_call`、`tool_exec` 和 `run_end`，并使用单调递增的 seq 和 JSON payload。

### 决策

将 tenant-scoped run traces 作为一等数据持久化。每条 trace event 都属于 `(tenant_id, run_id)`，拥有唯一的 sequence number，并且只有在 API-key 认证确认调用方属于同一个 tenant 后才会被返回。

platform logs 用于内部 troubleshooting，aggregate metrics 用于监控 scheduler 行为、run 结果、延迟和失败数量。这个设计让 tenant-visible observability 保持简单且安全，同时保留未来加入生产级监控能力的空间，而不需要改变公开 trace API。
