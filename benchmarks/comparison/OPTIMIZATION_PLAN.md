# Tysel 性能优化计划

本计划以 TypeScript 7、Linux arm64 完整对比结果为基线。目标是持续改善 Tysel 自身的吞吐、尾延迟与 CPU 效率，同时保持已经有优势的启动速度和内存占用；不把超过某个外部运行时作为发布前提。

## 当前阶段：持续负载稳定性与发布验收

后续优化不再以 C100 总吞吐作为多 worker 的单一决策依据，按以下顺序执行：

1. 在独占 Linux 主机对 64 KiB JSON/bytes 运行至少 120 秒的持续负载，关联每秒吞吐、CPU 频率、Tysel 各线程 CPU 和 `perf` QuickJS/native 热点。先解释并消除首尾时段下降，再修改运行时。
2. 以 `requests-per-server-cpu-second-p50` 作为主指标；总吞吐、延迟、内存和客户端 CPU 作为护栏。默认回归门禁只阻止 Tysel CPU 效率回退。
3. 修复后，在同一 commit、相同二进制和锁定 TypeScript 7 工具链下，分别于独占 Linux x86_64、arm64 主机完成三次四-seed record cycle。CPU 效率相对 spread 不超过 10%，吞吐和延迟不超过 15%。
4. 三轮均稳定后，将相同服务端二进制复制到独立服务主机，以另一台负载机复验。负载机 CPU 容量使用率必须低于 75%，否则不能排除客户端饱和，也不能形成官网结论。

诊断轨道会使用符号化但 release-equivalent 的 `profiling` profile；正式横向对比仍无 `perf`、频率或线程采样探针。两类数据不得合并计分。

## 基线结论

基线证据：`target/benchmark-comparison/typescript7-linux-arm64-rerun-seed2.json`。

| 指标 | Tysel | 当前最佳对照 | 判断 |
| --- | ---: | ---: | --- |
| 启动 p50 | 20.17 ms | Bun 19.21 ms | 已接近最佳，作为回归护栏 |
| 空闲 PSS | 11.73 MiB | Bun 13.74 MiB | 当前领先，作为回归护栏 |
| health C1 | 4,203 req/s | Deno 12,277 req/s | 固定请求开销偏高 |
| health C10 | 13,113 req/s | Bun 46,332 req/s | 固定开销和尾延迟共同影响 |
| health C100 | 50,748 req/s | Deno 49,641 req/s | 吞吐充足，但 CPU 成本偏高 |
| JSON 64 KiB C10 | 3,144 req/s | Bun 20,847 req/s | 序列化、复制和单 worker 是主要候选 |
| bytes 64 KiB C10 | 6,388 req/s | Deno 28,362 req/s | 响应传输路径存在明显优化空间 |
| health C10 p99 | 40.63 ms | 对照均低于 0.6 ms | 约 40 ms 的周期性停顿必须优先定位 |

`health C10` 中 68,919 个请求有 954 个超过 30 ms，且最大值为 42.49 ms。这个分布与 TCP delayed ACK/Nagle 相互作用高度吻合，而服务端接受连接后没有设置 `TCP_NODELAY`。这是高置信度假设，不是已经证明的根因，必须用单变量实验确认。

代码审查还确认了以下固定开销：

- trusted profile 固定创建 1 个 QuickJS worker；
- 每个请求都会重新 `eval` Request 工厂函数；
- 普通空 GET 仍创建请求体 channel 和 Tokio 转发任务；
- 非 WebSocket 请求也会创建两组 WebSocket channel；
- 成功响应统一通过 `mpsc<Vec<u8>>` 流式传输，缓冲响应没有长度提示，并经历字符串、`Vec<u8>`、channel、`Bytes` 的转换。

## 第一轮实施结果（Linux arm64，seed 2）

第一轮已实现服务端 `TCP_NODELAY`、缓存 QuickJS Request 工厂，以及空 body/非 WebSocket 请求快路径。实现是通用 runtime 路径，没有按 URL、workload、响应大小或 benchmark adapter 特判。

