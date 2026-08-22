# isolate

Run `tysel bench isolate`. Full latency distributions record 101 raw samples and
p50/p95/p99 for cold worker creation, warm in-process isolate creation, warm
dispatch, timeout reclamation, and crash replacement. Singleton memory and reuse
measurements retain their raw value without synthetic tail percentiles.

The release gate applies only to `warm_create_ms` (p50 ≤ 5ms).
