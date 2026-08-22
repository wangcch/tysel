import type {
  CronTask,
  DurableHandler,
  McpTask,
  QueueTask,
  TyselApp,
} from "@tysel/types";

export type {
  AppTask,
  CronTask,
  DurableContext,
  DurableHandler,
  DurableRetryPolicy,
  FetchHandler,
  JsonObject,
  JsonValue,
  McpTask,
  QueueTask,
  RequestContext,
  TyselApp,
} from "@tysel/types";

/** @deprecated Use `TyselApp`; retained for source compatibility. */
export type AppDefinition = TyselApp;

export function defineApp<T extends TyselApp>(app: T): T {
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

export function mcp<Input extends object, Output>(
  spec: { description: string; input: Readonly<Record<string, string>> },
  handler: McpTask<Input, Output>["handler"],
): McpTask<Input, Output> {
  return {
    kind: "mcp",
    description: spec.description,
    input: spec.input,
    handler,
  };
}

export function durableTask<
  Input,
  Output,
>(run: DurableHandler<Input, Output>): DurableHandler<Input, Output> {
  return run;
}
