import type { Metadata } from "next";
import Link from "next/link";
import { AdmissionScoreboard } from "@/components/benchmarks/admission-scoreboard";
import { RuntimeSuites } from "@/components/benchmarks/runtime-suites";
import { CopyButton } from "@/components/copy-button";
import { loadAdmissionBoard } from "@/lib/load-benchmark-evidence";
import { absoluteUrl, canonicalUrl, githubUrl } from "@/lib/shared";

export const metadata: Metadata = {
  title: "Benchmarks",
  description:
    "CI release-admission gates for Tysel on fixed Linux runners — not a production performance SLA.",
  alternates: { canonical: canonicalUrl("/benchmarks") },
  openGraph: {
    url: canonicalUrl("/benchmarks"),
    images: [absoluteUrl("/opengraph-image")],
  },
};

const publicationSteps = [
  {
    n: "01",
    title: "Lock the environment",
    body: "Pin runtime versions and hashes; reject dirty trees, loaded hosts, and unexpected CPU settings.",
  },
  {
    n: "02",
    title: "Rotate every runtime",
    body: "Run Node.js, Bun, Deno, and Tysel in four orders so one runtime does not always benefit from the same position.",
  },
  {
    n: "03",
    title: "Prove both architectures",
    body: "Complete three record cycles on dedicated Linux x86_64 and arm64 runners. Architectures are never merged into one ranking.",
  },
  {
    n: "04",
    title: "Replicate externally",
    body: "Repeat the workload from a separate load host with the exact recorded server binary before a comparison reaches this page.",
  },
];

const evidenceTiers = [
  {
    level: "01",
    label: "Release guards",
    state: "Shown",
  },
  {
    level: "02",
    label: "Runtime suites",
    state: "Observe",
  },
  {
    level: "03",
    label: "Cross-runtime",
    state: "Withheld",
  },
];

const benchCommand = `tysel bench startup
tysel bench all --format json
tysel bench all --format json \\
  --evidence dist/benchmark-evidence.json`;

export default function BenchmarksPage() {
  const board = loadAdmissionBoard();
  const published = board.status === "published";
  const sample = board.provenance?.sample === true;
  const claim = published && !sample;

  return (
    <main className="mx-auto w-full max-w-6xl px-6 py-14 sm:py-16">
      <div className="max-w-2xl">
        <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
          CI admission
        </p>
        <h1 className="font-heading mt-2 text-4xl font-medium tracking-tighter text-balance sm:text-5xl">
          {claim
            ? "Seven gates. CI measured."
            : sample
              ? "Seven gates. Sample layout."
              : "Seven gates. Waiting on data."}
        </h1>
        <p className="mt-3 text-sm leading-6 text-fd-muted-foreground">
          {sample
            ? "Illustrative layout only — not a published CI claim."
            : "Fixed-runner regression gates. Not a production performance SLA."}
        </p>
      </div>

      <div className="mt-8">
        <AdmissionScoreboard
          rows={board.rows}
          provenance={board.provenance}
          published={published}
        />
      </div>

      <section
        aria-label="Evidence ladder"
        className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-2 text-xs text-fd-muted-foreground"
      >
        <span className="font-mono text-[10px] uppercase tracking-[0.14em]">
          Ladder
        </span>
        {evidenceTiers.map((tier) => (
          <p key={tier.level} className="flex items-baseline gap-1.5">
            <span className="font-mono text-[10px] text-tysel-blue">
              {tier.level}
            </span>
            <span className="text-fd-foreground">{tier.label}</span>
            <span className="font-mono text-[10px] uppercase tracking-[0.12em] text-fd-muted-foreground">
              {tier.state}
            </span>
          </p>
        ))}
      </section>

      <RuntimeSuites suites={board.suites} />

      <section className="mt-14 max-w-3xl">
        <h2 className="font-heading text-xl font-medium tracking-tight">
          Reproduce locally
        </h2>
        <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
          Release mode, clean tree. Time gates admit on p50; suites may still
          report p95/p99. Startup uses 2 warm-ups + 11 samples on a warm page
          cache. Linux PSS is the memory result of record.
        </p>

        <div className="mt-5 overflow-hidden border border-fd-border bg-fd-muted/30">
          <div className="flex items-center justify-between border-b border-fd-border px-3 py-1.5 font-mono text-[11px] text-fd-muted-foreground">
            <span>release evidence</span>
            <CopyButton value={benchCommand} />
          </div>
          <pre className="overflow-x-auto px-4 py-4 font-mono text-[12px] leading-5">
            <code>{benchCommand}</code>
          </pre>
        </div>

        <div className="mt-4 flex flex-wrap gap-x-5 gap-y-2 text-sm">
          <Link
            href="/docs/performance"
            className="text-tysel-blue underline-offset-4 hover:underline"
          >
            Performance documentation
          </Link>
          <Link
            href={`${githubUrl}/tree/main/benchmarks`}
            className="text-tysel-blue underline-offset-4 hover:underline"
          >
            Benchmark source ↗
          </Link>
          <Link
            href="/benchmark-evidence/latest.json"
            className="text-tysel-blue underline-offset-4 hover:underline"
          >
            Raw evidence JSON
          </Link>
          <Link
            href="/reference/cli/evidence"
            className="text-fd-muted-foreground underline-offset-4 hover:underline"
          >
            Evidence reference
          </Link>
        </div>
      </section>

      <details className="group mt-14 border border-fd-border">
        <summary className="cursor-pointer list-none px-5 py-4 sm:flex sm:items-center sm:justify-between">
          <div>
            <h2 className="font-heading text-lg font-medium tracking-tight">
              Cross-runtime publication contract
            </h2>
            <p className="mt-1 text-sm text-fd-muted-foreground">
              Separate bar for public comparisons — not these CI gates.
            </p>
          </div>
          <span className="mt-3 font-mono text-[11px] uppercase tracking-[0.12em] text-fd-muted-foreground group-open:hidden sm:mt-0">
            Expand →
          </span>
          <span className="mt-3 hidden font-mono text-[11px] uppercase tracking-[0.12em] text-fd-muted-foreground group-open:inline sm:mt-0">
            Collapse
          </span>
        </summary>
        <div className="border-t border-fd-border px-5 py-5">
          <p className="max-w-2xl text-sm leading-6 text-fd-muted-foreground">
            No cross-runtime winner is published yet. The matched Node.js, Bun,
            Deno, and Tysel harness is public; results stay internal until these
            steps complete on both architectures.
          </p>
          <div className="mt-5 border border-fd-border">
            {publicationSteps.map((step) => (
              <article
                key={step.n}
                className="grid gap-2 border-b border-fd-border px-4 py-4 last:border-b-0 sm:grid-cols-[3rem_minmax(0,1fr)] sm:gap-4"
              >
                <p className="font-mono text-xs text-tysel-blue">{step.n}</p>
                <div>
                  <h3 className="text-sm font-medium">{step.title}</h3>
                  <p className="mt-1 text-sm leading-6 text-fd-muted-foreground">
                    {step.body}
                  </p>
                </div>
              </article>
            ))}
          </div>
          <div className="mt-4">
            <Link
              href={`${githubUrl}/tree/main/benchmarks/comparison`}
              className="text-sm text-tysel-blue underline-offset-4 hover:underline"
            >
              Comparison protocol ↗
            </Link>
          </div>
        </div>
      </details>

      <p className="mt-14 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
        Looking for the product story? Start on{" "}
        <Link
          href="/"
          className="text-fd-foreground underline-offset-4 hover:underline"
        >
          Home
        </Link>
        .
      </p>
    </main>
  );
}
