# Performance

Benchmark harnesses and reproduction commands live in `/benchmarks`. Release admission
checks these current gates:

- Median cold start: no more than 15 ms
- Idle Linux PSS: no more than 32 MiB
- Packaged executable: no more than 20 MiB
- Warm in-process isolate creation p50: no more than 5 ms
- Durable Task resume p50: no more than 10 ms

These are release-admission thresholds, not capacity promises. Evidence records include
the raw samples, artifact digest, source commit, command, CPU, operating system, and
target. Schema v2 adds isolate, task, durable, and HTTP suite reports with every raw
sample. Full latency distributions use 101 samples and publish p50/p95/p99;
singleton memory/size measurements omit unsupported tail percentiles. Metrics
without a roadmap threshold remain observational.
Linux PSS is the memory result of record; macOS RSS is reported only as a proxy.
CI uploads both the legacy release-admission document and the schema v2
multi-suite document from the Linux PSS job.
