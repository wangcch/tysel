# Performance and evidence

Tysel keeps benchmark methodology in the repository so size, startup, memory,
isolate, durable, task, and HTTP results can be reproduced instead of copied
as unsupported marketing claims.

## Current release-admission thresholds

| Metric | Gate | Measurement of record |
| --- | --- | --- |
| Median cold start | at most 15 ms | Linux benchmark job |
| Idle memory | at most 32 MiB PSS | Linux PSS |
| Packaged executable | at most 20 MiB | Built artifact size |
| Warm isolate creation p50 | at most 5 ms | In-process isolate suite |
| Durable task resume p50 | at most 10 ms | Durable resume suite |

These are engineering admission thresholds, not universal capacity or latency
promises. Results depend on hardware, kernel, application bundle, configuration,
measurement scale, and workload.

## Run benchmarks

```sh
tysel bench startup
tysel bench all --format json
tysel bench all --evidence dist/benchmark-evidence.json
```

Complete evidence requires a release-mode, full-scale `all` run. Schema v2
records raw samples, artifact digest, source commit, command, CPU, operating
system, target, and available p50/p95/p99 values. Short or singleton
measurements omit unsupported tail percentiles.

Linux PSS is the memory result of record. macOS RSS is a development proxy.
Metrics without an explicit gate are observational and do not fail release
admission.

Suite-specific setup and interpretation live in the
[benchmark README](https://github.com/wangcch/tysel/tree/main/benchmarks).
Publish an exact number only with the artifact, environment, command, and
evidence document that produced it.
