# memory

Idle memory is sampled after listen, before any request.

- Linux: `/proc/<pid>/smaps_rollup` PSS
- macOS: `ps` RSS (proxy; not equivalent to PSS)

```bash
cargo run -p tysel-testkit --bin tysel-bench --release
```

Gate: ≤ 32MB (roadmap §30). Linux PSS is the gate of record; the CI job `§30 linux pss` runs `tysel-bench` on `ubuntu-latest`. macOS RSS is a local proxy only.
