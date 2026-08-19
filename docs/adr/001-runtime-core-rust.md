# ADR-001：Runtime Core 使用 Rust

状态：Accepted

## 决策

Tysel Runtime Core 使用 Rust 实现。

## 理由

内存安全适合 Runtime 与 Sandbox；可直接集成 Wasmtime；适合网络、调度、IPC 与 Capability Broker；跨 Linux / macOS / Windows；可通过 LTO、feature 裁剪和 `panic=abort` 控制体积。
