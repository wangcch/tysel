下面这版可以直接作为 **Tysel 的项目 PRD、Architecture RFC 和阶段路线图母文档**。

# Tysel 完整规划与技术方案

**版本：** 规划稿 v0.2

**日期：** 2026-08-20

> **Tysel — A lightweight native runtime for TypeScript services and agents.**
>
> **Write TypeScript. Ship a binary.**

---

## 1. 项目摘要

Tysel 是一个面向现代后端服务、任务与 AI Agent 的轻量 TypeScript Runtime。

它希望覆盖大部分以 HTTP、数据库、消息队列、第三方 API、MCP 和 LLM 调用为主的服务端场景，同时提供：

- 单文件部署
- 毫秒级启动
- 较低的空闲内存
- Web 标准 API
- 原生能力模块
- 默认最小权限
- 不可信代码隔离
- Agent 任务挂起与恢复
- Wasm Component 扩展能力

Tysel 不把完整 Node.js 兼容性作为目标，也不要求生产环境携带 Node、V8、`node_modules` 或包管理器。

核心产品形态是：

```text
TypeScript Source
       │
       ▼
Tysel Build
       │
       ▼
Single Native Executable
       │
       ├── HTTP Service
       ├── Worker / Cron / Queue
       ├── MCP Tool
       ├── Agent Task
       └── Isolated Plugin
```

---

## 2. 核心战略判断

### 2.1 Tysel 首先是 Runtime，而不是新语言

开发者继续编写标准 TypeScript，不需要学习一门新的语法体系。

Tysel 的差异不应主要来自类型语法，而应来自：

1. 更轻的生产部署模型。
2. 更适合现代服务的能力模型。
3. 更适合 AI 代码的安全模型。
4. 更适合异步 Agent 的任务模型。
5. 原生能力与 Wasm 扩展机制。

### 2.2 不把完整 TS→Native 编译器放在 v1 关键路径

完整实现 JavaScript 动态语义，包括原型链、闭包、异常、异步、反射、动态导入、`Proxy`、GC 和任意 npm 依赖，是一个规模极大的编译器项目。

因此 Tysel 应采用分层执行策略：

```text
第一阶段
TypeScript → JavaScript Bundle → Compact JS Engine

第二阶段
TypeScript + Wasm Component 混合执行

长期阶段
受约束 TypeScript Static Profile → Wasm / Native AOT
```

这意味着 Tysel 在初期的“native”主要指：

- Runtime Core 是原生程序。
- HTTP、TLS、数据库、文件、加密等能力由原生实现。
- 最终产物是单个原生可执行文件。
- 生产环境不需要安装 JavaScript Runtime。
- Wasm 和 Native Capability 是一等公民。

初期并不宣称所有 TypeScript 业务逻辑都已直接编译成机器码。

### 2.3 TypeScript 7 是构建基础，而不是应用执行引擎

TypeScript 7 已把编译器和语言服务迁移到 Go，官方公布的完整构建性能通常比 TypeScript 6 快 8～12 倍。但 TypeScript 7.0 暂未提供稳定的程序化 API，官方预计由后续版本提供新的 API。Tysel 初期应通过 CLI 调用 TypeScript 7 做类型检查，而不是直接嵌入其内部实现。

---

## 3. 产品定位

### 3.1 一句话定位

> Tysel 是一个面向 TypeScript 服务和 Agent 的轻量原生 Runtime，让开发者编写 TypeScript，并直接交付单个可执行文件。

### 3.2 目标用户

| 用户                   | 需求                                          |
| ---------------------- | --------------------------------------------- |
| TypeScript 后端开发者  | 不想维护 Node、容器基础镜像和大量运行依赖     |
| 微服务团队             | 更小镜像、更快扩缩容、更低空闲内存            |
| AI Agent 开发者        | MCP、LLM、Tool、Workflow 的统一运行模型       |
| 企业平台团队           | 对 Agent 和生成代码实施权限、配额、审计与隔离 |
| Serverless / Edge 平台 | 快速实例化高密度执行单元                      |
| 插件平台               | 安全运行第三方或 AI 生成代码                  |

### 3.3 目标覆盖范围

Tysel 所说的“覆盖大部分服务端场景”，主要指以下类型：

```text
REST API
RPC
Webhook
WebSocket
SSE
反向代理
GraphQL Handler

Cron
Queue Consumer
Background Job
ETL
CDC Consumer
Scheduled Task

MCP Server
MCP Tool
Agent Action
LLM Function
AI Workflow Node
Plugin Function
```

更准确的目标是：

> 覆盖大部分 I/O 密集、事件驱动、请求驱动、无 Node Native Addon 依赖的现代 TypeScript 后端应用。

“80% 覆盖率”应作为待验证的产品假设，而不是直接作为宣传结论。

---

## 4. 明确非目标

Tysel v1 不承担以下目标：

| 非目标                  | 原因                                                   |
| ----------------------- | ------------------------------------------------------ |
| 完整 Node.js API 兼容   | 会重新引入 Node 的历史边界和复杂度                     |
| 任意 npm 包兼容         | 包含 Node API、Native Addon 和动态行为的包无法轻量支持 |
| `node-gyp` / N-API      | 使用 Tysel Capability 或 Wasm Component 替代           |
| Electron / 桌面应用     | 不属于服务 Runtime                                     |
| Next.js / Nuxt 完整 SSR | 依赖复杂 Node 生态和构建约定                           |
| NestJS 无修改运行       | 过度依赖 Node 反射、适配器和生态                       |
| 浏览器 Runtime          | Tysel 是服务端执行环境                                 |
| 通用 POSIX 容器         | 不提供任意进程、终端、裸 socket 和动态库加载           |
| 运行时安装依赖          | 所有依赖必须在构建期锁定并打包                         |
| 自动序列化任意 JS Heap  | Agent 挂起使用可重放任务模型，而非 Heap Snapshot       |
| 完整 workerd / Wrangler 兼容 | 会把主线带入 Cloudflare 平台兼容竞争，偏离 Service、Task 与 Agent 核心 |

---

## 5. 业务场景划分

Tysel 采用同一个 Runtime Core，提供两个主要信任模式。

### 5.1 Trusted Service Mode

运行团队自己编写并审核过的服务代码。

适合：

- API 服务
- 内部微服务
- WebSocket / SSE
- Worker
- Cron
- Queue Consumer
- MCP Server
- 长期运行的 Agent Gateway

特点：

