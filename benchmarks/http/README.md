# http

The packaged hello-service binary size is reported by `tysel-bench` (gate ≤ 20MB).

Request bodies are capped (`max_request_mb`, default 16). Oversized POST returns 413. Throughput and Keep-Alive load tests are M1.
