# ADR-002：v0.x 使用 QuickJS-ng 作为兼容执行引擎

状态：Accepted

## 决策

v0.x 的 Compatibility Engine 使用 QuickJS-ng。引擎通过 `tysel-engine::ExecutionEngine` 抽象，不把 QuickJS 写死为永久架构。

## 理由

体积小、易嵌入、启动开销低、支持现代 ECMAScript，且不含大型 JIT。完整 JS 语义的 AOT 不进入 v1 关键路径。