新证据：`target/benchmark-comparison/phase1-linux-arm64-seed2.json`。以下结果与旧基线使用相同完整矩阵、runtime 版本和 order seed；TypeScript 实际版本均为 7.0.2，但重装 Linux arm64 平台包后 `.bin/tsc` wrapper 哈希发生变化。因此这些结果足以做内部工程判断，却不满足聚合器的严格同工具链哈希条件，也不能代替重新建立的四 seed 基线。

| 指标 | 旧基线 | 第一轮 | 变化 | 判断 |
| --- | ---: | ---: | ---: | --- |
| health C1 | 4,203 req/s | 5,678 req/s | +35.1% | 明显改善 |
| health C10 | 13,113 req/s | 38,690 req/s | +195.0% | 明显改善 |
| health C10 p99 | 40.63 ms | 0.47 ms | -98.8% | 40 ms 停顿已消失 |
| health C100 | 50,748 req/s | 62,192 req/s | +22.6% | 明显改善 |
| health C10 CPU 效率 | 基线 | 新结果 | +42.3% | 吞吐增益并非只来自增加 CPU |
| JSON 64 KiB C10 | 3,144 req/s | 3,078 req/s | -2.1% | 在 ±5% 噪声带内 |
| bytes 64 KiB C10 | 6,388 req/s | 5,840 req/s | -8.6% | 需要更多 seed，暂按潜在回退处理 |
| 启动 p50 | 20.17 ms | 18.06 ms | -10.5% | 无回退；同场 peer 也有改善 |
| 空闲 PSS | 11.73 MiB | 11.57 MiB | -1.3% | 基本稳定 |

完整真实场景测试通过：QuickJS 106 项、Hono 集成 2 项、runtime 63 项，以及服务/打包集成测试；HTTP body、chunked body limit、HTTP/1 keep-alive、HTTP/2、双协议和 WebSocket 路径均通过。`cargo clippy`（warnings as errors）、格式和 diff 检查也通过。

第一轮结论：`TCP_NODELAY` 假设得到强支持，中低并发请求路径已获得实际收益；64 KiB payload 仍受响应 channel、复制和单 worker 限制。正式验收应在当前工具链安装上重新建立 before/after 四 seed 配对基线，并把 bytes 64 KiB 的下降视为待解决风险，而不是忽略。

### Buffered response 直通实验

第二轮为标量字符串/JSON 响应增加 direct buffered body 和精确 `size_hint`；数组分块响应仍保留 streaming channel 与背压，WebSocket 使用独立通道。证据：`target/benchmark-comparison/phase2-buffered-linux-arm64-seed2.json`。

相对第一轮，health 基本持平；bytes 64 KiB C10/C100 吞吐分别提升 12.1%/12.9%，CPU 效率分别提升 13.8%/16.3%，p99 分别下降 12.1%/16.2%。JSON 64 KiB C10 提升 2.3%，C100 提升 6.2%，但 C1 下降 6.7%，C100 p99 上升 15.1%。同场 peer 多数变化处于 ±5%，因此保留直通路径，但 JSON C1/C100 尾延迟必须在 seed 1、3、4 中复核。

相对最初基线，第二轮的 bytes 64 KiB C10/C100 为 +2.5%/+2.9%，JSON 64 KiB 吞吐约持平；由于上述工具链 wrapper 哈希差异，这组跨基线百分比仅作方向参考。第一轮与第二轮之间的工具链哈希一致，可确认 direct body 改善了高并发字节响应的 CPU 效率，但它没有解决 QuickJS JSON 和单 worker 的总体吞吐上限。

### 显式多 worker 实验

第三轮增加 `[server].workers`，仅允许 `service` profile 使用 1–64，默认值仍为 1。每个 worker 是独立 QuickJS isolate，拥有独立 JavaScript 全局状态和独立运行时内存上限，因此该能力只适合无状态服务或已将状态外置的应用；没有按 workload 或 URL 自动切换 worker 数。

同一 Linux arm64 容器、同一 release 二进制和同一 TypeScript 7.0.2 launcher 下完成 1/2/4 worker 全矩阵。证据分别为 `workers-1-linux-arm64-full.json`、`workers-2-linux-arm64-full.json` 和 `workers-4-linux-arm64-full.json`。

