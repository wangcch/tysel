# Isolated MCP Tool

This example exposes a `lookup` tool over bounded newline-delimited MCP stdio.
It runs in the `isolated` profile and returns only the opaque
`secret:OPENAI_API_KEY` handle. The raw environment value remains in the
supervisor and is never returned to the tool caller.

## Run the stdio demonstration

Install Tysel, then run these commands from the example directory:

```bash
tysel doctor --install
tysel config validate
```

Then send discovery, listing, and invocation requests. Each input line produces
one JSON-RPC response line:

```bash
export OPENAI_API_KEY=sk-demo-must-not-appear
META='"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}'
printf '%s\n' \
  "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{$META}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{$META}}" \
  "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"lookup\",\"arguments\":{\"customerId\":\"customer-42\"},$META}}" \
  | tysel mcp
```

The managed installation includes the matching `tysel-worker`; no separate
worker build or environment variable is required.

The tool result contains `customerId`, `isolated: true`, and the opaque handle
`secret:OPENAI_API_KEY`; it must not contain `sk-demo-must-not-appear`.

Input validation is derived from `input: { customerId: "string" }`. Missing,
mistyped, or additional arguments produce an MCP tool error. An unknown tool
produces JSON-RPC error `-32602` without invoking application code.

## Maintainer acceptance (source checkout only)

```bash
cargo test -p tysel-cli --test examples mcp_tool_covers_stdio_contract_and_opaque_secrets
```