```text
Runtime 自己管理 HTTP Listener
Runtime 自己管理 TLS / Connection Pool
Capability 可在同一进程执行
追求吞吐、低延迟和部署简单
```

### 5.2 Isolated Task Mode

运行第三方、租户、插件或 AI 生成代码。

适合：

- AI 生成的 Tool
- 多租户自动化
- 插件平台
- 用户提交的脚本
- 不可信 MCP Tool
- Agent 动态生成的执行步骤

特点：

```text
Supervisor
   │
   ├── 权限策略
   ├── Secrets
   ├── 数据库连接
   ├── 网络策略
   └── 审计
          │
          ▼
    Isolated Worker
          │
          └── 仅能调用授权 Capability
```

可信服务与不可信任务共享编程模型，但具有不同的进程边界和能力实现方式。

---

## 6. 外部参照与战略边界

Tysel 不以任何单一项目为对手或路线定义者。不同产品只用于验证不同维度：

| Tysel 维度             | 主要参照                              | 需要验证的问题                         |
| ---------------------- | ------------------------------------- | -------------------------------------- |
| TypeScript Service     | Node.js、Deno、Bun                    | 迁移成本、吞吐、启动、生态兼容         |
| 单文件部署             | Deno compile、Bun build、Go / Rust    | 产物大小、外部依赖、跨平台交付         |
| Durable Task           | Temporal、Restate、Inngest            | 恢复语义、幂等、调度、可观测性         |
| Agent Runtime          | LangGraph、Mastra、Cloudflare Agents  | LLM、MCP、Tool、Signal 与持久化体验    |
| 不可信代码执行         | workerd、Capsid、Wasm Sandbox         | 隔离、配额、取消、背压、资源回收       |
| Capability 与扩展 ABI  | WASI Component Model、Deno Permissions | 权限表达、接口演进、跨语言扩展         |

Tysel 的核心定位保持不变：

> **一个可以单文件交付、默认最小权限、支持持久化恢复的 TypeScript Service 与 Agent Runtime。**

路线优先验证三个闭环：

1. **Service**：标准 Fetch / Hono 服务可以低摩擦迁移并构建为单文件。
2. **Agent**：LLM、MCP、Queue、Signal 是统一 Task 模型的一等能力。
3. **Durable**：任务可以在进程或机器故障后安全恢复，副作用不被意外重复。

---

## 7. 核心设计原则

### 7.1 Web Standard First

Tysel 的基础 API 应优先兼容 Web 标准：

```text
Request
Response
Headers
URL
URLSearchParams
fetch
ReadableStream
WritableStream
WebSocket
Event
AbortController
TextEncoder
TextDecoder
crypto
FormData
Blob
File
```

ECMA-429 已定义面向浏览器与服务端 Runtime 的 Minimum Common Web API，适合作为 Tysel 的基础兼容目标。

### 7.2 No Ambient Authority

应用默认不能直接访问：

```text
process
全量环境变量
任意文件系统
任意网络
裸 TCP / UDP
子进程
动态库
FFI
系统用户信息
宿主机元数据
```

所有外部行为必须通过 Capability。

### 7.3 Build Once, Ship One File

生产部署不要求：

```text
Node
npm
pnpm
Tysel Runtime 安装包
node_modules
TypeScript Compiler
```

最终产物：

```bash
tysel build

./dist/orders
```

### 7.4 Capability，而不是 Node Module

平台能力由 Runtime 提供：

```ts
import { postgres } from "tysel:postgres";
import { llm } from "tysel:llm";
import { secrets } from "tysel:secrets";
```

而不是依赖任意 Node 包获得底层权限。

### 7.5 Task 是内部统一抽象

HTTP 请求、Cron、Queue、MCP、Agent 都进入同一个内部任务模型：

```text
Trigger
   ↓
Task
   ↓
Policy
   ↓
Scheduler
   ↓
Capability
   ↓
Result
```

### 7.6 不承诺魔法式自动挂起

普通：

```ts
await fetch(...)
```

只代表异步等待，不代表持久化。

只有显式 Durable API 才形成可恢复边界：

```ts
await ctx.step(...)
await ctx.effect(...)
await ctx.sleep(...)
await ctx.waitForSignal(...)
```

### 7.7 Protocol First，Public API Second

跨进程、跨 Runtime 与持久化边界必须先定义版本化协议和资源状态机，再冻结公共 TypeScript API：

```text
FetchRPC        Host / Supervisor ↔ Worker 的请求、响应、Upgrade 与流
CapabilityRPC   User Runtime ↔ Trusted Broker 的能力调用、代理与流
TaskRPC         Trigger / Scheduler ↔ Worker 的 claim、lease、cancel 与 result
DurableLog      Replay event、effect、timer、signal 与 generation fencing
```

这些协议可以共享 `resource_id`、credit、deadline、cancellation、half-close、lease 和 owner token 等概念，但不得因为复用代码而混淆信任边界。

每类远程资源必须明确：创建者、所有者、作用域、配额、取消方、超时行为、显式释放、崩溃回收和迟到结果处理。GC 只能作为资源释放的兜底机制。

### 7.8 Compatibility Is a Versioned Contract

每个 Tysel minor release 应固定并发布：

```text
QuickJS-ng revision
Web compatibility profile
WPT / Test262 selected revision
Capability ABI version
TAP package version
supported npm / framework fixtures
known deviations
```

未知 profile、ABI 或 compatibility flag 必须给出可执行的错误，不得静默忽略。差分测试用于发现差异，独立绝对断言用于证明行为正确。

---

## 8. 开发者编程模型

### 8.1 最小 Web 兼容入口

最小应用只需要导出 Fetch Handler：

```ts
export default {
  async fetch(request: Request): Promise<Response> {
    return Response.json({
      message: "Hello from Tysel",
      path: new URL(request.url).pathname,
    });
  },
};
```

运行：

```bash
tysel dev src/index.ts
```

构建：

```bash
tysel build src/index.ts
./dist/app
```

### 8.2 Tysel 扩展应用模型

