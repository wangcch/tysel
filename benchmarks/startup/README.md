# startup

Cold start is spawn of a packaged `hello-service` until stdout prints `tysel listen`.

```bash
cargo run -p tysel-testkit --bin tysel-bench --release
```

Gate: p50 ≤ 15ms. The `linux-pss` CI job on `ubuntu-latest` is the
measurement of record; local macOS numbers are a proxy.
