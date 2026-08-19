export interface RequestContext {
  readonly requestId: string;
  readonly deadlineMs: number;
}

export type FetchHandler = (
  request: Request,
  ctx: RequestContext,
) => Response | Promise<Response>;

export interface CronTask {
  readonly kind: "cron";
  readonly expression: string;
  readonly handler: (ctx: RequestContext) => Promise<void>;
}

export interface QueueTask<T = unknown> {
  readonly kind: "queue";
  readonly name: string;
  readonly handler: (message: T, ctx: RequestContext) => Promise<void>;
}

export interface McpTask<I = Record<string, unknown>, O = unknown> {
  readonly kind: "mcp";
  readonly description: string;
  readonly input: Record<string, string>;
  readonly handler: (input: I, ctx: RequestContext) => Promise<O>;
}

export type AppTask = CronTask | QueueTask | McpTask;

export interface AppDefinition {
  fetch?: FetchHandler;
  tasks?: Record<string, AppTask>;
}

export interface DurableContext {
  step<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  effect<T>(name: string, fn: () => Promise<T> | T): Promise<T>;
  sleep(duration: string): Promise<void>;
  waitForSignal<T = unknown>(name: string): Promise<T>;
  now(): Date;
  random(): number;
}

export function defineApp<T extends AppDefinition>(app: T): T {
  return app;
}

export function cron(
  expression: string,
  handler: CronTask["handler"],
): CronTask {
  return { kind: "cron", expression, handler };
}

export function queue<T>(
  name: string,
  handler: QueueTask<T>["handler"],
): QueueTask<T> {
  return { kind: "queue", name, handler };
}

export function mcp<I extends Record<string, unknown>, O>(
  spec: { description: string; input: Record<string, string> },
  handler: McpTask<I, O>["handler"],
): McpTask<I, O> {
  return {
    kind: "mcp",
    description: spec.description,
    input: spec.input,
    handler,
  };
}

export function durableTask<I, O>(
  run: (ctx: DurableContext, input: I) => Promise<O>,
): (ctx: DurableContext, input: I) => Promise<O> {
  return run;
}