```ts
import { defineApp, cron, queue, mcp } from "tysel";

import { postgres } from "tysel:postgres";

const db = postgres("main");

export default defineApp({
  async fetch(request, ctx) {
    const users = await db.query("select id, name from users limit 20");

    return Response.json({ users });
  },

  tasks: {
    cleanup: cron("0 3 * * *", async (ctx) => {
      await db.execute("delete from sessions where expires_at < now()");
    }),

    consumeOrder: queue("orders", async (message, ctx) => {
      await db.execute("insert into order_events(payload) values ($1)", [
        message,
      ]);
    }),

    analyzeCustomer: mcp(
      {
        description: "Analyze a customer",
        input: {
          customerId: "string",
        },
      },
      async (input, ctx) => {
        return ctx.agents.analyzeCustomer(input.customerId);
      },
    ),
  },
});
```

### 8.3 Durable Agent Task

```ts
import { durableTask } from "tysel";
import { llm } from "tysel:llm";
import { postgres } from "tysel:postgres";

const db = postgres("main");

export default durableTask(async (ctx, input) => {
  const customer = await ctx.step("load-customer", () =>
    db.queryOne("select * from customers where id = $1", [input.customerId]),
  );

  const analysis = await ctx.effect("analyze-customer", () =>
    llm.generate({
      model: "default",
      input: customer,
    }),
  );

  await ctx.step("save-result", () =>
    db.execute("insert into analyses(customer_id, result) values ($1, $2)", [
      input.customerId,
      analysis,
    ]),
  );

  await ctx.sleep("24h");

  return {
    status: "completed",
  };
});
```

---

## 9. 总体系统架构

```text
┌──────────────────────────────────────────────┐
│                 Build Plane                  │
│                                              │
│ TypeScript Project                           │
│      │                                       │
│      ├─ TypeScript 7 Type Check              │
│      ├─ TS Transpile / ESM Bundle            │
│      ├─ Dependency Resolution                │
│      ├─ Capability Analysis                  │
│      ├─ Policy Validation                    │
│      └─ Asset / Wasm Packaging               │
│                     │                        │
│                     ▼                        │
│             Tysel App Package                │
│                     │                        │
│                     ▼                        │
│          Native Executable Assembly          │
└─────────────────────┬────────────────────────┘
                      │
                      ▼
┌──────────────────────────────────────────────┐
│               Runtime Data Plane             │
│                                              │
│ ┌──────────────────────────────────────────┐ │
│ │ Supervisor                               │ │
│ │                                          │ │
│ │ HTTP / TLS / Routing                     │ │
│ │ Scheduler                                │ │
│ │ Policy Engine                            │ │
│ │ Capability Registry                      │ │
│ │ Secrets                                  │ │
│ │ Observability                            │ │
│ │ Durable State Store                      │ │
│ └───────────────┬──────────────────────────┘ │
│                 │                            │
│     ┌───────────┼──────────────┐             │
│     ▼           ▼              ▼             │
│ QuickJS      Wasm Component  Native Task     │
│ Isolate      Instance        Capability      │
│     │           │              │             │
│     └───────────┴──────────────┘             │
│                 │                            │
│                 ▼                            │
│ Postgres / Redis / HTTP / LLM / FS / Queue  │
└──────────────────────────────────────────────┘
```

---

## 10. 技术选型

### 10.1 Runtime Core：Rust

建议 Tysel Runtime Core 使用 Rust。

主要原因：

- 内存安全更适合 Runtime 和 Sandbox。
- 可直接集成 Wasmtime。
- 适合编写网络、调度、IPC 和 Capability Broker。
- 跨 Linux、macOS 和 Windows。
- 可通过 LTO、裁剪 feature、`panic=abort` 控制体积。
- 具有成熟的异步与系统编程生态。

建议技术栈：

```text
Language        Rust
Async Reactor   Tokio 初期，后续按性能评估
HTTP            Hyper / 自有薄封装
TLS             rustls，可选链接
JS Engine       QuickJS-ng
Wasm            Wasmtime
Serialization   自定义 bounded codec / MessagePack / CBOR
Config          TOML
Tracing         tracing + OTLP exporter
Local Store     SQLite
Distributed     Postgres
```

### 10.2 JavaScript Compatibility Engine：QuickJS-ng

QuickJS 的主要价值是：

- 体积小。
- 易嵌入。
- 启动开销低。
- 支持现代 ECMAScript。
- 不包含大型 JIT。

QuickJS 官方将其定位为 small and embeddable JavaScript engine，并公布简单程序可产生较小的代码体积和很低的 Runtime 实例生命周期。

Tysel 不应把 QuickJS 写死为永久架构，而应定义：

```rust
trait ExecutionEngine {
    fn create_isolate(&self, config: IsolateConfig) -> Result<Isolate>;
    fn load_module(&self, bundle: &[u8]) -> Result<Module>;
    fn invoke(&self, handler: HandlerId, input: Value) -> Future<Result<Value>>;
    fn interrupt(&self, reason: InterruptReason);
}
```

v0.x 只实现 QuickJS Engine。

未来可以增加：

```text
Static AOT Engine
Wasm Component Engine
High-throughput Engine
```

### 10.3 Wasm Runtime：Wasmtime

Wasmtime 支持 JIT 和 AOT 编译的 WebAssembly Module 与 Component，也提供 Component Model 嵌入接口。

Tysel 使用 Wasmtime 的主要目的不是把所有应用立即编译为 Wasm，而是：

1. 运行高性能计算模块。
2. 运行跨语言 Capability。
3. 运行第三方扩展。
4. 为 Static TypeScript Compiler 提供首个后端。
5. 使用 WIT 定义稳定 ABI。

### 10.4 Capability ABI：WIT

WebAssembly Interface Type 是 Component Model 的接口描述语言，可以定义类型、导入、导出和外部资源句柄。

每个 Tysel Capability 应包含：

```text
capability/
├── interface.wit
├── manifest.toml
├── typescript/
│   └── index.d.ts
├── host/
│   └── Rust implementation
├── policy.schema.json
└── tests/
```

例如：

```wit
package tysel:postgres@1.0.0;

interface database {
  resource connection {
    query: func(
      sql: string,
      params: list<value>
    ) -> result<list<row>, db-error>;
  }
}
```

---

## 11. 构建流程

### 11.1 构建流水线

```text
1. 读取 tysel.toml
2. 解析 package.json 和 lockfile
3. TypeScript 7 执行 noEmit 类型检查
4. 转译 TypeScript
5. 生成自包含 ESM Bundle
6. 分析 tysel:* Capability Import
7. 校验权限声明
8. 打包静态资源和 Wasm Component
9. 生成 Tysel App Package
10. 与目标平台 Runtime Stub 合并
11. 输出单个 executable
12. 生成 SBOM、Manifest 和校验摘要
```

