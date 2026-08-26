import type {
  CronTask,
  DurableHandler,
  McpInputSchema,
  McpTask,
  QueueTask,
  RuntimeFetchHandler,
  TyselApp,
  TyselRuntime,
} from "@tysel/types";

export type {
  AppTask,
  CronTask,
  DurableContext,
  DurableHandler,
  DurableRetryPolicy,
  FetchHandler,
  InferMcpInput,
  JsonObject,
  JsonValue,
  McpInputSchema,
  McpInputType,
  McpTask,
  QueueTask,
  RequestContext,
  RuntimeFetchHandler,
  SecretClient,
  TyselApp,
  TyselRuntime,
  TyselRuntimeWith,
} from "@tysel/types";

/** @deprecated Use `TyselApp`; retained for source compatibility. */
export type AppDefinition = TyselApp;

type McpApplication<Runtime, Schemas extends Readonly<Record<string, McpInputSchema>>> = {
  readonly fetch?: RuntimeFetchHandler<Runtime>;
  readonly tasks: {
    readonly [Name in keyof Schemas]: McpTask<Schemas[Name], unknown>;
  };
  readonly durable?: Readonly<Record<string, DurableHandler<never, unknown>>>;
};

type NonMcpApplication<Runtime> = {
  readonly fetch?: RuntimeFetchHandler<Runtime>;
  readonly tasks: Readonly<Record<string, CronTask | QueueTask<never>>>;
  readonly durable?: Readonly<Record<string, DurableHandler<never, unknown>>>;
};

type ApplicationWithoutTasks<Runtime> =
  | {
      readonly fetch: RuntimeFetchHandler<Runtime>;
      readonly tasks?: never;
      readonly durable?: Readonly<Record<string, DurableHandler<never, unknown>>>;
    }
  | {
      readonly fetch?: RuntimeFetchHandler<Runtime>;
      readonly tasks?: never;
      readonly durable: Readonly<Record<string, DurableHandler<never, unknown>>>;
    };

declare const inferredMcpTask: unique symbol;

type InferredMcpTask<Schema extends McpInputSchema, Output> = McpTask<Schema, Output> & {
  readonly [inferredMcpTask]: true;
};

type DefineAppFallback<Runtime> = TyselApp<Runtime> & {
  readonly tasks?: Readonly<
    Record<string, CronTask | QueueTask<never> | InferredMcpTask<any, unknown>>
  >;
};

interface DefineApp<Runtime> {
  <
    const Schemas extends Readonly<Record<string, McpInputSchema>>,
    const App,
  >(app: McpApplication<Runtime, Schemas> & App): App;
  <const App extends NonMcpApplication<Runtime>>(app: App): App;
  <const App extends ApplicationWithoutTasks<Runtime>>(app: App): App;
  <const App extends DefineAppFallback<Runtime>>(app: App): App;
}

export function defineApp<Runtime>(): DefineApp<Runtime>;
export function defineApp<
  const Schemas extends Readonly<Record<string, McpInputSchema>>,
  const App,
>(
  app: McpApplication<TyselRuntime, Schemas> & App,
): App;
export function defineApp<const App extends NonMcpApplication<TyselRuntime>>(app: App): App;
export function defineApp<const App extends ApplicationWithoutTasks<TyselRuntime>>(app: App): App;
export function defineApp<const App extends DefineAppFallback<TyselRuntime>>(app: App): App;
export function defineApp(app?: unknown): unknown {
  if (app === undefined) {
    return <const App extends TyselApp>(definition: App): App => definition;
  }
  return app;
}

export function cron(
  expression: string,
  handler: CronTask["handler"],
): CronTask {
  return { kind: "cron", expression, handler };
}

export function queue<Message>(
  name: string,
  handler: QueueTask<Message>["handler"],
): QueueTask<Message> {
  return { kind: "queue", name, handler };
}

export function mcp<const Schema extends McpInputSchema, Output>(
  definition: {
    readonly description: string;
    readonly input: Schema;
    readonly handler: McpTask<Schema, Output>["handler"];
  },
): InferredMcpTask<Schema, Output> {
  return {
    kind: "mcp",
    description: definition.description,
    input: definition.input,
    handler: definition.handler,
  } as InferredMcpTask<Schema, Output>;
}

export function durableTask<
  Input,
  Output,
>(run: DurableHandler<Input, Output>): DurableHandler<Input, Output> {
  return run;
}
