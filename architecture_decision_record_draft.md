# Architecture Decision Record

## tenant isolation

### context

租户隔离是平台最核心的安全边界。如果不能在数据、工具、运行时和 trace 查询中稳定识别并限制租户身份，任意一个客户端都可能越权访问其他租户的 agent、tool、run 或 trace。因此这里需要明确平台如何识别租户、如何保存租户归属关系，以及在资源创建、配置、调度和查询时如何执行隔离检查。

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
* API-key 是租户身份来源，客户端请求体中的 tenant id 不能作为鉴权依据
* 每个 tenant 的资源表都需要包含 tenant id 字段
* 对于 CRUD 和 agent loop 调度，以及 LLM 返回的过程都应该强制检查 tenant 的归属关系
* tool schema 只暴露当前 tenant 和当前 agent 可见的工具
* trace 查询和 idempotency-key 都按 tenant 进行隔离

选择这个方案是因为它把租户边界放在统一的 tenant id 和 API-key 鉴权模型上，能够让 CRUD、配置、运行时、工具暴露和 trace 查询使用同一套隔离规则，减少只在某一层做校验导致的越权风险。

### tradeoff

pros.  

* 比起不完全的校验或者无校验能更安全的实现租户资源隔离
* 租户归属关系显式存储在资源表中，便于查询和审计
* API-key 解析租户身份可以避免信任客户端传入的身份字段

cons.  

* 无法处理跨区域，多库，以及资源冗余的问题
* 泄漏的 api-key 暂时没法 revoke
* 每个 CRUD，配置和运行路径都需要显式传递和检查 tenant_id，开发复杂度更高
* 单库共享表会让 tenant_id 过滤成为所有查询的强约束，遗漏校验会带来安全风险

## agent loop

### context

客户端不应该手动驱动 agent loop。平台负责管理 run 的完整生命周期。这里需要决策平台如何调度异步 agent 任务、如何限制并发、如何避免单个租户占用全部资源，以及如何避免一个 task 被重复执行。

### decision

要求异步，需要考虑并发问题，这里就会引入资源竞争以及调度公平性

* 设计任务池，类似于线程池，当池子空闲时对 task 进行调度
* 池满时可以直接走快速失败，不需要排队
* 限定每个租户的最大并发数，防止资源倾斜
* 每个 task 不能互相干扰，且需要隔离上下文
* 状态机防止一个 task 被重复执行
* 每个任务会有 max_retry（默认值为3），以此应对可能的失败
* 两层并发，global 和 per tenant 都有最大并发限制

选择这个方案是因为平台当前更需要可预测的资源上限和清晰的生命周期管理，而不是无限排队或动态超卖。任务池加两层并发限制可以在实现复杂度可控的前提下，同时解决异步执行、资源复用和租户公平性问题。

### tradeoff

pros.  

* 实现了异步调用问题
* 任务池可以资源复用，避免反复创建
* 限制每个用户并发数，防止恶意争抢
* 快速失败让系统不会因为无限排队而积压不可控状态
* 状态机可以降低 task 被重复执行的风险

cons.   

* 如果在 task 执行的过程中宕机如何恢复
* task 间应当有优先级
* 实际上一个具有弹性的资源平台应当是允许资源超卖的，能够动态调度一些空闲的任务池给到高并发量的用户
* 如何更好的利用现有的平台硬件资源，提供一定超卖比，承诺 SLA
* 快速失败会牺牲一部分可用性，调用方需要自己重试或稍后再提交


## Observability

### context

可观测性需要同时满足租户和平台两类需求。租户需要看到自己的 task 执行链路、瓶颈和运行状态；平台需要监控系统稳定性、容量和错误。这里需要决策哪些数据暴露给租户，哪些数据只用于平台治理，以及 trace 查询是否仍然要遵守租户隔离。

两个层面的可观测性：
* 租户可以随时查询自己的业务数据，包括每个 task 的全调用链路，能够分析瓶颈并进行任务监控
* 平台自己的监控指标，用于平台的稳定性治理

### decision

* 租户可以查询的 business trace
* 平台本身的 platform logs
* 给平台监控和告警用的聚合指标 metrics
* business trace event 使用 json payload
* business trace lookup 需要 api-key 校验和 tenant_id 隔离

选择这个方案是因为 business trace 和 platform logs/metrics 的受众、权限和用途不同。将它们分开可以避免把租户私有运行数据混入平台运维数据，同时仍然保留平台排障和容量治理所需的聚合信号。

### tradeoff

pros.  

* trace lookup 同样需要 api-key 校验
* business trace event 直接用 json payload
* 租户可观测数据和平台运维数据的边界更清晰
* json payload 方便扩展不同类型的 trace event

cons.  

* 租户 trace 应当是租户自己的私有资产，平台方应当都无权进行查看
* 由于时间关系，平台侧的 metric 和 trace 实现的还较为简陋（几乎没有）
* json payload 灵活但 schema 约束较弱，长期可能影响查询和分析质量
