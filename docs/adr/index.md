# Architecture decision records

Architecture decision records explain why Tysel has its current boundaries.
They are useful when changing the runtime or evaluating its tradeoffs; they are
not a substitute for the public API and operations contracts.

1. [Rust runtime core](001-runtime-core-rust.md)
2. [QuickJS engine](002-quickjs-ng-engine.md)
3. [Web-API-first application contract](003-web-api-first.md)
4. [Build once and ship one file](004-build-once-ship-one-file.md)
5. [Deny by default](005-deny-by-default.md)
6. [Process isolation](006-process-isolation.md)
7. [Durable replay](007-durable-replay.md)
8. [WIT capability ABI](008-wit-capability-abi.md)
9. [No AOT requirement on the v1 path](009-no-aot-on-v1-path.md)
10. [Static TypeScript frontend work in parallel](010-static-typescript-parallel.md)

Start with the [architecture overview](../architecture/README.md) for the
system boundaries and current implementation map.
