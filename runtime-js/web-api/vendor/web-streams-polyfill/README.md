# Web Streams vendor snapshot

web-streams-polyfill 4.3.0, MIT license (see LICENSE).
Source: https://github.com/MattiasBuelens/web-streams-polyfill/tree/v4.3.0
Package: https://registry.npmjs.org/web-streams-polyfill/-/web-streams-polyfill-4.3.0.tgz
Upstream SHA-256: `30835bf3c9af5236b16126f83512d5daaf5e6b53d93ae4e61efadce775857a95`
polyfill.js SHA-256: `aa87937bc409a8c6b1588fc848d40241a1e3220a2ff47b0a0460ac2af03a7477`

The ES2015 polyfill with the local patch below is embedded by the QuickJS adapter in
native read-only storage. build-runtime.mjs embeds the full MIT license and a
lazy loader; first access to a Streams constructor asks the native host to
evaluate the fixed vendor source. Buffered HTTP bodies use a loader-owned brand
check to avoid triggering initialization.
Update the pinned snapshot, types, license, checksum and QuickJS tests together.

The type snapshot adapts its AbortSignal alias to TyselAbortSignal so explicit
runtime types compose without depending on the complete ambient DOM interface.

## Local pipe backpressure patch

In `br` (ReadableStreamPipeTo), the `g` pipe step originally returns
`f(s._readyPromise,g)` while waiting for backpressure to clear. Repeated waits
retain a chain of adopted promises until piping finishes. On QuickJS this grows
with the number of chunks and can exhaust the isolate heap.

The snapshot replaces that expression with `f(s._readyPromise,()=>!1)`.
The existing outer pipe loop receives `false` and schedules the next step after
readiness, without linking each step's promise to the rest of the transfer.
Readiness rejection still reaches the existing pipe-loop rejection handler.
No other vendor runtime code is changed. Reassess this patch on vendor upgrades;
low-heap long-pipe, cancellation and error-propagation tests cover it.
