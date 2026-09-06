# JavaScript runtime compatibility

Browse the per-API contract pages in the
[JavaScript API reference](../reference/javascript/index.md).
This page explains rationale, evidence, and the full matrix.

Tysel implements a deliberately bounded, stable server-side Web API profile.
`partial` means the listed behavior is a supported contract, not that the
corresponding browser specification is implemented in full. Behavior outside
the listed subset is not guaranteed. The machine-readable source for this
matrix is `runtime-js/web-api/compatibility.json`. Per-API lookup pages live
in the [JavaScript reference](../reference/javascript/index.md).

| API | Status | Supported contract | Important exclusions |
| --- | --- | --- | --- |
| URL / URLSearchParams | Partial | Authority URLs, relative resolution, dot segments, live query mutation, iterable parameters | Full WHATWG parsing, IDNA, credentials, file/blob URLs |
| Headers | Partial | Common initializers and mutations, stable iteration | Header guards, Set-Cookie splitting, full token validation |
| Request / Response | Partial | Byte-preserving string/ArrayBuffer/ArrayBufferView bodies, single-use text/JSON/arrayBuffer, bodyUsed, streamed host bodies, buffered clone | Public ReadableStream, form/blob helpers, stream teeing/clone, browser policy fields |
| TextEncoder / TextDecoder | Partial | UTF-8 conversion | Legacy encodings and streaming decode options |
| Timers | Supported | Timeout/interval creation, clearing, isolate-reset cleanup | Browser scheduling guarantees |
| Event / EventTarget | Partial | Function/object listeners, deduplication, removal, once/signal options, cancellation | DOM trees, capture/bubble phases, browser exception reporting |
| AbortController / AbortSignal | Partial | EventTarget inheritance, reasons, static abort/timeout, fetch and body cancellation | `AbortSignal.any` |
| Crypto | Partial | Random values, SHA-2 digest, raw HMAC import/sign/verify | UUID, key export, encryption, asymmetric algorithms |
| fetch | Partial | Policy-controlled HTTP, binary-safe buffered uploads and response reads, redirects, abort | Streaming uploads, multipart helpers, browser cache/credential/mode fields |
| WebSocket | Partial | Accepted/outbound text sockets, core events, listener lifecycle, reuse cleanup | Subprotocols, extensions, Blob messages, buffered amount |

## Contract evidence

- Authored implementations live only in `runtime-js/web-api/source/` and
  `runtime-js/capability-client/source/`; generated runtime bundles are checked
  for drift.
- Public host types flow from `runtime-js` through `@tysel/types`.
- `tysel-engine-qjs` tests exercise the supported behavior inside QuickJS,
  including native I/O, cancellation, backpressure, WebSocket lifecycle, and
  isolate reuse.
- `runtime-js` contract tests bind source ownership, generated artifacts, the
  engine adapter, and compatibility manifests.

Adding a supported feature requires updating the authored implementation, its
public type when applicable, a QuickJS behavior test, and this matrix. Features
outside the matrix remain unsupported even if incidental behavior appears to
work.

## HTTP byte body implementation

HTTP chunks are copied into QuickJS-allocated Uint8Array storage so every retained
chunk counts against the isolate heap limit, including the single-chunk fast path.
Body reads
preserve arbitrary bytes and typed-array/DataView offsets. `text()` decodes UTF-8
only after collection, so code points split across transport chunks remain intact;
`arrayBuffer()` never decodes text. Buffered binary constructors and clones snapshot
the selected bytes. Tysel response chunk arrays snapshot the list and every binary
element; DataView and other typed views are normalized to Uint8Array while preserving
byte offsets and chunk boundaries. Body helpers concatenate the same bytes, and
clones have independent snapshots. Empty-body `json()` rejects with a SyntaxError.

For performance, a single received chunk is reused; multiple chunks use one
exact-size allocation and one linear copy pass. Buffered strings retain their
text fast path. UTF-8 decoding borrows valid input while constructing the QuickJS
string, avoiding an intermediate owned Rust string in both strict and replacement
modes; invalid input still allocates the required replacement text. Uploads take
one native snapshot before asynchronous I/O, and redirects share that native buffer.
The existing bounded channels, cancellation,
body limits, and isolate cleanup remain in effect. Full-body helpers still require
O(body size) memory; public Streams and streaming uploads remain unsupported.

Run the optional local throughput measurement with:

```sh
cargo test -p tysel-engine-qjs http_body_throughput -- --ignored --nocapture
```

It checks 1 MiB ASCII bodies with both text and arrayBuffer consumption, one
warm-up and 20 measured fetches, under a configured 16 MiB JS heap. Timings include
local HTTP I/O and are diagnostic, not a CI threshold or an RSS measurement.
