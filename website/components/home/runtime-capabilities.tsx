"use client";

import Link from "next/link";
import type { KeyboardEvent, ReactNode } from "react";
import { useId, useRef, useState } from "react";
import { CopyButton } from "@/components/copy-button";

type Example = {
  id: string;
  label: string;
  blurb: string;
  filename: string;
  code: string;
  /** Manifest / profile authority shown under the snippet. */
  grant: string;
  docsHref: string;
  exampleHref?: string;
  exampleLabel?: string;
};

type ApiItem = {
  name: string;
  description: string;
  href: string;
};

type ApiGroup = {
  title: string;
  items: ApiItem[];
};

/**
 * Curated “simple cases” — services, agents, and Wasm — not a Bun-style
 * stdlib tour. Each row names the grant or profile it actually needs.
 */
const examples: Example[] = [
  {
    id: "http",
    label: "HTTP service",
    blurb:
      "A Fetch handler is enough for a first-party service. No host grant until you call one.",
    filename: "src/index.ts",
    code: `import type { TyselApp } from "@tysel/types";

export default {
  async fetch(request, runtime) {
    const url = new URL(request.url);

    return Response.json({
      method: request.method,
      path: url.pathname,
      isolateId: runtime.isolateId,
    });
  },
} satisfies TyselApp;`,
    grant: "service · no host capability required",
    docsHref: "/reference/runtime/application#http-handler",
    exampleHref:
      "https://github.com/wangcch/tysel/tree/main/examples/hello-service",
    exampleLabel: "hello-service",
  },
  {
    id: "durable",
    label: "Durable agent",
    blurb:
      "LLM call, human approval, then resume after restart — completed effects replay instead of re-running.",
    filename: "src/agent.ts",
    code: `import type { DurableContext, TyselApp } from "@tysel/types";

const agent = async (ctx: DurableContext, input: Input) => {
  const draft = await ctx.effect("draft", () =>
    tysel.llm.generate({
      model: "default",
      input: input.prompt,
    }),
  );

  const approval = await ctx.waitForSignal("approval");
  return { draft, approval };
};

export default { durable: { agent } } satisfies TyselApp;`,
    grant: "service · durable store + LLM + declared secret",
    docsHref: "/reference/runtime/durable",
    exampleHref:
      "https://github.com/wangcch/tysel/tree/main/examples/durable-agent",
    exampleLabel: "durable-agent",
  },
  {
    id: "llm",
    label: "LLM call",
    blurb:
      "Generation goes through the host. Model aliases and credentials stay outside application source.",
    filename: "src/agent.ts",
    code: `const response = await tysel.llm.generate({
  model: "default",
  system: "Answer with one sentence.",
  input: "Summarize the queued job.",
  maxOutputTokens: 128,
});

return Response.json({
  output: response.output,
  usage: response.usage,
});`,
    grant: "service · provider config + declared secret",
    docsHref: "/reference/runtime/capabilities#llm-generation",
    exampleHref:
      "https://github.com/wangcch/tysel/tree/main/examples/durable-agent",
    exampleLabel: "durable-agent",
  },
  {
    id: "mcp",
    label: "MCP tool",
    blurb:
      "Validated JSON tools over bounded stdio. Isolated workers only see brokered secret handles.",
    filename: "src/index.ts",
    code: `import { defineApp } from "tysel";

export default defineApp({
  tasks: {
    lookup: {
      kind: "mcp",
      description: "Look up a customer",
      input: { customerId: "string" },
      async handler({ customerId }) {
        const credential = await tysel.secrets.ref(
          "CUSTOMER_API_KEY",
        );
        return { customerId, credential };
      },
    },
  },
});`,
    grant: "isolated · brokered secret handle only",
    docsHref: "/reference/runtime/application#mcp-tasks",
    exampleHref: "https://github.com/wangcch/tysel/tree/main/examples/mcp-tool",
    exampleLabel: "mcp-tool",
  },
  {
    id: "sqlite",
    label: "SQLite worker",
    blurb:
      "Local state through a runtime-owned client. The path is grant-bound, not ambient cwd access.",
    filename: "src/index.ts",
    code: `await tysel.sqlite.exec(
  "CREATE TABLE IF NOT EXISTS jobs (id TEXT PRIMARY KEY, state TEXT)",
);

const jobs = await tysel.sqlite.query(
  "SELECT id, state FROM jobs",
);

return Response.json({ jobs });`,
    grant: "service · runtime-relative SQLite grant",
    docsHref: "/reference/runtime/capabilities#sql",
    exampleHref:
      "https://github.com/wangcch/tysel/tree/main/examples/sqlite-worker",
    exampleLabel: "sqlite-worker",
  },
  {
    id: "postgres",
    label: "Postgres service",
    blurb:
      "Query a named database. The connection URL stays in the host; the app only sees the grant name.",
    filename: "src/index.ts",
    code: `const rows = await tysel.postgres.query(
  "SELECT id, name FROM greetings WHERE id = $1",
  [1],
);

return Response.json({ rows });`,
    grant: "service · named Postgres grant (e.g. main:read-write)",
    docsHref: "/reference/runtime/capabilities#sql",
    exampleHref:
      "https://github.com/wangcch/tysel/tree/main/examples/postgres-service",
    exampleLabel: "postgres-service",
  },
  {
    id: "websocket",
    label: "WebSocket",
    blurb:
      "Inbound upgrades need an explicit server flag. Outbound sockets follow the same fetch allowlist.",
    filename: "src/socket.ts",
    code: `import type { TyselApp } from "@tysel/types";

export default {
  fetch(_request, runtime) {
    const socket = runtime.acceptWebSocket();

    socket.onmessage = async (event) => {
      await socket.send(\`echo: \${event.data ?? ""}\`);
    };

    return new Response(null, { status: 101 });
  },
} satisfies TyselApp;`,
    grant: "service · server.websocket = true",
    docsHref: "/reference/runtime/capabilities#websockets",
  },
  {
    id: "filesystem",
    label: "Filesystem",
    blurb:
      "UTF-8 read and write only beneath manifest-pinned roots. There is no open-ended FS.",
    filename: "src/report.ts",
    code: `const source = await tysel.fs.read(
  "./fixtures/input.json",
);

await tysel.fs.write(
  "./data/output/result.json",
  source,
);`,
    grant: "service · matching fs_read / fs_write roots",
    docsHref: "/reference/runtime/capabilities#filesystem",
  },
  {
    id: "wasm",
    label: "Wasm Component",
    blurb:
      "A language-neutral, one-shot JSON task through the Component Model. Deny by default; experimental.",
    filename: "src/lib.rs",
    code: `impl Task for EchoComponent {
    type Input = Input;
    type Output = Output;

    fn run(input: Self::Input) -> Result<Self::Output, String> {
        Ok(Output { value: input.value })
    }
}

export!(Component);`,
    grant: "component · experimental · isolated-task trust",
    docsHref: "/reference/component",
    exampleHref: "/docs/guides/wasm-component-rust",
    exampleLabel: "Rust / Go guides",
  },
];

