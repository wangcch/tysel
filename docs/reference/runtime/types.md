# Core runtime types

This page indexes shared declarations exported by `@tysel/types`. Import them
with `import type`; the package is declaration-only.

## JSON values

```ts
type JsonPrimitive = string | number | boolean | null;
type JsonValue = JsonPrimitive | JsonObject | JsonValue[];

interface JsonObject {
  readonly [key: string]: JsonValue;
}

type MaybePromise<T> = T | Promise<T>;
```

Host protocols serialize JSON values. `undefined`, functions, symbols,
cyclic structures, and class instances are not portable protocol values.

## Execution and trust

```ts
type ExecutionProfile = "service" | "isolated" | "component";
type TrustMode = "trusted-service" | "isolated-task";

interface CapabilityRequirement {
  readonly id: string;
  readonly resources: readonly string[];
}
```

`ExecutionProfile` matches `app.profile`. `TrustMode` describes the effective
JavaScript trust boundary used by host integrations. A
`CapabilityRequirement` identifies a capability and its resource selectors;
it describes required authority but does not grant it.

See [Execution profiles](../../concepts/execution-profiles.md) and the
[Capability matrix](../../capabilities/README.md).

## Request context

```ts
interface RequestContext {
  readonly requestId: string;
  readonly deadlineMs: number;
}
```

Queue, cron, and MCP handlers receive this context. `deadlineMs` is an absolute
millisecond deadline. HTTP handlers receive only the `Request`.

## SQL values

```ts
type SqlParameter = JsonPrimitive;
type SqlRow = Readonly<Record<string, JsonValue>>;
```

`SqlClient.query<Row>()` can supply a more specific row type when the
application knows the selected columns. This generic is a static assertion,
not runtime row validation.

## Secret references

```ts
declare const secretReferenceBrand: unique symbol;
type SecretReference = string & {
  readonly [secretReferenceBrand]: "TyselSecretReference";
};
```

The private brand prevents ordinary strings from being mistaken for an opaque
host secret reference. Applications cannot import `secretReferenceBrand` or
read the secret value.

## LLM values

```ts
interface LlmGenerateOptions<Input = JsonValue> {
  readonly model: string;
  readonly input: Input;
  readonly system?: string;
  readonly maxOutputTokens?: number;
  readonly temperature?: number;
}

interface LlmUsage {
  readonly input_tokens: number;
  readonly output_tokens: number;
}

interface LlmResponse<Output = JsonValue> {
  readonly output: Output;
  readonly usage: LlmUsage;
  readonly provider_request_id?: string;
}
```

Generic `Input` and `Output` types improve application typing; the protocol
still requires values accepted by the configured provider and bounded host
adapter. See [Host capabilities](capabilities.md#llm-generation).

## Deprecated alias

`DurableHost` is retained as a deprecated alias of `DurableContext`. New code
should import `DurableContext` directly from `@tysel/types`.

## Complete export inventory

This inventory names every declaration exported by `@tysel/types`. The linked
contract page carries the behavioral detail; the inventory is also compared
with the package source in documentation CI so a new public declaration cannot
remain undiscoverable.

| Area | Exported declarations | Contract |
| --- | --- | --- |
| JSON | `JsonPrimitive`, `JsonValue`, `JsonObject`, `MaybePromise` | [JSON values](#json-values) |
| Execution | `ExecutionProfile`, `TrustMode`, `CapabilityRequirement`, `RequestContext` | [Execution and trust](#execution-and-trust) |
| Application | `FetchHandler`, `CronTask`, `QueueTask`, `McpInputSchema`, `McpTask`, `AppTask`, `TyselApp` | [Application module](application.md) |
| Durable handler | `DurableHandler`, `DurableDuration`, `DurableRetryPolicy`, `DurableContext`, `DurableHost` | [Durable API](durable.md) |
| Durable control | `DurableSuspendedResult`, `DurableCompletedResult`, `DurableStartResult`, `DurableControlClient` | [Durable control](durable.md#control-api) |
| SQL and files | `SqlParameter`, `SqlRow`, `SqlClient`, `FileSystemClient` | [Host capabilities](capabilities.md) |
| Secrets | `SecretReference`, `SecretClient` | [Secrets](capabilities.md#secrets) |
| LLM | `LlmGenerateOptions`, `LlmUsage`, `LlmResponse`, `LlmClient` | [LLM generation](capabilities.md#llm-generation) |
| Accepted WebSocket | `AcceptedWebSocketEvent`, `AcceptedWebSocketEventType`, `AcceptedWebSocketListener`, `AcceptedWebSocket` | [WebSockets](capabilities.md#websockets) |
| Runtime host | `TyselRuntime`, `Tysel` | [Runtime overview](index.md) |
