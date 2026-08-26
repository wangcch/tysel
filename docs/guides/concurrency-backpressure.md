# Size concurrency and backpressure

This guide configures service workers and HTTP admission so overload is bounded
and visible instead of becoming an unbounded queue.

## Start with one worker

Each `service` worker owns an independent QuickJS isolate, global scope, and
memory budget. Begin with one worker and externalize shared state before adding
more:

```toml
[server]
workers = 1

[limits]
memory_mb = 128
request_timeout_ms = 15000
max_in_flight = 100
max_request_mb = 4
max_response_mb = 8
```

`workers` can range from 1 to 64 only in the `service` profile. Isolated and
Component applications require one worker. Raising the count multiplies the
isolate memory budget and does not make module globals shared.

## Understand the admission permit

`max_in_flight` is shared by the HTTP listener. Tysel tries to acquire a permit
before dispatching a request:

- no permit means an immediate HTTP `503`;
- the JSON error code is `OVERLOADED`;
- the response contains `Retry-After: 1`;
- Tysel does not create an unbounded waiter queue;
- the permit remains held until the response body completes or is dropped;
- an accepted WebSocket holds it until the upgraded connection closes.

Setting `max_in_flight = 0` deliberately sheds every request and can serve as a
circuit breaker. It does not mean unlimited.

## Choose an initial value

Use measured service time and a target throughput as a starting estimate:

```text
in-flight demand ≈ peak requests/second × p95 service time in seconds
```

Add a small, explicit burst margin, then load-test the complete deployment.
For example, 200 requests/second with a 250 ms p95 begins near 50 concurrent
requests, before the chosen burst margin. This is a sizing hypothesis, not a
universal recommendation.

If WebSockets share the listener, reserve capacity for their expected
simultaneous lifetime or separate the workload operationally. A few persistent
sockets can consume a small admission limit indefinitely.

## Validate overload behavior

Run the service, then generate more concurrent requests than the configured
limit with the load tool used by your team. Check the response contract directly:

```sh
curl -i http://127.0.0.1:3000/slow
```

During overload, clients should observe a response shaped like:

```http
HTTP/1.1 503 Service Unavailable
content-type: application/json
retry-after: 1

{"error":{"code":"OVERLOADED","message":"maximum in-flight request limit reached","requestId":"…"}}
```

Retry only idempotent operations automatically. Use bounded exponential
backoff with jitter at the caller; `Retry-After: 1` is a minimum hint, not an
instruction for every client to retry simultaneously.

## Tune in the right order

1. Bound request and response bodies.
2. Set a request deadline that is shorter than the caller and ingress deadline.
3. Measure one worker under representative CPU, SQL, LLM, and streaming work.
4. Set admission from observed service time and acceptable queueing.
5. Add workers only when handlers are stateless and CPU or isolate scheduling
   is the demonstrated constraint.
6. Recheck memory, startup, p95/p99 latency, `503` rate, and WebSocket lifetime.

Increasing workers or `max_in_flight` can worsen memory pressure and tail
latency. Neither setting replaces upstream connection pools, database bounds,
provider concurrency limits, or deployment-level CPU and memory controls.

## Failure interpretation

| Symptom | Likely boundary | Action |
| --- | --- | --- |
| Immediate `503 OVERLOADED` | HTTP admission full | Reduce demand, shorten work, or raise the measured limit carefully. |
| `413 BODY_TOO_LARGE` | Request-body limit | Reject or chunk earlier; raise only for a documented payload need. |
| `500 RESPONSE_TOO_LARGE` | Response-body limit | Paginate, stream within the bound, or reduce output. |
| Runtime timeout | Handler deadline | Bound upstream calls and keep the ingress deadline longer than Tysel's. |
| Memory grows after adding workers | Per-isolate budget multiplied | Reduce worker count or state; confirm external storage and cache strategy. |

See [Application limits](../reference/manifest/limits.md),
[Limits and defaults](../reference/limits-and-defaults.md), and
[Performance evidence](../performance/README.md).