/** Contract index — surfaces Tysel actually ships, not a kitchen-sink stdlib. */
const apiGroups: ApiGroup[] = [
  {
    title: "Services",
    items: [
      {
        name: "fetch handler",
        description: "Web Request / Response entrypoint",
        href: "/reference/runtime/application#http-handler",
      },
      {
        name: "WebSocket",
        description: "Inbound upgrade or allowlisted client",
        href: "/reference/runtime/capabilities#websockets",
      },
      {
        name: "cron · queue",
        description: "Scheduled and queued task handlers",
        href: "/reference/runtime/application#cron-tasks",
      },
    ],
  },
  {
    title: "Agents",
    items: [
      {
        name: "tysel.durable",
        description: "Effects, sleep, signals, resume",
        href: "/reference/runtime/durable",
      },
      {
        name: "tysel.llm",
        description: "Host-brokered model generation",
        href: "/reference/runtime/capabilities#llm-generation",
      },
      {
        name: "MCP tasks",
        description: "Bounded stdio tools with validation",
        href: "/reference/runtime/application#mcp-tasks",
      },
    ],
  },
  {
    title: "Host grants",
    items: [
      {
        name: "tysel.secrets",
        description: "Opaque handles, never plaintext env",
        href: "/reference/runtime/capabilities#secrets",
      },
      {
        name: "tysel.sqlite / postgres",
        description: "Named, grant-bound SQL clients",
        href: "/reference/runtime/capabilities#sql",
      },
      {
        name: "tysel.fs",
        description: "Read / write under pinned roots",
        href: "/reference/runtime/capabilities#filesystem",
      },
    ],
  },
  {
    title: "Web APIs",
    items: [
      {
        name: "Request · Response · URL",
        description: "Web-API-first application surface",
        href: "/reference/javascript",
      },
      {
        name: "fetch",
        description: "Outbound HTTP on an allowlist",
        href: "/reference/javascript/fetch",
      },
      {
        name: "crypto",
        description: "Web Crypto subset, no Node crypto",
        href: "/reference/javascript/crypto",
      },
    ],
  },
  {
    title: "Wasm Components",
    items: [
      {
        name: "Component ABI",
        description: "One-shot JSON task world",
        href: "/reference/component/abi",
      },
      {
        name: "Rust SDK",
        description: "Guest types and dispatcher",
        href: "/reference/component/rust-sdk",
      },
      {
        name: "Go SDK",
        description: "Generated bindings",
        href: "/reference/component/go-sdk",
      },
    ],
  },
  {
    title: "Contracts",
    items: [
      {
        name: "permissions",
        description: "Manifest grants and reductions",
        href: "/reference/manifest/permissions",
      },
      {
        name: "tysel inspect",
        description: "Effective authority and limits",
        href: "/reference/cli/development#tysel-inspect",
      },
      {
        name: "@tysel/test",
        description: "Handlers in a fresh isolate",
        href: "/reference/runtime/testing",
      },
    ],
  },
];

