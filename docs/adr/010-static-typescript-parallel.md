# ADR-010：Static TypeScript Compiler 作为并行研发路线

状态：Proposed

## 决策

受约束 TypeScript Static Profile → Wasm / Native AOT 作为与 Runtime 主线并行的实验，不阻塞 v1。应用允许混合 QuickJS 模块、Static 模块与 Wasm Component。
