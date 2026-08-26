import type {
  DurableContext,
  FetchHandler,
  FileSystemClient,
  McpInputSchema,
  RequestContext,
  SecretClient,
  SqlClient,
  TyselApp,
  TyselRuntimeWith,
} from "@tysel/types";
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
  // @ts-expect-error the second argument is the Tysel runtime, not task metadata
  fetch: async (_request: Request, context: RequestContext) =>
    new Response(context.requestId),
} satisfies TyselApp;

const misspelledFetch = {
  // @ts-expect-error application entrypoint names are checked
  fecth: async (_request: Request) => new Response("typo"),
} satisfies TyselApp;

const invalidFetchResult = {
  // @ts-expect-error fetch handlers must return a Web Response
  fetch: async () => ({ status: 200 }),
} satisfies TyselApp;

// @ts-expect-error an application must expose at least one entrypoint
const emptyApp = {} satisfies TyselApp;

const invalidMcpInput: McpInputSchema = {
  // @ts-expect-error MCP schema values use the documented protocol vocabulary
  query: "text",
};

const invalidMcpApp = {
  tasks: {
    lookup: {
      kind: "mcp",
      description: "invalid schema",
      input: {
        // @ts-expect-error TyselApp validates MCP schema vocabulary too
        query: "text",
      },
      async handler(input) {
        return input;
      },
    },
  },
} satisfies TyselApp;

const rawMcpApp = {
  tasks: {
    lookup: {
      kind: "mcp",
      description: "raw registry validation",
      input: { query: "string" },
      async handler(input) {
        // @ts-expect-error raw TyselApp does not expose an unvalidated MCP input
        input.query;
        return null;
      },
    },
  },
} satisfies TyselApp;

const narrowRuntimeApp = {
  fetch: async (_request, runtime) => {
    runtime.isolateId.toFixed();
    // @ts-expect-error filesystem access was not selected for this handler
    await runtime.fs.read("input.txt");
    return new Response("ok");
  },
} satisfies TyselApp<TyselRuntimeWith<never>>;

type GeneratedEnv = Omit<
  TyselRuntimeWith<"acceptWebSocket" | "durable" | "sqlite">,
  "fs" | "postgres" | "secrets"
> & {
  readonly fs: Pick<FileSystemClient, "read">;
  readonly postgres: Pick<SqlClient, "query">;
  readonly secrets: SecretClient<"API_TOKEN">;
};

const generatedEnvApp = {
  fetch: async (_request, runtime) => {
    await runtime.fs.read("input.txt");
    await runtime.postgres.query("SELECT 1");
    await runtime.secrets.ref("API_TOKEN");
    // @ts-expect-error the manifest did not grant filesystem writes
    await runtime.fs.write("output.txt", "data");
    // @ts-expect-error undeclared secret names are rejected
    await runtime.secrets.ref("OTHER_TOKEN");
    return new Response("ok");
  },
} satisfies TyselApp<GeneratedEnv>;

void app;
void invalidFetchContext;
void misspelledFetch;
void invalidFetchResult;
void emptyApp;
void invalidMcpInput;
void invalidMcpApp;
void rawMcpApp;
void narrowRuntimeApp;
void generatedEnvApp;
const response = await invokeFetch(app.fetch, "https://example.test");
await response.text();
declare const legacyFetch: FetchHandler;
await invokeFetch(legacyFetch, "https://example.test");
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
