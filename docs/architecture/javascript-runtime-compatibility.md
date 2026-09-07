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
| Headers | Partial | Iterable/record initializers, token validation, independent Set-Cookie values, getSetCookie | Browser header guards |
| Request / Response | Partial | Byte-preserving string/ArrayBuffer/ArrayBufferView bodies, single-use text/JSON/arrayBuffer, bodyUsed, ReadableStream bodies and response output, buffered clone | Form/blob helpers, streamed Body.clone, browser policy fields |
| TextEncoder / TextDecoder | Partial | UTF-8 conversion, stateful decode/flush, TextDecoderStream, encodeInto | Legacy encodings |
| Timers | Supported | Timeout/interval creation, clearing, isolate-reset cleanup | Browser scheduling guarantees |
| Event / EventTarget | Partial | Function/object listeners, deduplication, removal, once/signal options, cancellation | DOM trees, capture/bubble phases, browser exception reporting |
| AbortController / AbortSignal | Partial | EventTarget inheritance, reasons, static abort/timeout/any, fetch and body cancellation | Browser task scheduling guarantees |
| Crypto | Partial | Random values and UUID v4, SHA-2 digest, raw HMAC import/sign/verify | key export, encryption, asymmetric algorithms |
| fetch | Partial | Policy-controlled HTTP, binary-safe buffered uploads and response reads, redirects, abort | Streaming uploads, multipart helpers, browser cache/credential/mode fields |
| Streams | Partial | Default readers, async iteration, writers, TransformStream, pipeTo/pipeThrough, tee, strategies, cancellation | HTTP uploads/BYOB, transfer of host-owned buffers, streamed Body.clone |
| Microtasks/base64 | Supported | Native queueMicrotask scheduling, atob/btoa validation | — |
| WebSocket | Partial | Accepted/outbound text sockets, core events, listener lifecycle, reuse cleanup | Subprotocols, extensions, Blob messages, buffered amount |

## Contract evidence

- Authored implementations live only in `runtime-js/web-api/source/` and
  `runtime-js/capability-client/source/`; generated runtime bundles are checked
  for drift. The pinned MIT-licensed Streams implementation is in
  `runtime-js/web-api/vendor/web-streams-polyfill/`.
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
O(body size) memory; streaming uploads remain unsupported. Applications can use
`body.getReader()` or `pipeThrough()` to avoid whole-body accumulation.

Run the optional local throughput measurement with:

```sh
cargo test -p tysel-engine-qjs http_body_throughput -- --ignored --nocapture
```

It checks 1 MiB ASCII bodies with both text and arrayBuffer consumption, one
warm-up and 20 measured fetches, under a configured 16 MiB JS heap. Timings include
local HTTP I/O and are diagnostic, not a CI threshold or an RSS measurement.

## P1 Streams and lifecycle

The public `Request.body` and `Response.body` properties now return a lazy
`ReadableStream<Uint8Array>` (or null). Code that previously inspected raw body
storage should use readers or `arrayBuffer()`. Untouched buffered responses retain
the native buffered output fast path. Tysel chunk arrays remain an input extension;
the public body is a stream, not that input array.

Native HTTP body streams use a zero high-water mark and read only on demand. The
existing bounded native queues remain in place. Canceling a reader cancels its
in-flight native operation and drops the body receiver. Request generation checks
prevent a retained body from reading I/O belonging to a later request. Whole-body
helpers and individual reads share disturbance and lock state.

`new Response(stream)` accepts Uint8Array chunks. Its pump awaits each bounded
native write; slow consumers apply backpressure. A separate native close watcher
cancels the source even while its next read is pending; an unresolved source
cancel callback cannot retain the worker. The close race is installed once per
response so promise reactions do not accumulate per chunk.
The isolate stays assigned until the response stream finishes, fails, disconnects,
or reaches its normal CPU/request deadline. After headers have been sent, failures
are surfaced as HTTP body errors rather than a successful truncated response.
`dispatch_response` exposes checked completion; the legacy chunk-only
`dispatch_incoming` API cannot surface errors after its head. `dispatch_sync`
collects checked completion and rejects on stream failure.

Streams are embedded from web-streams-polyfill 4.3.0 in native read-only storage.
Constructor access loads the implementation on demand; buffered bodies avoid that
initialization. First-use loading is charged to the active isolate budgets. The reviewed contract covers
default streams; native HTTP bodies do not expose BYOB readers. Buffered body clone
remains supported, but streaming Body.clone is explicitly excluded. The vendor's
`_disturbed` flag is an internal adapter dependency bound by lifecycle tests.

TextDecoder preserves incomplete UTF-8 prefixes (at most three bytes), handles BOM
only at the start of each decoding session, and flushes pending bytes on the final
`decode()`. TextDecoderStream uses those same semantics. `atob`/`btoa` operate on
binary Latin-1 strings, not Unicode text. `AbortSignal.any` propagates dependencies
independently of whether a public abort listener stops event propagation. All
dependents are marked before dispatching events, so reentrant abort callbacks
cannot change the first reason. Weak dependencies and finalization cleanup allow
discarded composites to be collected. Synchronous HTTP handlers also drain jobs
within their request budgets, including microtasks and finalization callbacks.

