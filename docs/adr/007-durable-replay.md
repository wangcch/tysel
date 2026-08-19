# ADR-007：Durable Task 使用 Replay，而不是 JS Heap Snapshot

状态：Accepted

## 决策

Agent 挂起与恢复使用 Event Log + Deterministic Replay + 显式 Durable API（`ctx.step` / `ctx.effect` / `ctx.sleep`）。不把任意 JS Heap Snapshot 作为主方案。