### 11.2 TypeScript 构建器

初期建议：

```text
Type Check    TypeScript 7 CLI
Transpile     esbuild 或 Oxc Adapter
Bundle        esbuild
Minify        可选
Source Map    Debug 模式保留
```

由于 TypeScript 7.0 尚无稳定程序化 API，初期应将 TypeScript Checker 做成外部 Tool Adapter。未来 API 稳定后再评估嵌入。

### 11.3 Tysel App Package

定义稳定的内部包格式，简称 TAP：

```text
TAP Header

Manifest
├── formatVersion
├── runtimeVersion
├── applicationId
├── entrypoint
├── executionProfile
├── capabilityRequirements
├── resourceLimits
├── bundleHash
└── buildMetadata

Payload
├── application.esm
├── assets/
├── components/
├── source-map/
└── signatures/
```

推荐使用：

```text
Manifest       CBOR
Compression    zstd
Hash           SHA-256
Signature      Ed25519，可选
```

TAP 是稳定部署格式。

QuickJS Bytecode 不应成为公共部署 ABI，因为它通常依赖具体引擎版本和构建身份。

### 11.4 单文件可执行程序

v0.1 采用预构建 Runtime Stub：

```text
tysel-service-linux-x64
          +
TAP Application Payload
          ↓
dist/orders
```

应用 Payload 可以放在：

- 可执行文件自定义 Section。
- 文件尾部 Trailer。
- 平台 Resource Section。

启动时 Runtime Memory Map Payload，无需解压到文件系统。

初期提供两个主要 Stub：

```text
tysel-service
tysel-isolate
```

后续再实现 Capability Linker，只链接实际使用的能力。

---

## 12. Runtime 调度模型

### 12.1 核心对象

```text
Runtime
├── Supervisor
├── Reactor
├── Scheduler
├── Execution Pool
├── Capability Registry
├── Policy Engine
├── State Store
└── Observability
```

### 12.2 Isolate 与线程

QuickJS Runtime 通常应固定在所属 Worker Thread 上。

建议：

```text
Global Native I/O Reactor
          │
          ├── Worker Thread 1
          │      └── QuickJS Isolate Pool
          │
          ├── Worker Thread 2
          │      └── QuickJS Isolate Pool
          │
          └── Worker Thread N
                 └── QuickJS Isolate Pool
```

原生 I/O 执行完成后，将结果投递回 Isolate 的 Completion Queue。

任何 QuickJS 对象、函数引用或 Native Handle 都不能跨线程或跨进程传递。

跨边界数据只允许：

```text
null
boolean
number
string
bytes
array
record
error
stream token
opaque resource handle
```

### 12.3 Task 生命周期

```text
Created
   ↓
Queued
   ↓
Running
   ├── Waiting I/O
   ├── Suspended
   ├── Retrying
   ├── Completed
   ├── Failed
   └── Canceled
```

每个 Task 都拥有：

```text
Task ID
Application ID
Tenant ID
Deadline
Cancellation Token
CPU Budget
Memory Budget
Capability Scope
Trace Context
Idempotency Key
```

### 12.4 多核扩展

Tysel 不在一个 JS Isolate 内并行执行 JavaScript。

多核扩展通过：

- 多 Isolate。
- 多 Worker Thread。
- 多 Worker Process。
- Wasm / Native Capability 并行。
- 请求分片。
- Queue Partition。

---

## 13. Capability 系统

### 13.1 四层权限模型

Tysel 权限通过四层求交集：

```text
1. Runtime Build 中实际存在的 Capability
2. 应用 Manifest 请求的 Capability
3. 部署策略允许的 Capability
4. OS Sandbox 最终允许的资源

Effective Permission =
Build ∩ App Request ∩ Deployment Policy ∩ OS Boundary
```

应用永远不能在运行时扩大权限。

### 13.2 Capability Import

```ts
import { fetchClient } from "tysel:http";
import { postgres } from "tysel:postgres";
import { llm } from "tysel:llm";
import { secrets } from "tysel:secrets";
```

### 13.3 Capability 分层

#### Ring 0：Runtime Core

```text
Task Scheduler
Timers
Encoding
URL
Fetch primitives
Streams
Crypto primitives
Cancellation
Logging
Policy
```

#### Ring 1：官方 Capability

```text
HTTP Client
HTTP Server
WebSocket
Filesystem
SQLite
Postgres
Redis
Object Storage
Queue
Cron
JWT
OAuth
Secrets
MCP
LLM
OpenTelemetry
```

#### Ring 2：生态 Capability

```text
Wasm Component
第三方数据库驱动
SaaS Connector
企业内部服务
自定义 Agent Tool
```

### 13.4 Trusted Mode

可信服务中，Capability 可以直接在同进程运行：

```text
QuickJS Isolate
      │
      ▼
Native Capability
      │
      ▼
Postgres
```

优点：

- 调用开销低。
- 共享连接池。
- 适合高吞吐服务。

### 13.5 Isolated Mode

不可信模式中，Capability 由 Supervisor Broker 持有：

```text
Untrusted Worker
      │
      │ bounded IPC
      ▼
Capability Broker
      │
      ├── Credential
      ├── Connection Pool
      ├── Network
      └── Audit
```

Worker 不持有：

```text
数据库密码
原始 Secret
任意 socket
文件描述符
Native Pointer
宿主环境变量
```

### 13.6 Secret 设计

优先使用不透明 Secret Handle：

```ts
const token = secrets.ref("OPENAI_API_KEY");

const response = await fetchClient.request({
  url: "https://api.example.com",
  auth: {
    type: "bearer-secret",
    secret: token,
  },
});
```

在 Isolated Mode 中，Secret 原文不进入 JavaScript Heap。

只有可信模式下，才可通过额外权限读取 Secret 原始值。

---

## 14. 配置设计

```toml
[app]
name = "orders"
entry = "src/index.ts"
profile = "service"

[server]
listen = "0.0.0.0:3000"
http1 = true
http2 = true
websocket = true

[permissions]
fetch = [
  "api.openai.com",
  "crm.internal.example"
]
secrets = [
  "OPENAI_API_KEY"
]
postgres = [
  "main:read-write"
]
fs_read = [
  "./public"
]

[limits]
memory_mb = 128
cpu_ms_per_turn = 50
request_timeout_ms = 30000
max_in_flight = 1000
max_response_mb = 16

[durable]
store = "sqlite"
path = "./data/tysel.db"

[observability]
logs = "json"
traces = "otlp"
metrics = "prometheus"
```

