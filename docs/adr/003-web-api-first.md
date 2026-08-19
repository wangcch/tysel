# ADR-003：Web API 优先，不以 Node API 为基准

状态：Accepted

## 决策

基础 API 优先兼容 Web 标准（`Request` / `Response` / `fetch` / Streams / `crypto` 等）。不以完整 Node.js API 兼容为目标。

## 理由

Node 历史边界会重新引入体积、权限与生态复杂度。服务端最小公共 Web API（ECMA-429）更适合单文件 Runtime。
