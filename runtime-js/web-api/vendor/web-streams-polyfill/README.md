# Web Streams vendor snapshot

web-streams-polyfill 4.3.0, MIT license (see LICENSE).
Source: https://github.com/MattiasBuelens/web-streams-polyfill/tree/v4.3.0
Package: https://registry.npmjs.org/web-streams-polyfill/-/web-streams-polyfill-4.3.0.tgz
polyfill.js SHA-256: `30835bf3c9af5236b16126f83512d5daaf5e6b53d93ae4e61efadce775857a95`

The unmodified published ES2015 polyfill is embedded by the QuickJS adapter in
native read-only storage. build-runtime.mjs embeds the full MIT license and a
lazy loader; first access to a Streams constructor asks the native host to
evaluate the fixed vendor source. Buffered HTTP bodies use a loader-owned brand
check to avoid triggering initialization.
Update the pinned snapshot, types, license, checksum and QuickJS tests together.

The type snapshot adapts its AbortSignal alias to TyselAbortSignal so explicit
runtime types compose without depending on the complete ambient DOM interface.
The runtime polyfill itself is unmodified.