构建检查：

```bash
tysel inspect
```

输出：

```text
Application: orders
Profile: service

Capabilities
  HTTP Server
  HTTP Client
    api.openai.com
    crm.internal.example

  Postgres
    main: read-write

  Secrets
    OPENAI_API_KEY

Filesystem
  Read
    ./public

Denied
  Raw TCP
  Child Process
  FFI
  Dynamic Library
  Environment
```

---

## 15. 安全架构

### 15.1 威胁模型

Tysel 应区分三类代码：

| 类型               | 信任程度           |
| ------------------ | ------------------ |
| 团队自己的服务代码 | Trusted            |
| 第三方插件         | Untrusted          |
| AI 生成代码        | Hostile by default |

### 15.2 不可信执行边界

不可信代码必须使用进程隔离。

单纯创建另一个 JavaScript Context 不能作为完整安全边界。Node.js 官方也明确指出 `node:vm` 不是运行不可信代码的安全机制。

Linux Production Sandbox 建议包括：

```text
Separate Process
User Namespace
Mount Namespace
Network Namespace
seccomp
Landlock
cgroup
rlimit
no_new_privs
Read-only Root
Ephemeral Tmp
PID Limit
CPU Quota
Memory Limit
FD Limit
```

macOS 和 Windows 初期只提供开发或可信运行模式。

正式的不可信代码安全承诺仅针对经过验证的 Linux Sandbox。

### 15.3 网络安全

Outbound Fetch 必须验证：

1. 请求域名。
2. DNS 解析结果。
3. 目标 IP 范围。
4. 重定向后的域名和 IP。
5. 代理目标。
6. 响应体大小。
7. 超时与连接数。

默认拒绝：

```text
localhost
link-local
metadata service
内网网段
Unix Socket
裸 IP
DNS Rebinding
未授权重定向
```

### 15.4 代码限制

Isolated Mode 默认禁用：

```text
eval
new Function
任意动态 import
远程 import
file: import
FFI
WASI
process
child_process
raw socket
native addon
```

### 15.5 供应链

Release Build 应支持：

```text
锁定 lockfile
依赖完整性验证
TAP Hash
SBOM
Capability Manifest
可执行文件签名
可重复构建记录
依赖 License 报告
```

---

## 16. npm 与生态兼容策略

Tysel 不创建新的包管理器。

开发阶段继续使用：

```text
npm
pnpm
yarn
JSR
```

Tysel 只负责读取依赖图、锁定版本并将依赖打包进应用。

### 16.1 兼容分级

| Tier | 类型                             | 支持       |
| ---- | -------------------------------- | ---------- |
| A    | 纯 ESM + Web API                 | 直接支持   |
| B    | 纯 JS，依赖少量通用 Node Shim    | 构建期兼容 |
| C    | Node 文件、网络、Buffer 深度依赖 | 部分迁移   |
| D    | N-API、`node-gyp`、FFI           | 不支持     |
| E    | Electron、原生桌面依赖           | 不支持     |

### 16.2 可提供的轻量 Shim

```text
buffer
path
util
events
assert
querystring
```

这些 Shim 只用于代码兼容，不赋予额外系统权限。

不提供：

```text
child_process
cluster
vm
worker_threads 的 Node 语义
native addon
任意 net socket
```

### 16.3 Framework 目标

首批验证：

```text
原生 Fetch Handler
Hono
itty-router
H3 Fetch Adapter
轻量 GraphQL Handler
```

不为 Framework 写特殊 Runtime 分支。

Framework 必须最终编译为：

```ts
export default {
  fetch(request) {
    // ...
  },
};
```

---

## 17. Durable Task 与 Agent Suspend/Resume

### 17.1 不采用 Heap Snapshot 作为主方案

自动保存任意 JavaScript Heap 会带来：

- 引擎版本绑定。
- Native Handle 无法序列化。
- 网络连接无法恢复。
- Closure 与原型对象复杂。
- 升级兼容困难。
- Snapshot 体积不可控。
- 安全边界复杂。

Tysel 应采用：

> Event Log + Deterministic Replay + Explicit Durable Effects

### 17.2 Durable 执行原理

```text
Task Input
   │
   ▼
Run Function
   │
   ├── ctx.step("A")
   │       └── 保存结果
   │
   ├── ctx.effect("B")
   │       └── 保存外部副作用结果
   │
   ├── ctx.sleep("24h")
   │       └── 保存 Wake-up 时间
   │
   ▼
Suspend
```

恢复时：

```text
重新执行函数
   │
   ├── Step A 命中历史结果
   ├── Effect B 命中历史结果
   └── 从下一未完成边界继续
```

### 17.3 Durable API

```ts
ctx.step(name, fn);
ctx.effect(name, fn);
ctx.sleep(duration);
ctx.waitForSignal(name);
ctx.random();
ctx.now();
ctx.retry(policy, fn);
ctx.spawn(task, input);
ctx.cancel(taskId);
```

`ctx.random()` 和 `ctx.now()` 需要被记录，保证 Replay 一致。

### 17.4 一致性语义

Tysel 不宣称通用 exactly-once。

默认语义：

```text
Task Execution        at-least-once
Effect Deduplication  idempotency key
Queue Delivery        at-least-once
Step Result           persisted once per task history
```

对数据库场景可以提供：

- Transactional Outbox。
- Postgres Advisory Lock。
- Idempotency Table。
- Effect Token。

### 17.5 Durable Store

接口：

```rust
trait DurableStore {
    async fn load_history(&self, task_id: TaskId) -> Result<History>;
    async fn append_event(&self, event: TaskEvent) -> Result<()>;
    async fn schedule_wakeup(&self, wakeup: Wakeup) -> Result<()>;
    async fn claim_task(&self, task_id: TaskId, lease: Lease) -> Result<bool>;
}
```

实现：

```text
Local       SQLite
Production  Postgres
Large Data  Object Storage
Cache       Redis，可选
```

---

## 18. 三层执行架构

### 18.1 Compatibility Engine

```text
TypeScript
   ↓
JavaScript Bundle
   ↓
QuickJS
```

适用：

- 常规服务逻辑。
- npm 纯 JS 包。
- 动态 TypeScript 应用。
- 快速迁移。

特点：

```text
兼容度最高
启动快
体积小
CPU 峰值性能有限
```

### 18.2 Wasm Component Engine