| 指标 | 1 worker | 2 workers | 变化 | 4 workers | 判断 |
| --- | ---: | ---: | ---: | ---: | --- |
| 启动 p50 | 15.38 ms | 15.88 ms | +3.3% | 19.35 ms | 2 worker 在护栏内，4 worker 明显增加启动成本 |
| 空闲 PSS | 11.27 MiB | 11.74 MiB | +4.2% | 13.14 MiB | 增量可控，但不是零成本 |
| health C100 | 66,398 req/s | 74,781 req/s | +12.6% | 64,148 req/s | 2 worker 有收益，4 worker 回退 |
| JSON 1 KiB C100 | 47,139 req/s | 65,486 req/s | +38.9% | 56,993 req/s | 2 worker 最均衡 |
| JSON 64 KiB C100 | 3,236 req/s | 6,290 req/s | +94.4% | 10,167 req/s | 大响应随并行度扩展 |
| bytes 64 KiB C100 | 6,634 req/s | 11,904 req/s | +79.4% | 18,252 req/s | 大响应随并行度扩展 |
| health C100 p99 | 2.43 ms | 3.36 ms | +38.1% | 6.43 ms | 多 worker 的小响应尾延迟仍需优化 |
| bytes 64 KiB C100 p99 | 15.96 ms | 13.59 ms | -14.9% | 22.52 ms | 2 worker 改善，4 worker 反而恶化 |

结论：保留显式 opt-in 配置，推荐无状态、高并发 payload 服务先从 2 worker 验证；不改变默认值，也不把 4 worker 宣传为通用更快。下一步应把 `max_in_flight` 真正接入 HTTP 入口并比较最短队列/共享入口调度，重点降低 2 worker 的小响应 C100 p99，而不是继续盲目增加 isolate。

### `max_in_flight` 背压实现

`limits.max_in_flight` 已从 manifest 贯通 TAP、生产 service、`tysel dev` 和热重载。HTTP 入口使用 semaphore 立即负载卸载：容量耗尽返回结构化 `503 OVERLOADED` 与 `Retry-After: 1`，不创建无界等待队列。许可持有到 buffered/streaming body 结束或被丢弃；WebSocket upgrade 则持有到连接关闭。旧 TAP 缺少该字段时保持历史默认值 1000，配置为 0 时可作为拒绝全部 HTTP 请求的 circuit breaker。

正确性验证覆盖并发超限、恢复、streaming body 完整生命周期和 WebSocket 生命周期，完整 runtime 65 项与 CLI 回归通过。正常负载性能护栏使用 2 worker、`max_in_flight = 1024`，证据为 `admission-workers-2-linux-arm64-full.json`。相对接入前的同机 2 worker 结果：health C1/C10/C100 分别为 -0.1%/+0.8%/-0.7%，启动 p50 15.88 → 15.84 ms，空闲 PSS 11.74 → 11.84 MiB，均在噪声/护栏范围内；全矩阵 0 错误。payload 指标本轮多数更快，但单 seed 不把这部分记为 semaphore 带来的收益。

下一步进入调度优化：对真实混合快慢请求比较 round-robin 与最短队列，重点验证 2 worker 下小响应 C100 p99，且必须保留公平性、取消和流式背压语义。

### 最少未完成请求调度

多 worker 调度已从固定 round-robin 改为“最少未完成请求”，并以轮转起点打破负载相同的平局。每个 job 从入队到 handler、streaming 和 scope teardown 完成期间都计入对应 isolate；调用方取消不会提前伪造空闲状态，job 实际结束后计数自动回收。负载计数只用于启发式选择，不承载内存可见性，因此使用 `Relaxed` 原子操作，并将所有 worker 计数放在一个共享连续数组中，减少分配与 ARM 内存屏障成本。

真实混合测试按顺序发送：worker 0 上一个 200 ms 慢请求、worker 1 上一个快请求、随后再发一个快请求。新调度让第三个请求在 100 ms 护栏内由 worker 1 完成；固定 round-robin 会把它排到 worker 0 的慢请求之后。另有取消测试确认排队请求的调用 future 被取消后，计数仍在 job 实际完成时归零。

