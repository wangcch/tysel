# http

Run `tysel bench http`. The local loopback suite covers HTTP/1.1 keep-alive,
HTTP/2, JSON 1KB/64KB responses, a 64KB byte response, streaming, WebSocket echo, SSE, and concurrency
levels. HTTP/1.1 and HTTP/2 are reported as separate curves with fixed connection
strategies; full measurements use 101 samples and contain p50/p95/p99.

These end-to-end timings are observational. They are not used for the roadmap's
“HTTP Handler Runtime extra overhead” gate because that gate requires a matched
transport baseline and subtraction methodology.
