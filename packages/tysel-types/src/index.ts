/** The public, declaration-only contract exposed by the Tysel native runtime. */

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonObject | JsonValue[];
export interface JsonObject {
  readonly [key: string]: JsonValue;
}

export type MaybePromise<T> = T | Promise<T>;
export type ExecutionProfile = "service" | "isolated" | "component";
export type TrustMode = "trusted-service" | "isolated-task";

export interface CapabilityRequirement {
  readonly id: string;
  readonly resources: readonly string[];
}

export interface RequestContext {
  readonly requestId: string;
  readonly deadlineMs: number;
}

/** Handles one HTTP request without requiring an injected runtime. */
export type FetchHandler = (request: Request) => MaybePromise<Response>;

/** Handles one HTTP request with the runtime capability host. */
export type RuntimeFetchHandler<Runtime = TyselRuntime> = (
  request: Request,
  runtime: Runtime,
) => MaybePromise<Response>;

export interface CronTask {
  readonly kind: "cron";
  readonly expression: string;
  readonly handler: (context: RequestContext) => MaybePromise<void>;
}

export interface QueueTask<Message = JsonValue> {
  readonly kind: "queue";
  readonly name: string;
  readonly handler: (
    message: Message,
    context: RequestContext,
  ) => MaybePromise<unknown>;
}

export type McpInputType = "string" | "number" | "integer" | "boolean" | "object" | "array";
export type McpInputSchema = Readonly<Record<string, McpInputType>>;

type McpInputValue<Type extends McpInputType> = Type extends "string"
  ? string
  : Type extends "number" | "integer"
    ? number
    : Type extends "boolean"
      ? boolean
      : Type extends "object"
        ? JsonObject
        : JsonValue[];

/** Infers the handler input accepted by one literal MCP schema. */
export type InferMcpInput<Schema extends McpInputSchema> = {
  -readonly [Key in keyof Schema]: McpInputValue<Schema[Key]>;
};

type McpTaskSchema<InputOrSchema extends object> = InputOrSchema extends McpInputSchema
  ? InputOrSchema
  : McpInputSchema;

type McpTaskInput<InputOrSchema extends object> = InputOrSchema extends McpInputSchema
  ? InferMcpInput<InputOrSchema>
  : InputOrSchema;

export interface McpTask<
  InputOrSchema extends object = JsonObject,
  Output = JsonValue,
> {
  readonly kind: "mcp";
  readonly description: string;
  readonly input: McpTaskSchema<InputOrSchema>;
  readonly handler: (
    input: McpTaskInput<InputOrSchema>,
    context: RequestContext,
  ) => MaybePromise<Output>;
}

/** Registry-level MCP shape: validates the schema while leaving precise inference to `defineApp` or `mcp`. */
interface RegisteredMcpTask {
  readonly kind: "mcp";
  readonly description: string;
  readonly input: McpInputSchema;
  readonly handler: (
    input: never,
    context: RequestContext,
  ) => MaybePromise<unknown>;
}

export type AppTask = CronTask | QueueTask | McpTask;

export type DurableHandler<
  Input = JsonValue,
  Output = JsonValue,
> = (context: DurableContext, input: Input) => MaybePromise<Output>;

interface TyselAppMembers<Runtime> {
  readonly fetch: RuntimeFetchHandler<Runtime>;
  readonly tasks: Readonly<Record<string, CronTask | QueueTask<never> | RegisteredMcpTask>>;
  readonly durable: Readonly<Record<string, DurableHandler<never, unknown>>>;
}

type RequireAtLeastOne<T> = {
  [Key in keyof T]-?: Required<Pick<T, Key>> & Partial<Omit<T, Key>>;
}[keyof T];

/** An application declares at least one HTTP, task-registry, or durable-registry entrypoint group. */
export type TyselApp<Runtime = TyselRuntime> = RequireAtLeastOne<
  TyselAppMembers<Runtime>
>;

export type DurableDuration = number | string;

export interface DurableRetryPolicy {
  readonly maxAttempts?: number;
  readonly delay?: DurableDuration;
  readonly factor?: number;
  readonly maxDelay?: DurableDuration;
}

export interface DurableContext {
  step<T>(name: string, operation: () => MaybePromise<T>): Promise<T>;
  effect<T>(name: string, operation: () => MaybePromise<T>): Promise<T>;
  sleep(duration: DurableDuration): Promise<void>;
  waitForSignal<T = JsonValue>(name: string): Promise<T>;
  retry<T>(
    policy: DurableRetryPolicy,
    operation: (attempt: number) => MaybePromise<T>,
  ): Promise<T>;
  now(): Date;
  random(): number;
}

