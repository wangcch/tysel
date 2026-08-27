import { githubUrl } from "./shared";

export type ExampleStatus = "runnable" | "experimental" | "recipe";

export type Example = {
  id: string;
  name: string;
  /** One sentence: what this path is for. */
  purpose: string;
  status: ExampleStatus;
  profile: string;
  /** Capabilities exercised by this example, excluding unused profile defaults. */
  capabilities: string;
  /** Primary verification command from the README. */
  run: string;
  /** Context shown above the command block. */
  runLabel?: string;
  sourceHref: string;
  sourceLabel?: string;
  docsHref?: string;
  contractHref: string;
};

function tree(path: string) {
  return `${githubUrl}/tree/main/${path}`;
}

/** Featured path — the Tysel-specific story, not a Hello World. */
export const featuredExample: Example = {
  id: "durable-agent",
  name: "Durable agent",
  purpose:
    "Start an LLM draft, suspend for human approval, restart the process, resume, and save the result once.",
  status: "runnable",
  profile: "service",
  capabilities: "LLM · secret · SQLite · durable execution",
  run: "tysel task verify\n./demo.sh",
  sourceHref: tree("examples/durable-agent"),
  docsHref: "/docs/concepts/durable-execution",
  contractHref: "/reference/runtime/durable",
};

export const featuredSnippet = {
  filename: "src/index.ts",
  code: `const agent = async (ctx: DurableContext, input: AgentInput) => {
  const draft = await ctx.effect("draft-with-llm", () =>
    tysel.llm.generate({
      model: "default",
      input: input.prompt ?? \`Summarize account \${input.customerId}\`,
    }),
  );
  const approval = await ctx.waitForSignal<Approval>("approval");
  await ctx.effect("save-result", () => persist(input.runId, draft, approval));
  return { draft, approved: approval.approved === true };
};`,
};

export type ExampleSection = {
  id: string;
  title: string;
  /** Why this group exists — one line. */
  intent: string;
  /** Effective profile behavior shared by entries in this group. */
  note?: string;
  examples: Example[];
};

