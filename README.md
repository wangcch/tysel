# Tysel

> **A lightweight native runtime for TypeScript services and agents.**
>
> **Write TypeScript. Ship a binary.**

Tysel 是面向现代后端服务、任务与 AI Agent 的轻量 TypeScript Runtime。开发者编写标准 TypeScript，构建后得到单个原生可执行文件，生产环境不需要 Node、V8 或 `node_modules`。

当前仓库处于 **M0 脚手架**：工作区、CLI、manifest schema 与 SDK 类型已就位。下一步是 `roadmap.md` 中的四个可行性 Spike。

## 仓库结构

```text
crates/          Rust workspace（Runtime Core、CLI、Capability）
packages/        TypeScript SDK 与类型
runtime-js/      Isolate 内 bootstrap / Web API / Durable client
wit/             Capability WIT ABI（实验性）
examples/        演示应用
docs/adr/        已采纳的架构决策
benchmarks/      性能对照（尚未填充）
```

## 要求

- Rust 1.85+（`rustup` stable）
- Node.js 22+
- pnpm 11+
- TypeScript 7 CLI（`tsc --noEmit`，不嵌入 Compiler API）

## 快速开始

```bash
pnpm install
cargo test --workspace
cargo run -p tysel-cli -- --help
cargo run -p tysel-cli -- inspect --manifest examples/hello-service/tysel.toml
```

最小应用：

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

`tysel dev` / `tysel build` 将在 M0 Spike 通过后实现。完整规划见 [roadmap.md](./roadmap.md)。

## 许可证

Apache-2.0
