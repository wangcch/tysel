# ADR-002：v0.x 使用 QuickJS-ng 作为兼容执行引擎

状态：Accepted

## 决策

v0.x 的 Compatibility Engine 使用 QuickJS-ng。引擎通过 `tysel-engine::ExecutionEngine` 抽象，不把 QuickJS 写死为永久架构。

## 理由

体积小、易嵌入、启动开销低、支持现代 ECMAScript，且不含大型 JIT。完整 JS 语义的 AOT 不进入 v1 关键路径。

## 版本与升级边界

当前兼容执行引擎固定为 QuickJS-ng 0.16.2（`2c620e4`），通过 rquickjs
0.12.2 的适配提交 `810b2b6` 接入。依赖必须使用不可变 revision；
`runtime-js/compatibility.json` 是 adapter identity、编译期版本和发布 SBOM 的唯一版本来源。
供应链生成必须校验其 rquickjs revision 与 `Cargo.lock` 中的生产依赖完全一致，并从该
rquickjs 提交的 `sys/quickjs` gitlink 验证实际 QuickJS-ng revision。

rquickjs 上游发布包含同等适配的正式版本后，应重新验证并回到正式 crate。当前适配提交虽然位于
第三方 fork，但依赖使用完整 revision，`Cargo.lock` 固定同一来源，供应链生成校验 adapter revision
及其 QuickJS gitlink，发布产物再由 Tysel release metadata 签名。因此 fork 后续发生分支移动不会
改变已构建或已发布的字节；仓库不可用会阻止重建，而不会让依赖静默漂移。迁移到 Tysel-owned mirror
仍是 v1.0 前的连续性改进，不再作为 v0.x stable 的绝对前置条件。

2026-09-04 的 stable qualification 复核了 QuickJS-ng `0.16.2` 之后的固定提交、当时全部公开安全
公告的修复版本、rquickjs workspace 测试、Tysel 引擎兼容与 host-backed ArrayBuffer 生命周期测试、
原生方法 interrupt 测试、完整 workspace 测试和发布供应链清单校验。该 adapter 因此标记为
`validated`，允许进入 canary 与 stable。该结论只适用于清单中的两个完整 revision；任一 revision
变化都必须重新降为 candidate 并完成同等验证。公开漏洞修复不等于不存在未知内存安全缺陷，生产
`isolated` profile 仍必须使用文档规定的 Linux 进程与操作系统隔离。

QuickJS-ng 升级至少需要通过引擎兼容测试、host-backed ArrayBuffer 生命周期测试、原生方法
interrupt 测试、完整 workspace 测试和发布供应链清单校验。底层引擎字节码不作为跨版本持久化格式。