### Local performance checks

The optional release profile separates bootstrap, first Streams initialization,
fresh-isolate startup, small JSON dispatch, and buffered/streamed output:

```sh
cargo run --release -p tysel-engine-qjs --example p1_profile -- cold
cargo run --release -p tysel-engine-qjs --example p1_profile -- bootstrap-streams
cargo run --release -p tysel-engine-qjs --example p1_profile -- small
cargo run --release -p tysel-engine-qjs --example p1_profile -- stream16k
```

Compare equivalent release builds on the same idle machine and alternate versions.
These are local diagnostics; packaged startup, HTTP and Linux PSS use the existing
[benchmark workflow](../performance/README.md).

### URL subset semantics

Dot-segment normalization preserves empty path segments, including repeated
slashes and trailing slashes, through construction, relative resolution and
pathname updates. Default ports are omitted for HTTP/HTTPS/WS/WSS/FTP, including
host, port, href and protocol changes. Query parsing decodes valid percent bytes
next to malformed input, replaces malformed UTF-8, preserves BOM and normalizes
unpaired UTF-16 surrogates in parameter strings. Live searchParams identity is
preserved when search is reassigned. These changes do not add full WHATWG URL
parsing, IDNA or file/blob support.

The focused differential fixtures use Node for comparable supported cases and
explicit expectations for raw Unicode plus malformed percent input, where the
local Node 22 reference differs from the [URL form-query parsing algorithm](https://url.spec.whatwg.org/#concept-urlencoded-parser).
QuickJS regressions and a real bundled Hono route exercise the native runtime.

### Body consumption and Headers iteration

Buffered body helpers still avoid Streams initialization. If a consumed buffered
body is subsequently exposed as a stream, it is materialized as closed and
already disturbed using a public reader operation. This matches the existing
helper path's closed, disturbed, unlocked state: Tysel releases its internal
reader after whole-body consumption. The consumed stream cannot be accepted as a
new Response body, regardless of when `.body` was first accessed. Request and
Response helpers, clone rejection, bodyUsed and stream identity are covered.

Headers iterators index the current sorted, flattened list, including separate
Set-Cookie entries. Mutations invalidate a per-iterator snapshot; unchanged lists
are not repeatedly sorted. Deletion, updates, insertion before/after the cursor,
and append after a done result are covered for entries/keys/values. This follows
the live cursor behavior in the [Web IDL iterator algorithm](https://webidl.spec.whatwg.org/#default-iterator-objects),
without changing the native header serialization path.

### Encoding into caller buffers and Response factories

`TextEncoder.encodeInto` writes into the selected Uint8Array view and reports
UTF-16 code units read and UTF-8 bytes written. It replaces unmatched surrogates
and never writes a partial code point. Conversion uses at most 16 Ki UTF-16 units
per native call, producing at most 48 KiB of UTF-8 payload. This payload limit
excludes the terminator, string metadata, allocator rounding and JS temporary
objects; it is not an upper bound on allocated memory or request peak memory.
It does not allocate an encoded result proportional to the full source when the
destination is short. Chunking bounds conversion scratch, rather than promising
zero allocation or faster execution for every size.

`Response.redirect` accepts absolute URLs within the supported URL subset and
301/302/303/307/308 statuses. It creates an immutable Location header.
`Response.error` creates a status-0, type-error response with immutable headers;
returning it from a handler fails native response preparation rather than sending
an invalid HTTP status. Clones preserve these immutable headers. Other response
header guards remain outside the supported subset.

Ordinary Response construction accepts statuses 200–599 and rejects non-null
bodies for 204, 205 and 304. Tysel's existing status-101 WebSocket upgrade remains
available only after `tysel.acceptWebSocket()`, also with a null body. Null-body
helpers remain repeatable and leave bodyUsed false.

### Byte consumption and text encoding streams

`Request.bytes()` and `Response.bytes()` return an independent `Uint8Array`
with the same consumption rules as `arrayBuffer()`. They collect the entire
body, requiring O(body size) memory.

`TextEncoderStream` converts text chunks to UTF-8 for `pipeThrough()` and
streaming responses. It retains at most one trailing high surrogate to join
split pairs and replaces unmatched surrogates, including on close. Chunks use
DOMString conversion, which rejects Symbols. The underlying TransformStream
loads on construction and supplies backpressure, cancellation and error propagation.
Output allocation scales with each input chunk, so applications should bound
chunk sizes.

These contracts follow the [Body bytes helper](https://fetch.spec.whatwg.org/#dom-body-bytes)
and [TextEncoderStream algorithms](https://encoding.spec.whatwg.org/#interface-textencoderstream).