```text
Rust / Go / C / Zig / Static TS
              ↓
        Wasm Component
              ↓
           Wasmtime
```

适用：

- 图片处理。
- 压缩。
- 数据转换。
- 加密。
- 规则计算。
- CPU 密集逻辑。
- 第三方 Capability。

### 18.3 Static TypeScript Engine

长期实验路线：

```text
Restricted TypeScript
        ↓
Typed AST
        ↓
Tysel HIR
        ↓
Typed SSA IR
        ↓
Wasm / Cranelift
        ↓
AOT Artifact
```

---

## 19. Static TypeScript Profile

Static Profile 不等同于完整 JavaScript。

建议约束：

```text
禁止 eval / new Function
禁止修改内建原型
禁止任意动态 import
限制 Proxy
限制 Reflect 动态访问
禁止 WeakRef / FinalizationRegistry
限制任意属性注入
禁止未约束 any
禁止 Node Native Addon
依赖必须通过静态分析
```

可以支持：

```text
类型别名
接口
泛型
枚举
判别联合
类
闭包
async / await
异常
数组
Map / Set
结构化对象
可空类型
模式匹配式控制流
```

### 19.1 编译器内部阶段

```text
Parser
  ↓
Typed AST
  ↓
Semantic Graph
  ↓
HIR
  ↓
Control Flow Graph
  ↓
TIR / SSA
  ↓
Escape Analysis
  ↓
Shape Analysis
  ↓
Async State Machine
  ↓
Backend
  ├── Wasm Component
  └── Cranelift Native
```

### 19.2 混合执行

一个应用允许同时包含：

```text
app.ts                 QuickJS
pricing.static.ts      Static AOT
image.wasm             Wasm Component
postgres               Native Capability
```

开发者不需要一次迁移整个项目。

```ts
import { calculatePrice } from "./pricing.static";
import { image } from "tysel:image";

export default {
  async fetch(request) {
    const price = calculatePrice(await request.json());
    const preview = await image.resize(/* ... */);

    return Response.json({ price, preview });
  },
};
```

---

## 20. Service Runtime 能力

### 20.1 HTTP

v0.1：

```text
HTTP/1.1
Keep Alive
Streaming Request
Streaming Response
SSE
WebSocket
Static Asset
Reverse Proxy Header
Graceful Shutdown
```

后续：

```text
HTTP/2
HTTP/3
Unix Socket Listener
mTLS
Advanced Load Shedding
```

### 20.2 数据访问

首批：

```text
SQLite
Postgres
HTTP API
KV
```

后续：

```text
Redis
MySQL
Kafka
NATS
S3
ClickHouse
```

### 20.3 原生快路径

以下能力优先在 Rust 中实现：

```text
HTTP Parsing
TLS
DNS
Connection Pool
Compression
Crypto
JWT
JSON Serialization Fast Path
Database Protocol
Static File
Multipart
Tracing
Metrics
```

这样即使业务控制逻辑运行在 QuickJS，主要 I/O 与重计算也可以由原生层完成。

---

## 21. CLI 与开发体验

```bash
tysel init
tysel dev
tysel check
tysel run
tysel test
tysel build
tysel inspect
tysel compat
tysel bench
tysel image
```

### 21.1 `tysel dev`

提供：

```text
文件监听
快速重新打包
Isolate 热替换
Source Map
结构化错误
Capability Mock
本地 SQLite
本地 Secret
请求日志
```

### 21.2 `tysel check`

执行：

```text
TypeScript Type Check
Capability Validation
Permission Validation
Unsupported API Detection
Node Compatibility Scan
Static Profile Check
Manifest Check
```

### 21.3 `tysel compat`

示例：

```text
Compatibility Report

Compatible
  zod
  date-fns
  hono

Requires Shim
  buffer
  events

Unsupported
  sharp
    reason: Node native addon

  express
    reason: node:http server ownership

  child_process
    reason: forbidden ambient process authority
```

### 21.4 `tysel build`

```bash
tysel build \
  --target linux-arm64 \
  --profile service \
  --release
```

输出：

```text
Type check       passed
Bundle           184 KB
Capabilities     http, postgres, secrets
Runtime          service
Executable       13.8 MB
Target           linux-arm64
Output           dist/orders
```

---

## 22. 可观测性

Tysel 应原生提供：

```text
Structured Log
Metric
Distributed Trace
Audit Event
Task History
Capability Span
Cold-start Metric
Queue Delay
Isolate Memory
Task CPU Budget
```

每次 Capability 调用自动生成 Span：

```text
task.id
application.id
tenant.id
capability
operation
resource
duration
result
policy.decision
retry.count
```

支持：

```text
OTLP
Prometheus
JSON Log
OpenTelemetry Trace Context
```

避免在 JavaScript 应用中携带完整 OpenTelemetry SDK。

---

## 23. 性能目标

以下均为产品目标，不是当前已有性能声明。外部 Runtime 公布的数据只用于量级参考；正式对比必须锁定版本、构建配置、测试负载、硬件与操作系统。

### 23.1 v0.1 Release Gate

| 指标                          | Release Gate | 长期目标 |
| ----------------------------- | -----------: | -------: |
| 基础可执行文件                |       ≤ 20MB |   ≤ 12MB |
| 小型 Service 冷启动 p50       |       ≤ 15ms |    ≤ 8ms |
| 单 Service 空闲 PSS           |       ≤ 32MB |   ≤ 20MB |
| Warm Isolate 创建             |        ≤ 5ms |    ≤ 2ms |
| HTTP Handler Runtime 额外开销 |    ≤ 1ms p50 |  ≤ 250μs |
| Durable Task 恢复             |       ≤ 10ms |    ≤ 3ms |
| Suspended Task 元数据         |       ≤ 32KB |    ≤ 8KB |
| Capability IPC p50            |      ≤ 250μs |  ≤ 100μs |
| Release Binary 外部依赖       |            0 |        0 |

### 23.2 Benchmark 矩阵

必须覆盖：

```text
Cold Start
Warm Start
Idle Memory
Peak Memory
Binary Size

JSON 1KB / 64KB
Bytes
Streaming
WebSocket
SSE

Postgres
SQLite
Fetch
Crypto
Compression

CPU Loop
JSON Parse
JSON Serialize
Regex
Map / Set

100 / 1K / 10K Concurrent Tasks
Agent LLM Wait
Suspend / Resume
Isolate Crash
Cancellation
Timeout
```

