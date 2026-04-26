# Architecture Decision Record

## tenant isolation

### context

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

### decision

* 每一个 tenant 都有全局唯一的 tenant id
* 每个 tenant 的资源表都需要包含 tenant id 字段
* 对于 CRUD 和 agent loop 调度，以及 LLM 返回的过程都应该强制检查 tenant 的归属关系

### tradeoff

pros.  

* 比起不完全的校验或者无校验能更安全的实现租户资源隔离

cons.  

* 无法处理跨区域，多库，以及资源冗余的问题
* 泄漏的 api-key 暂时没法 revoke

## agent loop

### context

客户端不应该手动驱动 agent loop。平台负责管理 run 的完整生命周期

### decision

要求异步，需要考虑并发问题，这里就会引入资源竞争以及调度公平性

* 设计任务池，类似于线程池，当池子空闲时对 task 进行调度
* 池满时可以直接走快速失败，不需要排队
* 限定每个租户的最大并发数，防止资源倾斜
* 每个 task 不能互相干扰，且需要隔离上下文
* 状态机防止一个 task 被重复执行
* 每个任务会有 max_retry（默认值为3），以此应对可能的失败
* 两层并发，global 和 per tenant 都有最大并发限制

### tradeoff

pros.  

* 实现了异步调用问题
* 任务池可以资源复用，避免反复创建
* 限制每个用户并发数，防止恶意争抢

cons.   

* 如果在 task 执行的过程中宕机如何恢复
* task 间应当有优先级
* 实际上一个具有弹性的资源平台应当是允许资源超卖的，能够动态调度一些空闲的任务池给到高并发量的用户
* 如何更好的利用现有的平台硬件资源，提供一定超卖比，承诺 SLA


## Observability

### context

两个层面的可观测性：
* 租户可以随时查询自己的业务数据，包括每个 task 的全调用链路，能够分析瓶颈并进行任务监控
* 平台自己的监控指标，用于平台的稳定性治理

### decision

* 租户可以查询的 business trace
* 平台本身的 platform logs
* 给平台监控和告警用的聚合指标 metrics

### tradeoff

pros.  

* trace lookup 同样需要 api-key 校验
* business trace event 直接用 json payload

cons.  

* 租户 trace 应当是租户自己的私有资产，平台方应当都无权进行查看
* 由于时间关系，平台侧的 metric 和 trace 实现的还较为简陋（几乎没有）