export const exampleSections: ExampleSection[] = [
  {
    id: "services",
    title: "Services",
    intent:
      "Trusted first-party HTTP apps. Labels name the behavior each example exercises.",
    note:
      'The service profile currently defaults to durable.store = "sqlite" and ./data/tysel.db, even when an example does not call SQLite. Run tysel inspect to review the complete effective configuration.',
    examples: [
      {
        id: "hello-service",
        name: "Hello service",
        purpose: "Minimal Fetch handler and one-executable build.",
        status: "runnable",
        profile: "service",
        capabilities: "HTTP · Fetch",
        run: "tysel task verify\ntysel dev",
        sourceHref: tree("examples/hello-service"),
        docsHref: "/docs/getting-started",
        contractHref: "/reference/runtime/application#http-handler",
      },
      {
        id: "hono-api",
        name: "Hono API",
        purpose: "Web-API-first npm router. Compatibility is scanned, not assumed.",
        status: "runnable",
        profile: "service",
        capabilities: "HTTP · Hono",
        run: "npm install\ntysel task verify\ntysel run",
        sourceHref: tree("examples/hono-api"),
        docsHref: "/docs/compatibility",
        contractHref: "/reference/javascript",
      },
      {
        id: "websocket-service",
        name: "WebSocket service",
        purpose: "HTTP/1.1 upgrade and bounded text echo.",
        status: "runnable",
        profile: "service",
        capabilities: "HTTP · inbound WebSocket",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/websocket-service"),
        docsHref: "/docs/guides/service-networking",
        contractHref: "/reference/javascript/websocket",
      },
      {
        id: "sqlite-worker",
        name: "SQLite worker",
        purpose: "Persistent counter through a runtime-owned, grant-bound client.",
        status: "runnable",
        profile: "service",
        capabilities: "SQLite",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/sqlite-worker"),
        docsHref: "/docs/guides/sqlite",
        contractHref: "/reference/runtime/capabilities#sql",
      },
      {
        id: "postgres-service",
        name: "Postgres service",
        purpose: "Named database grant. The URL stays in the host environment.",
        status: "runnable",
        profile: "service",
        capabilities: "PostgreSQL · main:read-write",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/postgres-service"),
        docsHref: "/docs/guides/postgresql",
        contractHref: "/reference/runtime/capabilities#sql",
      },
      {
        id: "redis-service",
        name: "Redis service",
        purpose: "Bounded cache operations through a named, host-injected Redis grant.",
        status: "runnable",
        profile: "service",
        capabilities: "Redis · cache:read-write",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/redis-service"),
        docsHref: "/docs/guides/redis",
        contractHref: "/reference/runtime/capabilities#redis",
      },
      {
        id: "filesystem-transform",
        name: "Filesystem transform",
        purpose: "Read and write UTF-8 files beneath separate pinned roots.",
        status: "runnable",
        profile: "service",
        capabilities: "filesystem read · filesystem write",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/filesystem-transform"),
        docsHref: "/docs/guides/filesystem",
        contractHref: "/reference/runtime/capabilities#filesystem",
      },
    ],
  },
  {
    id: "tasks",
    title: "Cron and Queue",
    intent: "Scheduled and produced JSON work on the bounded task path.",
    examples: [
      {
        id: "task-worker",
        name: "Task worker",
        purpose: "UTC Cron registration and one named Queue consumer.",
        status: "runnable",
        profile: "service",
        capabilities: "Cron · Queue",
        run:
          "tysel task verify\ntysel queue jobs --input '{\"id\":\"job_123\",\"action\":\"reindex\"}'",
        sourceHref: tree("examples/task-worker"),
        docsHref: "/docs/guides/cron-queue",
        contractHref: "/reference/runtime/application#cron-tasks",
      },
    ],
  },
  {
    id: "agents",
    title: "Agents and tools",
    intent: "Work that must survive restarts, or run as an isolated MCP tool.",
    examples: [
      featuredExample,
      {
        id: "llm-service",
        name: "LLM service",
        purpose: "Host-configured provider alias with credential isolation and usage metadata.",
        status: "runnable",
        profile: "service",
        capabilities: "LLM · provider secret",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/llm-service"),
        docsHref: "/docs/guides/llm-gateway",
        contractHref: "/reference/runtime/capabilities#llm-generation",
      },
      {
        id: "mcp-tool",
        name: "MCP tool",
        purpose:
          "Validated JSON tool over bounded stdio. Returns an opaque secret handle only.",
        status: "runnable",
        profile: "isolated",
        capabilities: "MCP · brokered secret",
        run: "tysel task verify\ntysel mcp",
        sourceHref: tree("examples/mcp-tool"),
        contractHref: "/reference/runtime/application#mcp-tasks",
      },
    ],
  },
  {
    id: "isolation",
    title: "Isolation and Wasm",
    intent:
      "Separate worker process, or a language-neutral one-shot Component task.",
    examples: [
      {
        id: "isolated-plugin",
        name: "Isolated plugin",
        purpose:
          "Fetch handler under the isolated profile. Manifest grants are still denied.",
        status: "runnable",
        profile: "isolated",
        capabilities: "host capabilities denied",
        run: "tysel task verify\ntysel run",
        sourceHref: tree("examples/isolated-plugin"),
        docsHref: "/docs/concepts/execution-profiles",
        contractHref: "/docs/capabilities",
      },
      {
        id: "rust-component",
        name: "Rust Component",
        purpose: "One-shot JSON task via tysel:component/task@0.4.0.",
        status: "experimental",
        profile: "component",
        capabilities: "JSON task · no ambient WASI",
        run: "printf '{\"value\":42}' | tysel run",
        runLabel: "after building the Rust starter",
        sourceHref: "/docs/guides/wasm-component-rust",
        sourceLabel: "Starter guide",
        contractHref: "/reference/component",
      },
      {
        id: "go-component",
        name: "Go Component",
        purpose: "Same Component world with committed Go bindings.",
        status: "experimental",
        profile: "component",
        capabilities: "JSON task · no ambient WASI",
        run: "printf '{\"value\":42}' | tysel run",
        runLabel: "after building the Go starter",
        sourceHref: "/docs/guides/wasm-component-go",
        sourceLabel: "Starter guide",
        contractHref: "/reference/component/go-sdk",
      },
    ],
  },
];

/** Supported APIs without a dedicated acceptance tree. */
export const recipes: Array<{
  name: string;
  purpose: string;
  href: string;
}> = [
  {
    name: "Handler tests",
    purpose: "Invoke Fetch handlers in a fresh isolate.",
    href: "/reference/runtime/testing",
  },
];
