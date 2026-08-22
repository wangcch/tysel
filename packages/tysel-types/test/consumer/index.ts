import type {} from "@tysel/types";
import type { DurableContext, RequestContext, TyselApp } from "@tysel/types";
import { invokeFetch } from "@tysel/test";

interface UserRow {
  id: number;
  name: string;
}

interface AgentInput {
  prompt: string;
}

const app = {
  fetch: async (request) => new Response(request.method),
  durable: {
    agent: async (context: DurableContext, input: AgentInput) =>
      context.step("answer", () => ({ answer: input.prompt })),
  },
} satisfies TyselApp;

const invalidFetchContext = {
  // @ts-expect-error the native fetch runtime supplies only Request
  fetch: async (_request: Request, context: RequestContext) =>
    new Response(context.requestId),
} satisfies TyselApp;

void app;
void invalidFetchContext;
const response = await invokeFetch(app.fetch, "https://example.test");
await response.text();
const rows = await tysel.sqlite.query<UserRow>("SELECT id, name FROM users");
rows[0]?.name.toUpperCase();
const generated = await tysel.llm.generate<{ answer: string }>({
  model: "default",
  input: { prompt: "hello" },
});
generated.output.answer.toUpperCase();
const socket = new WebSocket("wss://example.test");
await socket.opened;

// @ts-expect-error underscored native bindings are intentionally private
tysel._sqliteExec("SELECT 1", "[]");
// @ts-expect-error model is required by the public LLM contract
await tysel.llm.generate({ input: "hello" });
