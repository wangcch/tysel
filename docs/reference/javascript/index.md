# JavaScript APIs

Tysel implements a bounded server-side Web API profile. The versioned source
of truth is `runtime-js/web-api/compatibility.json`; it distinguishes supported,
partial, and intentionally excluded behavior for every global.

## API groups

| Group | Included globals |
| --- | --- |
| HTTP values | `Request`, `Response`, `Headers`, `fetch` |
| URLs and encoding | `URL`, `URLSearchParams`, `TextEncoder`, `TextDecoder` |
| Scheduling and events | timers, `Event`, `EventTarget`, `AbortController`, `AbortSignal` |
| Security | `crypto.getRandomValues`, supported `crypto.subtle` digest and HMAC operations |
| Realtime | inbound and outbound `WebSocket` subsets |

Use the per-global pages in the
[public JavaScript API reference](https://tysel.dev/reference/javascript/) for
the complete supported and excluded behavior.

`partial` is a supported subset, not a promise that undocumented browser
behavior works. Tysel is server-side and does not implement browser policy,
DOM, or general Node.js compatibility. Run [`tysel compat`](../../compatibility/README.md)
when evaluating a package rather than inferring support from a global name.
