# Configure service networking

This guide exposes a Tysel service deliberately: choose its listener and HTTP
protocols, enable WebSocket upgrades, grant outbound destinations, and verify
the failure paths before deployment.

## Prerequisites

- Tysel installed and passing `tysel doctor --install`;
- the `service` profile for inbound WebSockets and outbound network access;
- a TLS-terminating ingress or reverse proxy for public HTTPS;
- optionally, `websocat` or another WebSocket client for the upgrade check.

## Configure the listener

Start with HTTP/1.1 on loopback during development:

```toml
[app]
name = "websocket-service"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:3000"
http1 = true
http2 = false
websocket = true

[permissions]
fetch = ["api.example.com"]
```

`listen` controls inbound traffic. `permissions.fetch` controls outbound
`fetch` and WebSocket destinations; neither setting implies the other. Change
the listener to `0.0.0.0:3000` only inside a container or host where the port is
intended to be reachable.

Tysel serves cleartext HTTP/2 (h2c), not TLS. Enabling both versions performs
protocol selection on one listener. HTTP/2-only clients must use h2c prior
knowledge. WebSocket upgrades require HTTP/1.1:

```toml
[server]
http1 = true
http2 = true
websocket = true
```

Terminate HTTPS at an ingress and forward HTTP/1.1 upgrades without removing
`Connection` and `Upgrade`. Do not expose a cleartext public listener as a
substitute for TLS termination.

## Implement an inbound WebSocket

```ts
export default {
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const upgrade = request.headers.get("upgrade")?.toLowerCase();
    if (url.pathname !== "/ws" || upgrade !== "websocket") {
      return Response.json({ websocket: "/ws" }, { status: 426 });
    }

    const socket = tysel.acceptWebSocket();
    socket.addEventListener("message", async (event) => {
      await socket.send(`echo:${event.data}`);
    });
    return new Response(null, { status: 101 });
  },
};
```

`acceptWebSocket()` is valid only while an eligible inbound HTTP/1.1 upgrade is
being handled. The accepted socket supports text messages and the bounded event
surface documented in the [WebSocket contract](https://tysel.dev/reference/javascript/websocket/).

Run the complete [WebSocket service example](https://github.com/wangcch/tysel/tree/main/examples/websocket-service):

```sh
cd examples/websocket-service
tysel check
tysel run
```

In another terminal:

```sh
curl -i http://127.0.0.1:3000/
websocat ws://127.0.0.1:3000/ws
```

The HTTP request returns `426`; text entered through `websocat` returns with an
`echo:` prefix.

## Call an outbound destination

The allowlist contains hostnames, without schemes, ports, paths, or wildcards:

```toml
[permissions]
fetch = ["api.example.com"]
```

```ts
const response = await fetch("https://api.example.com/v1/status");
if (!response.ok) throw new Error(`upstream returned ${response.status}`);
```

Every redirect target is checked again. IP literals are rejected, DNS is
resolved by the host, private and special-use addresses are denied, and a
redirect cannot escape the declared hostname set. An outbound WebSocket uses
the same allowlist and permits one active socket per isolate request.

## Verify protocols and denials

```sh
# HTTP/1.1
curl --http1.1 -i http://127.0.0.1:3000/

# h2c prior knowledge, when http2 is enabled
curl --http2-prior-knowledge -i http://127.0.0.1:3000/

# Effective profile and grants
tysel inspect
```

Also test an undeclared outbound hostname and confirm that the host rejects it.
Do not turn a denied-host failure into a generic success response; preserve a
bounded application error and log the request identifier.

## Production checklist

- bind externally only behind the intended network policy;
- terminate TLS and preserve HTTP/1.1 upgrade headers;
- declare exact outbound hostnames and test redirect behavior;
- set body, timeout, and admission limits for the workload;
- include long-lived WebSockets in concurrency sizing;
- use application-owned readiness endpoints.

Continue with [Concurrency and backpressure](concurrency-backpressure.md), the
[server fields](../reference/manifest/app-server.md), and the
[network capability contract](../reference/manifest/permissions.md#outbound-network).
