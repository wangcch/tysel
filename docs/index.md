# Tysel

Tysel is a lightweight native runtime for TypeScript services and durable agents. Write against Web APIs, grant platform capabilities explicitly, and ship one executable without Node.js or `node_modules` in production.

## Start here

- [Install](install.md): put `tysel`, `tysel-service`, and `tysel-worker` on `PATH`.
- [Getting started](getting-started.md): create, check, test, run, and package an application.
- [CLI reference](cli.md): command contracts, output formats, and exit behavior.
- [Runtime API](api/runtime.md): handlers, Web APIs, tasks, durable execution, and host capabilities.
- [Capability matrix](capabilities/README.md): what is available in trusted and isolated profiles.
- [npm compatibility](compatibility/README.md): how to interpret `tysel compat`.
- [Durable Agent demo](https://github.com/wangcch/tysel/tree/main/examples/durable-agent): LLM call, human approval, restart, and exactly-once result persistence.
- [Isolated Plugin example](../examples/isolated-plugin/README.md): denied host capabilities and worker recovery.
- [MCP Tool example](../examples/mcp-tool/README.md): bounded stdio, validated calls, and opaque secrets.
- [Production operations](operations/production.md): deployment, recovery, observability, and release evidence.
- [Phase 3 acceptance](architecture/phase3-acceptance.md): Web Crypto, outbound WebSocket, HTTP/2, compatibility, and OTLP evidence.

## Developer loop

```bash
tysel init my-service
cd my-service
tysel check
tysel test
tysel dev
```

Use `tysel --error-format json …` in editors and CI. Test failures are mapped from generated `app.js` frames back to their TypeScript source.
