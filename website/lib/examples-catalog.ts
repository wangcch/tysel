import { githubUrl } from "./shared";

export type ExampleStatus = "runnable" | "experimental" | "recipe";

export type Example = {
  id: string;
  name: string;
  /** One sentence: what this path is for. */
  purpose: string;
  status: ExampleStatus;
  profile: string;
  /** Named grants or explicit "none". */
  grants: string;
  /** Primary verification command from the README. */
  run: string;
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
  grants: "LLM · secret · SQLite · durable store",
  run: "./demo.sh",
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
  examples: Example[];
};

export const exampleSections: ExampleSection[] = [
  {
    id: "services",
    title: "Services",
    intent: "Trusted first-party HTTP apps. Grants stay closed until declared.",
    examples: [
      {
        id: "hello-service",
        name: "Hello service",
        purpose: "Minimal Fetch handler and one-executable build.",
        status: "runnable",
        profile: "service",
        grants: "none",
        run: "tysel run",
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
        grants: "none",
        run: "npm install && tysel run",
        sourceHref: tree("examples/hono-api"),
        docsHref: "/docs/compatibility",
        contractHref: "/reference/javascript",
      },
      {
        id: "sqlite-worker",
        name: "SQLite worker",
        purpose: "Persistent counter through a runtime-owned, grant-bound client.",
        status: "runnable",
        profile: "service",
        grants: "SQLite",
        run: "tysel run",
        sourceHref: tree("examples/sqlite-worker"),
        contractHref: "/reference/runtime/capabilities#sql",
      },
      {
        id: "postgres-service",
        name: "Postgres service",
        purpose: "Named database grant. The URL stays in the host environment.",
        status: "runnable",
        profile: "service",
        grants: "main:read-write",
        run: "tysel run",
        sourceHref: tree("examples/postgres-service"),
        contractHref: "/reference/runtime/capabilities#sql",
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
        id: "mcp-tool",
        name: "MCP tool",
        purpose:
          "Validated JSON tool over bounded stdio. Returns an opaque secret handle only.",
        status: "runnable",
        profile: "isolated",
        grants: "brokered secret only",
        run: "tysel mcp",
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
        grants: "host capabilities denied",
        run: "tysel run",
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
        grants: "deny by default",
        run: "printf '{\"value\":42}' | tysel run",
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
        grants: "deny by default",
        run: "printf '{\"value\":42}' | tysel run",
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
    name: "WebSocket",
    purpose: "Inbound upgrade or allowlisted outbound client.",
    href: "/reference/runtime/capabilities#websockets",
  },
  {
    name: "Filesystem",
    purpose: "UTF-8 read/write under manifest-pinned roots only.",
    href: "/reference/runtime/capabilities#filesystem",
  },
  {
    name: "Cron and queue",
    purpose: "Typed scheduled and queued handlers.",
    href: "/reference/runtime/application#cron-tasks",
  },
  {
    name: "Handler tests",
    purpose: "Invoke Fetch handlers in a fresh isolate.",
    href: "/reference/runtime/testing",
  },
];
