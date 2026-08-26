# WebSocket service

This service accepts an HTTP/1.1 WebSocket upgrade at `/ws` and echoes text
frames. The connection consumes one `max_in_flight` permit until it closes.

```sh
tysel check
tysel run
```

From another terminal:

```sh
curl -i http://127.0.0.1:3000/
websocat ws://127.0.0.1:3000/ws
```

See the [service networking guide](../../docs/guides/service-networking.md).
