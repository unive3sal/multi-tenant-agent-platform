# Architecture Decision Record

## tenant isolation

租户隔离至少要做到：

* 数据隔离
* 工具隔离
* agent loop 的生命周期隔离
* trace 隔离

> 每个从创建，到运行 agent loop，查询运行 trace，都只能严格的访问自己的资源，不允许越级访问;
>
> 永远不要相信客户端说自己是谁，访问一切 agent 的资源都需要鉴权（用 API-key）
>
> 所有由租户持有的资源都应该有 tenant_id 字段
>
> 租户匹配应当在 CRUD，配置和运行时都进行检查
>
> Agent loop 本身的生命周期也需要隔离
>
> 给到 LLM 的 tools schema 也需要隔离
>
> trace 查询也需要 tenant_id
>
> Idempotency-Key 需要 per tenant 进行生成配置
>
> 同一个 tenant 的不同 task 需要隔离上下文，但是可以共享只读资源

## agent loop

要求异步，需要考虑并发问题，这里就会引入资源竞争以及调度公平性

* 设计调度池，类似于线程池，当池子空闲时对 task 进行调度
* 池满时可以直接走快速失败，不需要排队
* 限定每个租户的最大并发数，防止资源倾斜
* 每个 task 不能互相干扰，且需要隔离上下文
* 状态机防止一个 task 被重复执行

每个任务会有 max_retry（默认值为3），以此应对可能的失败

## Observability

可观测性的设计，要求每个租户的每个 task 的调用全链路都可追溯。有利于平台监控和 troubleshooting。至少应该包含：

* 租户可以查询的 business trace
* 平台本身的 platform logs
* 给平台监控和告警用的聚合指标 metrics