const TOKEN =
  /(\/\/[^\n]*|"(?:\\.|[^"\\])*"|`(?:\\.|[^`\\])*`|\b(?:export|default|async|await|return|const|let|import|from|type|interface|function|fn|impl|for|new|Promise|Response|Request|URL|Input|Output|Ok|Self|Result|String|DurableContext|Component)\b)/g;

function highlight(code: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let key = 0;

  for (const match of code.matchAll(TOKEN)) {
    const index = match.index ?? 0;
    if (index > last) nodes.push(code.slice(last, index));

    const token = match[0];
    const className = token.startsWith("//")
      ? "text-white/35"
      : token.startsWith('"') || token.startsWith("`")
        ? "text-tysel-lime"
        : "text-tysel-blue";

    nodes.push(
      <span key={key} className={className}>
        {token}
      </span>,
    );
    key += 1;
    last = index + token.length;
  }

  if (last < code.length) nodes.push(code.slice(last));
  return nodes;
}

function ExamplePanel({ example }: { example: Example }) {
  return (
    <div className="home-terminal flex min-h-0 flex-1 flex-col bg-tysel-ink text-white">
      <div className="flex flex-wrap items-baseline justify-between gap-x-6 gap-y-2 border-b border-white/10 px-5 py-4">
        <p
          key={example.id}
          className="home-code-fade max-w-xl text-sm leading-6 text-white/65"
        >
          {example.blurb}
        </p>
        <Link
          href={example.docsHref}
          className="shrink-0 text-sm text-white/70 underline decoration-white/30 underline-offset-4 hover:text-white hover:decoration-white"
        >
          Docs →
        </Link>
      </div>

      <div className="flex items-center justify-between border-b border-white/10 px-5 py-2 font-mono text-xs text-white/45">
        <span>{example.filename}</span>
        <CopyButton
          value={example.code}
          className="text-white/45 hover:bg-white/10 hover:text-white"
        />
      </div>

      <pre
        key={example.id}
        className="home-code-fade overflow-x-auto px-5 py-5 font-mono text-[13px] leading-6 text-white/88"
      >
        <code>{highlight(example.code)}</code>
      </pre>

      <div className="mt-auto flex flex-col justify-between gap-2 border-t border-white/10 px-5 py-3 text-xs text-white/45 sm:flex-row sm:items-center">
        <span>
          <span className="text-white/30">Grant · </span>
          {example.grant}
        </span>
        {example.exampleHref ? (
          <Link
            href={example.exampleHref}
            className="shrink-0 text-white/70 underline decoration-white/25 underline-offset-4 hover:text-white hover:decoration-white"
          >
            {example.exampleLabel
              ? `${example.exampleLabel} →`
              : "Open example →"}
          </Link>
        ) : (
          <span className="shrink-0 text-white/35">Reference recipe</span>
        )}
      </div>
    </div>
  );
}

