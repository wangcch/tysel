# Phase 3 acceptance

Phase 3 closes the production-capability gaps identified after the developer-experience milestone. It is complete when the following contracts and automated evidence pass.

| Area | Acceptance contract | Automated evidence |
| --- | --- | --- |
| Web Crypto | SHA-256/384/512 digest and raw HMAC import/sign/verify; normalized names, opaque `CryptoKey`, enforced usages, native constant-time verification. | Known digest/HMAC vectors, altered signature, invalid usage, and key-access tests. |
| Outbound WebSocket | Trusted code can connect one request-scoped socket with `ws`/`wss`, receive text/binary messages, send text, and close. The fetch host allowlist, capability policy, request deadline, cancellation, and audit log apply. | Loopback handshake and echo integration test; isolated policy tests cover denial. |
| HTTP/2 | Manifest flags reach development and packaged servers. HTTP/1-only, HTTP/2-only h2c, and dual-protocol listeners are supported; both flags false is rejected. WebSocket upgrade remains HTTP/1.1-only. | h2 prior-knowledge request integration test and manifest validation test. |
| npm compatibility | `tysel compat` reports compatible, shim, unsupported, and unknown dependencies with stable JSON and CI exit policy. Scoped packages and subpaths classify by their package root. | CLI human/JSON/strict integration tests and package-root unit tests. |
| OTLP | Traces and metrics export over OTLP/HTTP with bounded endpoint configuration, redacted attributes, and clean-shutdown flush. | Existing M5 fake-collector protobuf integration test and configuration/redaction unit tests. |

HTTP/2 is cleartext at the Tysel listener. Public deployments terminate TLS at an ingress or reverse proxy. `wss` uses the platform certificate store. OTLP and `tysel compat` build on the production and Phase 2 contracts rather than introducing parallel implementations.