/** @deprecated Use `DurableContext`. */
export type DurableHost = DurableContext;

declare const secretReferenceBrand: unique symbol;
export type SecretReference = string & {
  readonly [secretReferenceBrand]: "TyselSecretReference";
};

export type SqlParameter = JsonPrimitive;
export type SqlRow = Readonly<Record<string, JsonValue>>;

export interface SqlClient {
  exec(sql: string, params?: readonly SqlParameter[]): Promise<number>;
  query<Row = SqlRow>(
    sql: string,
    params?: readonly SqlParameter[],
  ): Promise<Row[]>;
}

export interface FileSystemClient {
  read(path: string): Promise<string>;
  write(path: string, data: string): Promise<void>;
}

export interface SecretClient<Name extends string = string> {
  ref(name: Name): Promise<SecretReference>;
}

export interface LlmGenerateOptions<Input = JsonValue> {
  readonly model: string;
  readonly input: Input;
  readonly system?: string;
  readonly maxOutputTokens?: number;
  readonly temperature?: number;
}

export interface LlmUsage {
  readonly input_tokens: number;
  readonly output_tokens: number;
}

export interface LlmResponse<Output = JsonValue> {
  readonly output: Output;
  readonly usage: LlmUsage;
  readonly provider_request_id?: string;
}

export interface LlmClient {
  generate<Output = JsonValue, Input = JsonValue>(
    options: LlmGenerateOptions<Input>,
  ): Promise<LlmResponse<Output>>;
}

export interface DurableSuspendedResult {
  readonly status: "suspended";
  readonly taskId: string;
}

export interface DurableCompletedResult<Output = JsonValue> {
  readonly status: "completed";
  readonly taskId: string;
  readonly value: Output;
}

export type DurableStartResult<Output = JsonValue> =
  | DurableSuspendedResult
  | DurableCompletedResult<Output>;

export interface DurableControlClient {
  start<Output = JsonValue, Input = JsonValue>(
    name: string,
    input?: Input,
  ): DurableStartResult<Output>;
  sendSignal<Payload = JsonValue>(
    taskId: string,
    name: string,
    payload?: Payload,
  ): void;
}

export interface AcceptedWebSocketEvent {
  readonly type: AcceptedWebSocketEventType;
  readonly target: AcceptedWebSocket;
  readonly currentTarget: AcceptedWebSocket;
  readonly data?: string;
  readonly code?: number;
  readonly reason?: string;
  readonly wasClean?: boolean;
  readonly error?: unknown;
}

export type AcceptedWebSocketEventType = "message" | "close" | "error";
export type AcceptedWebSocketListener =
  | ((event: AcceptedWebSocketEvent) => void)
  | { handleEvent(event: AcceptedWebSocketEvent): void };

export interface AcceptedWebSocket {
  readonly readyState: number;
  onmessage: ((event: AcceptedWebSocketEvent) => void) | null;
  onclose: ((event: AcceptedWebSocketEvent) => void) | null;
  onerror: ((event: AcceptedWebSocketEvent) => void) | null;
  send(data: string): Promise<void>;
  close(): Promise<void>;
  addEventListener(
    type: AcceptedWebSocketEventType,
    listener: AcceptedWebSocketListener,
    options?: { once?: boolean },
  ): void;
  removeEventListener(
    type: AcceptedWebSocketEventType,
    listener: AcceptedWebSocketListener,
  ): void;
}

export interface TyselRuntime {
  readonly isolateId: number;
  sleep(milliseconds: number): Promise<void>;
  echo(value: string): Promise<string>;
  httpGet(url: string): Promise<Response>;
  acceptWebSocket(): AcceptedWebSocket;
  readonly sqlite: SqlClient;
  readonly postgres: SqlClient;
  readonly fs: FileSystemClient;
  readonly secrets: SecretClient;
  readonly llm: LlmClient;
  readonly durable: DurableControlClient;
}

/** Selects the runtime members exposed to an application handler. */
export type TyselRuntimeWith<Capability extends keyof TyselRuntime> = Pick<
  TyselRuntime,
  "isolateId" | "sleep" | "echo" | Capability
>;

/** Conventional application-facing name for the public runtime host. */
export type Tysel = TyselRuntime;

declare global {
  const tysel: TyselRuntime;

  interface WebSocket {
    /** Resolves when the Tysel WebSocket client opens; rejects on connection failure. */
    readonly opened: Promise<WebSocket>;
  }
}