同质负载最终证据为 `scheduler-shared-workers-2-linux-arm64-full.json`，相对接入调度前的 `admission-workers-2-linux-arm64-full.json`：

| 指标 | 调度前 | 最少未完成 | 变化 |
| --- | ---: | ---: | ---: |
| 启动 p50 | 15.84 ms | 16.22 ms | +2.4% |
| 空闲 PSS | 11.84 MiB | 11.91 MiB | +0.5% |
| health C1 | 5,731 req/s | 5,630 req/s | -1.8% |
| health C10 | 42,437 req/s | 42,260 req/s | -0.4% |
| health C100 | 74,287 req/s | 74,973 req/s | +0.9% |
| JSON 64 KiB C100 p99 | 24.11 ms | 16.22 ms | -32.7% |
| bytes 64 KiB C100 p99 | 14.23 ms | 8.32 ms | -41.5% |

全矩阵 0 错误；除尾延迟改善外的吞吐与 CPU efficiency 变化均在 ±5% 护栏内。第一版 acquire/release 原子计数曾令 health C100 单次回退 7.2%，已被 `Relaxed` 连续计数布局消除，因此不保留该中间实现。

## 优化原则与验收规则

1. 以同一机器、同一容器镜像、同一 TypeScript 7 工具链下的 Tysel 基线为主比较；Node、Bun、Deno 只用于判断差距结构。
2. 每次只合入一个可归因的改动，完成单元测试、协议一致性测试和完整基准后再叠加下一项。
3. 正式判定至少运行 4 个 seed，使用各 seed 中位数；变化小于 5% 默认视为噪声，除非置信区间稳定支持改善。
4. 任何优化必须保持 0 请求错误，并通过 HTTP body、headers、streaming、WebSocket、取消和超限请求测试。
5. 启动 p50 回退不得超过 5%；空闲 PSS、峰值 PSS 和 health C100 p99 回退不得超过 10%。高风险阶段需要单独说明例外预算。

主指标为 `health C1/C10 req/s`、`health C10 p99`、`JSON/bytes 64 KiB C10 req/s` 和每 server CPU-second 完成的请求数。启动时间、PSS、错误数和 C100 p99 是护栏指标。

## 阶段 0：补齐诊断能力

预计 0.5–1 天。

- 增加仅在诊断构建中启用的分段计时：HTTP 解码、dispatch 排队、JS 执行、响应提取/复制、scope teardown、socket 写出。
- 在 Linux runner 上采集 `perf`/flamegraph 和线程 CPU；正式计分运行关闭所有探针。
- 增加实验性 worker 数参数（1/2/4），暂不改变默认值。
- 增加“预生成字符串响应”和 `Response.json` 两个诊断 workload，分离 JSON 序列化与响应传输成本。
- 重跑 seed 1–4，确认约 40 ms 停顿可以稳定复现。

退出条件：能够解释主要 CPU 热点和排队时间，且后续每项优化都能归因到具体阶段。

## 阶段 1：低风险固定开销与尾延迟

预计 1–2 天，按以下顺序逐项验证。

1. 对服务端接受的 TCP socket 设置 `TCP_NODELAY`。目标：`health C10 p99 < 5 ms`，理想目标 `< 2 ms`，且吞吐不下降超过 5%。若 40 ms 峰值不消失，立即回退假设并采集 socket/系统调用证据。
2. 在 QuickJS bootstrap 时缓存 Request 工厂，移除每请求 `ctx.eval`。目标：`health C1` 或 CPU 效率提升至少 10%。
3. 空请求体不再启动 body pump 任务；非升级请求不创建 WebSocket channels。目标：`health C1/C10` 再提升至少 10%，每请求 CPU 时间下降。
4. 为一次性 body 提供准确 `size_hint`，并确认 Hyper 可以发送 `Content-Length` 而不是不必要的 chunked framing。

阶段退出条件：相对当前基线，`health C1/C10` 吞吐或 CPU 效率累计提升至少 20%，并消除或解释 40 ms 尾延迟。

