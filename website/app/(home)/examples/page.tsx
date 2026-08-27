import Link from "next/link";
import type { Metadata } from "next";
import { CopyButton } from "@/components/copy-button";
import {
  exampleSections,
  featuredExample,
  featuredSnippet,
  recipes,
  type Example,
  type ExampleStatus,
} from "@/lib/examples-catalog";
import { absoluteUrl, canonicalUrl } from "@/lib/shared";

export const metadata: Metadata = {
  title: "Examples",
  description:
    "Runnable Tysel acceptance paths for services, durable agents, isolation, and Wasm Components.",
  alternates: { canonical: canonicalUrl("/examples") },
  openGraph: {
    url: canonicalUrl("/examples"),
    images: [absoluteUrl("/opengraph-image")],
  },
};

const statusLabel: Record<ExampleStatus, string> = {
  runnable: "Runnable",
  experimental: "Experimental",
  recipe: "Recipe",
};

const examplesDownload = `version="$(tysel --version | awk '{print $2}')"
curl -fsSL "https://github.com/wangcch/tysel/archive/refs/tags/v\${version}.tar.gz" | tar -xz
cd "tysel-\${version}/examples/hello-service"`;

const starterTemplates = [
  {
    name: "HTTP service",
    description: "Fetch-style service on the Web API runtime.",
    command: `tysel init my-service --template http --yes
cd my-service
tysel task verify
tysel dev`,
  },
  {
    name: "Queue worker",
    description: "Service with one named Queue handler.",
    command: `tysel init my-worker --template worker --yes
cd my-worker
tysel task verify
tysel run`,
  },
  {
    name: "MCP tool",
    description: "Isolated MCP stdio tool with validated input.",
    command: `tysel init my-tool --template mcp --yes
cd my-tool
tysel task verify
tysel mcp`,
  },
  {
    name: "Minimal service",
    description: "Smallest Fetch handler for custom application structure.",
    command: `tysel init my-app --template minimal --yes
cd my-app
tysel task verify
tysel dev`,
  },
];

function MetaLine({ example }: { example: Example }) {
  return (
    <p className="font-mono text-xs text-tysel-blue">
      {statusLabel[example.status]} · {example.profile} · {example.capabilities}
    </p>
  );
}

function ExampleRow({ example }: { example: Example }) {
  const sourceExternal = example.sourceHref.startsWith("https://");

  return (
    <div className="grid gap-3 border-b border-fd-border px-5 py-5 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] sm:gap-8">
      <div>
        <h3 className="text-base font-medium">{example.name}</h3>
        <div className="mt-1.5">
          <MetaLine example={example} />
        </div>
        <p className="mt-3 text-sm leading-6 text-fd-muted-foreground">
          {example.purpose}
        </p>
        <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-sm">
          <Link
            href={example.sourceHref}
            target={sourceExternal ? "_blank" : undefined}
            rel={sourceExternal ? "noreferrer" : undefined}
            className="underline-offset-4 hover:underline"
          >
            {example.sourceLabel ?? "Development source"}
          </Link>
          {example.docsHref ? (
            <Link
              href={example.docsHref}
              className="text-fd-muted-foreground underline-offset-4 hover:underline"
            >
              Guide
            </Link>
          ) : null}
          <Link
            href={example.contractHref}
            className="text-fd-muted-foreground underline-offset-4 hover:underline"
          >
            Contract
          </Link>
        </div>
      </div>
      <div className="overflow-hidden border border-fd-border bg-fd-muted/30">
        <div className="flex items-center justify-between border-b border-fd-border px-3 py-1.5 font-mono text-[11px] text-fd-muted-foreground">
          <span>{example.runLabel ?? "after opening the example directory"}</span>
          <CopyButton value={example.run} />
        </div>
        <pre className="overflow-x-auto px-3 py-3 font-mono text-[12px] leading-5">
          <code>{example.run}</code>
        </pre>
      </div>
    </div>
  );
}

