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

rquickjs 上游发布包含同等适配的正式版本后，应重新验证并回到正式 crate。若正式发布前需要进入
stable，必须先把临时适配迁移到 Tysel-owned fork，不把第三方可变分支作为发布依赖。
候选 adapter 只允许进入 canary；release workflow 必须根据 compatibility manifest 拒绝 stable 发布。

QuickJS-ng 升级至少需要通过引擎兼容测试、host-backed ArrayBuffer 生命周期测试、原生方法
interrupt 测试、完整 workspace 测试和发布供应链清单校验。底层引擎字节码不作为跨版本持久化格式。