## 阶段 2：缓冲响应直通路径

预计 2–4 天。

- 将响应体明确区分为 `Buffered(Bytes)` 与 `Stream`；普通字符串、JSON 和字节响应直接返回缓冲 body，真实流才进入 channel。
- 缓冲响应携带精确长度，流式响应继续保持背压与取消语义。
- 流式 channel 尽量传递 `Bytes`，减少 `Vec<u8>` 到 `Bytes` 的再次转换。
- 统计并削减 QuickJS string → Rust string → `Vec<u8>` → channel → `Bytes` 链路中的分配和复制。
- 增加 buffered/streaming、空 body、多 chunk、取消、异常和大 body 回归测试。

目标：`bytes 64 KiB C10` 提升至少 30%，`JSON 64 KiB C10` 提升至少 20%，峰值 PSS 增幅不超过 10%。

## 阶段 3：并行度与背压

预计 3–5 天。这是收益可能很高、但产品语义风险也最高的阶段。

- 基准比较 1/2/4 workers 的吞吐、排队时间、CPU、启动和 PSS，不预设“越多越好”。
- 启用 manifest 中的 `max_in_flight`，用 semaphore 建立明确背压，避免过载时无限排队。
- 比较轮询、最短队列或共享入口队列，减少慢请求造成的 head-of-line blocking。
- 明确多 isolate 的应用全局状态语义。若不同 isolate 会产生可见状态分叉，多 worker 只能先作为 stateless/显式 opt-in 模式，不能直接替换默认值。

建议目标：2 workers 下 `JSON/bytes 64 KiB C10` 相对基线提升至少 50%，health CPU 效率提升至少 30%；空闲 PSS控制在 20 MiB 左右，启动 p50 控制在 30 ms 内，尾延迟不得回退。

## 阶段 4：QuickJS 与 JSON 专项

仅在阶段 2 和阶段 3 后仍确认 JSON 序列化是主要热点时启动。

- 分离测量 `JSON.stringify`、headers 构建、Request/Response 对象构建和 scope teardown。
- 优先减少对象与 header 分配，谨慎评估复用 request scope 中的不可变工厂和对象形状。
- 只有在能精确保持 Web `JSON.stringify` 语义时才评估宿主原生序列化；否则接受并记录引擎上限，不为基准破坏兼容性。

目标：在阶段 3 结果之上让 `JSON 64 KiB C10` 再提升 15–20%，且语义测试完全一致。

## 优先级表

| 优先级 | 工作项 | 预期收益 | 风险 | 决策 |
| --- | --- | --- | --- | --- |
| P0 | 服务端 `TCP_NODELAY` 单变量实验 | 高尾延迟收益 | 低 | 立即做 |
| P0 | 缓存 Request 工厂 | 中等 CPU/吞吐收益 | 低 | 立即做 |
| P0 | 空 body/非 WS 快路径 | 中等小请求收益 | 低 | 立即做 |
| P0 | Buffered response 直通 | 高 payload 收益 | 中 | 阶段 1 后做 |
| P1 | worker 数可配置并实测 | 高吞吐收益 | 高：状态语义、内存 | 实验后决策 |
| P1 | `max_in_flight` 背压 | 高稳定性和 p99 收益 | 中 | 与并发模型一起做 |
| P2 | 宿主原生 JSON | 不确定 | 高：Web 语义 | 只有 profile 证明后评估 |

## 建议的第一个优化迭代

第一个迭代只包含 `TCP_NODELAY`、缓存 Request 工厂、空 body/非 WebSocket 快路径，并为三项分别保留独立 benchmark 结果；未经明确要求不创建 commit。完成 seed 1–4 后再决定是否进入 Buffered response 直通路径。这样可以在 1–2 天内得到确定收益，也不会提前承担多 worker 的状态语义风险。

本计划不承诺超过 Bun、Deno 或 Node。建议第一阶段对外目标表述为：保持约 20 ms 启动和低于 12 MiB 的空闲 PSS，同时显著降低中低并发请求成本与尾延迟；达到多架构、重复运行和统计稳定后，再把数据用于官网营销。