对照组：

```text
Node
Deno
Bun
Capsid
Tysel
```

### 23.3 证据规则

任何“快几倍”的宣传必须具备：

```text
同一硬件
同一 OS
同一负载
同一网络
原始样本
p50 / p95 / p99
CPU
RSS / PSS
二进制体积
Profile
完整命令
Commit SHA
```

Hello World 只能说明启动和基础开销，不能代表真实服务吞吐。

---

## 24. 仓库结构

```text
tysel/
├── Cargo.toml
├── pnpm-workspace.yaml
├── crates/
│   ├── tysel-cli/
│   ├── tysel-build/
│   ├── tysel-package/
│   ├── tysel-manifest/
│   ├── tysel-runtime/
│   ├── tysel-engine/
│   ├── tysel-engine-qjs/
│   ├── tysel-engine-wasm/
│   ├── tysel-scheduler/
│   ├── tysel-task/
│   ├── tysel-durable/
│   ├── tysel-policy/
│   ├── tysel-capability/
│   ├── tysel-cap-http/
│   ├── tysel-cap-fs/
│   ├── tysel-cap-sqlite/
│   ├── tysel-cap-postgres/
│   ├── tysel-cap-llm/
│   ├── tysel-cap-mcp/
│   ├── tysel-isolate/
│   ├── tysel-ipc/
│   ├── tysel-observability/
│   └── tysel-testkit/
│
├── packages/
│   ├── tysel/
│   ├── tysel-types/
│   ├── tysel-test/
│   └── tysel-compat/
│
├── wit/
│   ├── core/
│   ├── http/
│   ├── database/
│   ├── secrets/
│   ├── llm/
│   └── mcp/
│
├── runtime-js/
│   ├── bootstrap/
│   ├── web-api/
│   ├── capability-client/
│   └── durable/
│
├── examples/
│   ├── hello-service/
│   ├── hono-api/
│   ├── sqlite-worker/
│   ├── postgres-service/
│   ├── mcp-tool/
│   ├── durable-agent/
│   └── isolated-plugin/
│
├── benchmarks/
│   ├── startup/
│   ├── http/
│   ├── memory/
│   ├── task/
│   ├── isolate/
│   └── durable/
│
└── docs/
    ├── architecture/
    ├── security/
    ├── capabilities/
    ├── compatibility/
    ├── performance/
    └── adr/
```

---

## 25. 核心 ADR

### ADR-001：Runtime Core 使用 Rust

状态：Accepted

### ADR-002：v0.x 使用 QuickJS-ng 作为兼容执行引擎

状态：Accepted

### ADR-003：Web API 优先，不以 Node API 为基准

状态：Accepted

### ADR-004：生产环境不运行时安装依赖

状态：Accepted

### ADR-005：Capability 默认拒绝，权限取四层交集

状态：Accepted

### ADR-006：不可信代码必须采用进程隔离

状态：Accepted

### ADR-007：Durable Task 使用 Replay，而不是 JS Heap Snapshot

状态：Accepted

### ADR-008：WIT 作为 Capability 的长期 ABI 描述

状态：Accepted

### ADR-009：完整 TypeScript AOT Compiler 不进入 v1 关键路径

状态：Accepted

### ADR-010：Static TypeScript Compiler 作为并行研发路线

状态：Proposed

### ADR-011：跨边界协议先于公共 API 稳定

状态：Proposed

Host / Worker、User Runtime / Capability Broker、Scheduler / Task Worker 与 Durable Store 分别使用版本化协议。协议可以共享资源状态机概念，但不得混淆 wire format、ownership 或 trust boundary。

### ADR-012：兼容性是可版本化、可验证的契约

状态：Proposed

每个 minor release 固定引擎、Web profile、标准测试集、Capability ABI 和已知偏差。兼容性差分结果必须配有独立绝对断言和可机器读取的 Release Evidence。

---

## 26. 里程碑规划

### M0：技术可行性验证

必须完成四个 Spike：

#### Spike A：QuickJS + Native Async

验证：

```text
QuickJS Promise
Rust Async Operation
Completion Queue
Cancellation
Timeout
Memory Limit
CPU Interrupt
```

#### Spike B：Native HTTP Service

验证：

```text
Native Listener
Request → QuickJS Fetch Handler
Streaming Response
Keep Alive
Multi-isolate
```

#### Spike C：单文件打包

验证：

```text
Runtime Stub
Embedded ESM Bundle
Embedded Manifest
直接执行
Source Map
```

#### Spike D：隔离 Worker

验证：

```text
Supervisor
Worker Process
Bounded IPC
Capability Broker
Crash Recovery
Linux Resource Limit
```

只有四项都通过性能和复杂度门槛，项目才进入完整 Runtime 开发。

### M1：Service Runtime v0.1

范围：

```text
tysel dev
tysel check
tysel build
Fetch Handler
HTTP/1.1
Streaming
WebSocket
Timers
URL
Encoding
Crypto
Outbound Fetch
SQLite
Secrets
Structured Log
Single Executable
Web API Surface Manifest
Known Deviations
```

平台：

```text
macOS arm64       开发
Linux x86-64      生产
Linux arm64       生产
```

### M2：Capability 与安全模型 v0.2

范围：

```text
Capability Manifest
Policy Engine
Postgres
Filesystem
Opaque Secrets
Capability Audit
Isolated Worker
seccomp
Landlock
cgroup / rlimit
Crash Replacement
CapabilityRPC v1
Resource ID / Ownership / Lease
Credit Backpressure
Late-result Disposal
Protocol Negative Tests
```

### M3：Task 与 Agent v0.3

范围：

```text
Cron
Queue
MCP Tool
LLM Capability
Task Scheduler
Retry
Timeout
Signal
SQLite Durable Store
Replay
Suspend / Resume
TaskRPC v1
Claim / Lease / Generation Fencing
DurableLog Version Contract
Worker Crash / Timeout / Late Commit Tests
```

### M4：Wasm Component v0.4

范围：

```text
Wasmtime
WIT ABI
Component SDK
Rust Component
Go Component
Capability Registry
AOT Precompile
```

### M5：Production v1

范围：

```text
稳定 TAP 格式
稳定 Capability ABI
跨版本兼容策略
完整 Benchmark
Security Audit
Fuzzing
SBOM
签名
Release Evidence Index
Machine-readable Compatibility Report
Reproducible Build Evidence
Postgres Durable Store
OTLP
多架构 Release
生产运维文档
```