export function RuntimeCapabilities() {
  const [id, setId] = useState(examples[0].id);
  const listId = useId();
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const active = examples.find((item) => item.id === id) ?? examples[0];

  function moveItem(event: KeyboardEvent<HTMLButtonElement>, index: number) {
    const delta =
      event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0;
    if (delta === 0 && event.key !== "Home" && event.key !== "End") return;

    event.preventDefault();
    const nextIndex =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? examples.length - 1
          : (index + delta + examples.length) % examples.length;
    setId(examples[nextIndex].id);
    itemRefs.current[nextIndex]?.focus();
  }

  return (
    <div>
      <div className="grid overflow-hidden border border-fd-border lg:grid-cols-[minmax(0,14rem)_minmax(0,1fr)]">
        <div
          role="tablist"
          aria-label="Tysel workload examples"
          aria-orientation="vertical"
          className="border-b border-fd-border bg-fd-background lg:border-r lg:border-b-0"
        >
          <ul className="divide-y divide-fd-border">
            {examples.map((item, index) => {
              const selected = item.id === active.id;
              return (
                <li key={item.id}>
                  <button
                    ref={(node) => {
                      itemRefs.current[index] = node;
                    }}
                    id={`${listId}-${item.id}`}
                    type="button"
                    role="tab"
                    aria-selected={selected}
                    aria-controls={`${listId}-panel`}
                    tabIndex={selected ? 0 : -1}
                    onClick={() => setId(item.id)}
                    onKeyDown={(event) => moveItem(event, index)}
                    className={`flex w-full items-center gap-3 px-4 py-3 text-left text-sm transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-tysel-blue ${
                      selected
                        ? "bg-fd-accent text-fd-foreground"
                        : "text-fd-muted-foreground hover:bg-fd-accent/60 hover:text-fd-foreground"
                    }`}
                  >
                    <span
                      aria-hidden="true"
                      className={`size-1.5 shrink-0 rounded-full transition-colors ${
                        selected ? "bg-tysel-blue" : "bg-fd-border"
                      }`}
                    />
                    {item.label}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>

        <div
          id={`${listId}-panel`}
          role="tabpanel"
          aria-labelledby={`${listId}-${active.id}`}
          className="flex min-h-[24rem] flex-col"
        >
          <ExamplePanel example={active} />
        </div>
      </div>

      <div className="mt-16 flex flex-col justify-between gap-4 sm:flex-row sm:items-end">
        <div>
          <h3 className="font-heading max-w-xl text-xl font-medium tracking-tight text-balance sm:text-2xl">
            Lookup by contract
          </h3>
          <p className="mt-2 max-w-xl text-sm leading-6 text-fd-muted-foreground">
            Each link is an exact interface page — accepted values, defaults,
            and what the profile can still deny.
          </p>
        </div>
        <Link
          href="/reference"
          className="inline-flex shrink-0 items-center border border-fd-border px-4 py-2 text-sm font-medium transition-colors hover:bg-fd-accent"
        >
          API reference →
        </Link>
      </div>

      <div className="mt-8 grid gap-px bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
        {apiGroups.map((group) => (
          <div key={group.title} className="bg-fd-background p-5">
            <p className="text-[11px] font-medium uppercase tracking-[0.14em] text-fd-muted-foreground">
              {group.title}
            </p>
            <ul className="mt-4 space-y-4">
              {group.items.map((item) => (
                <li key={item.name}>
                  <Link href={item.href} className="group block">
                    <p className="font-mono text-sm text-tysel-blue group-hover:underline">
                      {item.name}
                    </p>
                    <p className="mt-1 text-sm leading-5 text-fd-muted-foreground">
                      {item.description}
                    </p>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <p className="mt-6 max-w-3xl text-sm leading-6 text-fd-muted-foreground">
        Shell, child processes, FFI, and dynamic libraries are outside the
        application contract — not missing features. See the{" "}
        <Link
          href="/docs/capabilities"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          capability matrix
        </Link>
        ,{" "}
        <Link
          href="/docs/concepts/execution-profiles"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          execution profiles
        </Link>
        , and{" "}
        <Link
          href="/docs/guides/examples"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          example gallery
        </Link>
        .
      </p>
    </div>
  );
}
