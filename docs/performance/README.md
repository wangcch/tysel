# Performance

Benchmark harnesses and reproduction commands live in `/benchmarks`. Release admission
checks a packaged hello service against these current gates:

- Median cold start: no more than 15 ms
- Idle Linux PSS: no more than 32 MiB
- Packaged executable: no more than 20 MiB

These are release-admission thresholds, not capacity promises. Evidence records include
the raw samples, artifact digest, source commit, command, CPU, operating system, and
target. Linux PSS is the memory result of record; macOS RSS is reported only as a proxy.