export default function ExamplesPage() {
  return (
    <main className="mx-auto w-full max-w-6xl px-6 py-16">
      <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
        Examples
      </p>
      <h1 className="font-heading mt-3 max-w-3xl text-4xl font-medium tracking-tighter text-balance">
        Create a starter or open an exact example.
      </h1>
      <p className="mt-4 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
        Use a built-in template when starting a new project. Use the
        version-matched repository examples when you need a complete capability
        walkthrough such as WebSocket, Postgres, Durable execution, or Wasm.
      </p>

      <section className="mt-10">
        <div className="flex items-end justify-between gap-6">
          <div>
            <h2 className="font-heading text-xl font-medium tracking-tight">
              Start from a built-in template
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
              These commands create the project before validating and running it.
              Choose the closest application shape, then add explicit capabilities.
            </p>
          </div>
          <Link
            href="/reference/cli/project#tysel-init"
            className="hidden shrink-0 text-sm text-tysel-blue underline-offset-4 hover:underline sm:block"
          >
            Init reference
          </Link>
        </div>
        <div className="mt-5 grid gap-px bg-fd-border md:grid-cols-2">
          {starterTemplates.map((template) => (
            <div key={template.name} className="bg-fd-background">
              <div className="flex items-start justify-between gap-4 border-b border-fd-border px-4 py-3">
                <div>
                  <h3 className="text-sm font-medium">{template.name}</h3>
                  <p className="mt-1 text-xs leading-5 text-fd-muted-foreground">
                    {template.description}
                  </p>
                </div>
                <CopyButton value={template.command} />
              </div>
              <pre className="overflow-x-auto px-4 py-4 font-mono text-[12px] leading-5">
                <code>{template.command}</code>
              </pre>
            </div>
          ))}
        </div>
      </section>

      <section className="mt-14 max-w-3xl border border-fd-border">
        <div className="flex items-center justify-between border-b border-fd-border px-4 py-2 font-mono text-[11px] text-fd-muted-foreground">
          <span>download version-matched examples</span>
          <CopyButton value={examplesDownload} />
        </div>
        <pre className="overflow-x-auto px-4 py-4 font-mono text-[12px] leading-5">
          <code>{examplesDownload}</code>
        </pre>
        <p className="border-t border-fd-border px-4 py-3 text-xs leading-5 text-fd-muted-foreground">
          Replace <code>hello-service</code> with another directory listed below.
          This downloads application source without a fork, repository clone,
          or local Tysel build.
        </p>
      </section>

      <section className="mt-12 border border-fd-border">
        <div className="border-b border-fd-border px-5 py-4 sm:flex sm:items-start sm:justify-between sm:gap-8">
          <div>
            <p className="text-xs font-medium uppercase tracking-[0.16em] text-fd-muted-foreground">
              Start with the durable path
            </p>
            <h2 className="font-heading mt-2 text-2xl font-medium tracking-tight text-balance">
              {featuredExample.name}
            </h2>
            <div className="mt-2">
              <MetaLine example={featuredExample} />
            </div>
            <p className="mt-3 max-w-xl text-sm leading-6 text-fd-muted-foreground">
              {featuredExample.purpose} Requires a real OpenAI-compatible
              endpoint; see the README for{" "}
              <code className="font-mono text-fd-foreground">demo.sh</code>.
            </p>
            <div className="mt-4 flex flex-wrap gap-x-4 gap-y-1 text-sm">
              <Link
                href={featuredExample.sourceHref}
                target="_blank"
                rel="noreferrer"
                className="underline-offset-4 hover:underline"
              >
                Development source
              </Link>
              <Link
                href={featuredExample.docsHref!}
                className="text-fd-muted-foreground underline-offset-4 hover:underline"
              >
                Durable execution
              </Link>
              <Link
                href={featuredExample.contractHref}
                className="text-fd-muted-foreground underline-offset-4 hover:underline"
              >
                Durable API
              </Link>
            </div>
          </div>
          <div className="mt-6 w-full max-w-md overflow-hidden border border-fd-border bg-fd-muted/30 sm:mt-0">
            <div className="flex items-center justify-between border-b border-fd-border px-3 py-1.5 font-mono text-[11px] text-fd-muted-foreground">
              <span>{featuredExample.runLabel ?? "after opening the example directory"}</span>
              <CopyButton value={featuredExample.run} />
            </div>
            <pre className="overflow-x-auto px-3 py-3 font-mono text-[12px] leading-5">
              <code>{featuredExample.run}</code>
            </pre>
          </div>
        </div>
        <div className="bg-tysel-ink text-white">
          <div className="flex items-center justify-between border-b border-white/10 px-5 py-2 font-mono text-[11px] text-white/45">
            <span>{featuredSnippet.filename}</span>
            <CopyButton
              value={featuredSnippet.code}
              className="text-white/45 hover:bg-white/10 hover:text-white"
            />
          </div>
          <pre className="overflow-x-auto px-5 py-5 font-mono text-[13px] leading-6 text-white/88">
            <code>{featuredSnippet.code}</code>
          </pre>
        </div>
      </section>

      <section className="mt-14 border border-fd-border px-5 py-5">
        <h2 className="text-sm font-medium uppercase tracking-[0.16em] text-fd-muted-foreground">
          One installed toolchain
        </h2>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
          Install Tysel once, then use the same <code>tysel</code> command in
          every example project. The managed installation already includes the
          matching service runtime and isolated worker; no repository build or
          worker-path override is required.
        </p>
        <Link
          href="/docs/install"
          className="mt-3 inline-block text-sm text-tysel-blue underline-offset-4 hover:underline"
        >
          Installation guide
        </Link>
      </section>

      <div className="mt-16 space-y-12">
        {exampleSections.map((section) => (
          <section key={section.id} id={section.id}>
            <h2 className="font-heading text-xl font-medium tracking-tight">
              {section.title}
            </h2>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
              {section.intent}
            </p>
            {section.note ? (
              <p className="mt-4 max-w-3xl border-l-2 border-tysel-blue pl-4 text-xs leading-5 text-fd-muted-foreground">
                <span className="font-medium text-fd-foreground">
                  Effective profile default.
                </span>{" "}
                {section.note}
              </p>
            ) : null}
            <div className="mt-5 border border-fd-border">
              {section.examples
                .filter((example) => example.id !== featuredExample.id)
                .map((example) => (
                  <ExampleRow key={example.id} example={example} />
                ))}
            </div>
          </section>
        ))}
      </div>

      <section className="mt-16" id="recipes">
        <h2 className="font-heading text-xl font-medium tracking-tight">
          Supported, no dedicated tree yet
        </h2>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
          These APIs ship today. They are reference recipes — not missing
          Node-style stdlib examples, and not end-to-end acceptance paths.
        </p>
        <div className="mt-5 grid gap-px bg-fd-border sm:grid-cols-2">
          {recipes.map((item) => (
            <Link
              key={item.name}
              href={item.href}
              className="bg-fd-background px-5 py-4 transition-colors hover:bg-fd-accent"
            >
              <p className="font-medium">{item.name}</p>
              <p className="mt-1 text-sm leading-6 text-fd-muted-foreground">
                {item.purpose}
              </p>
              <p className="mt-3 font-mono text-xs text-tysel-blue">
                Recipe · contract only
              </p>
            </Link>
          ))}
        </div>
      </section>

      <p className="mt-14 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
        Shell, child processes, FFI, and dynamic libraries stay outside the
        application contract. See the{" "}
        <Link
          href="/docs/capabilities"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          capability matrix
        </Link>{" "}
        and{" "}
        <Link
          href="/docs/guides/examples"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          docs gallery
        </Link>
        .
      </p>
    </main>
  );
}
