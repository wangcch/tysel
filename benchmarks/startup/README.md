# startup

Cold start is spawn of a packaged `hello-service` until stdout prints `tysel listen`.

```bash
cargo run -p tysel-testkit --bin tysel-bench --release
```

Gate: p50 ≤ 15ms (roadmap §30).