### Parallel：Static TypeScript Compiler

与 Runtime 主线并行，不阻塞 v1：

```text
Static Profile
Typed IR
Wasm Backend
Async State Machine
GC / String Runtime
Hybrid Module
Native Backend
```

---

## 27. v0.1 严格范围

v0.1 只做：

```text
TypeScript 类型检查
ESM Bundle
QuickJS-ng
Native HTTP
Fetch API
SQLite
Secrets
结构化日志
单文件构建
基础权限
macOS 开发
Linux 生产
```

v0.1 明确不做：

```text
Package Manager
完整 Node Shim
NestJS
SSR
Cloud Hosting
分布式调度
完整 Debugger
自动 Heap Suspend
Static TS Compiler
第三方 Capability Market
Windows Production Sandbox
完整 workerd / Wrangler 兼容
Cloudflare Cache / Assets / Service Binding parity
```

---

## 28. 第一组演示应用

### Demo 1：API Service

```text
Hono Fetch Handler
Postgres / SQLite
JWT
JSON API
单文件运行
```

重点展示：

```text
构建体积
冷启动
空闲内存
吞吐
无需 Node
```

### Demo 2：Background Worker

```text
Queue Input
数据处理
HTTP 调用
数据库写入
失败重试
```

重点展示：

```text
统一 Task 模型
并发
取消
超时
原生 Capability
```

### Demo 3：Isolated MCP Tool

```text
AI 生成 TypeScript Tool
仅允许一个 API 域名
仅允许一个数据库只读 Capability
无原始 Secret
进程级隔离
```

重点展示：

```text
Deny by default
Capability Broker
审计
Crash Recovery
```

### Demo 4：Durable Agent

```text
读取客户
调用 LLM
等待人工信号
24 小时后恢复
保存结果
```

重点展示：

```text
Replay
Suspend / Resume
Effect Deduplication
低驻留内存
```

---

## 29. 主要风险

| 风险                     | 表现                           | 应对                                    |
| ------------------------ | ------------------------------ | --------------------------------------- |
| QuickJS CPU 性能不足     | CPU 密集服务落后于 V8/JSC      | 原生快路径、Wasm、Static Profile        |
| Node 生态迁移成本高      | 大量包无法运行                 | Web API 优先、兼容报告、Capability 替代 |
| Rust 二进制膨胀          | TLS、数据库、Wasmtime 增大产物 | Feature Link、LTO、按需 Capability      |
| 调试体验弱               | QuickJS 缺少成熟 DevTools      | Source Map、Tracing、Runtime Inspector  |
| TS7 API 不稳定           | 无法直接嵌入 Checker           | CLI Adapter，等待稳定 API               |
| Sandbox 宣传过度         | macOS/Windows 隔离不等价       | 安全承诺限制在 Linux                    |
| Durable 语义复杂         | 非确定代码 Replay 失败         | 显式 Durable API、Determinism Check     |
| Static Compiler 范围失控 | 陷入完整 JavaScript 实现       | Static Profile 明确限制，不阻塞 Runtime |
| Capability ABI 过早固化  | 后续难演进                     | v0.x 标记实验，v1 才稳定                |
| “覆盖 80%”缺少证据       | 定位变成口号                   | 建立真实应用兼容语料库                  |

---

## 30. Go / No-Go 标准

完成 M0 后，只有满足以下条件才继续推进：

```text
小型 Service 冷启动不高于 15ms
基础空闲 PSS 不高于 32MB
单文件产物不高于 20MB
Native Async 与 QuickJS Promise 稳定
HTTP Streaming 无全量 Buffer
Capability 调用支持取消和 Backpressure
Isolated Worker 可以可靠回收
Hono 基础应用可以运行
Source Map 能定位 TypeScript 源码
```

如果 QuickJS Handler 性能不满足服务场景，需要在进入大规模开发前重新评估：

```text
QuickJS-ng
JavaScriptCore
V8 Compact Profile
静态 Route / JSON Fast Path
Wasm-first Service Profile
```

---

## 31. 商业与开源边界

建议采用：

```text
Tysel Runtime          Apache-2.0
Tysel SDK              Apache-2.0
Capability SDK         Apache-2.0
Core CLI               Apache-2.0
```

潜在商业部分放在 Runtime 之外：

```text
Tysel Fleet
企业策略中心
集中 Secret 管理
Capability Registry
多租户调度
审计与合规
可观测性平台
托管 Durable Store
企业连接器
商业支持
```

第一阶段不做云平台。

先证明 Runtime 在真实项目中的：

```text
小
快
安全
可迁移
可部署
```

---

## 32. 最终产品边界

```text
                         Tysel

        Trusted Service                 Isolated Task
               │                              │
      ┌────────┼────────┐            ┌────────┼────────┐
      ▼        ▼        ▼            ▼        ▼        ▼
     HTTP    Worker    Queue         MCP     Plugin    Agent
      │        │        │            │        │        │
      └────────┴────────┴────────────┴────────┴────────┘
                              │
                              ▼
                            Task
                              │
                 ┌────────────┼────────────┐
                 ▼            ▼            ▼
              QuickJS       Wasm        Static AOT
                 │            │            │
                 └────────────┼────────────┘
                              ▼
                     Capability Runtime
                              │
            ┌─────────────────┼─────────────────┐
            ▼                 ▼                 ▼
          HTTP              Database           LLM
```

Tysel 不需要在第一天取代 Node.js。

更可行的路径是：

1. 先成为更好的 TypeScript 单文件 Service Runtime。
2. 再成为更安全的 Agent 与 Tool Runtime。
3. 再成为统一的 Durable Task Runtime。
4. 最后逐步把静态 TypeScript 热路径从 JavaScript Engine 移到 Wasm 和 Native。

最终护城河不是单独的 QuickJS、Rust 或 Wasm，而是以下能力的组合：

```text
TypeScript DX
+
Single Binary Deployment
+
Native Capability
+
Web Standard Compatibility
+
Trusted / Isolated Dual Profile
+
Durable Task Execution
+
Wasm Component ABI
+
渐进式 Static AOT
```

最终目标可以概括为：

> **Tysel 让现代 TypeScript 服务和 Agent 以一个更小、更快、更安全的可执行程序运行；普通代码保持 TypeScript 开发体验，关键能力由 Native 与 Wasm 承担，等待中的 Agent 则可以持久化挂起而不长期占用完整 Runtime。**
