import Link from "next/link";
import type { Metadata } from "next";
import { ArtifactDemo } from "@/components/home/artifact-demo";
import { FiveMinutes } from "@/components/home/five-minutes";
import { InstallPanel } from "@/components/home/install-panel";
import { RuntimeCapabilities } from "@/components/home/runtime-capabilities";
import { StructuredData } from "@/components/seo/structured-data";
import { absoluteUrl, canonicalUrl, githubUrl } from "@/lib/shared";

const description =
  "A native TypeScript runtime for services and agents. One executable, explicit capabilities, and durable work that survives restarts.";

export const metadata: Metadata = {
  alternates: { canonical: canonicalUrl() },
  openGraph: {
    url: canonicalUrl(),
    title: "Tysel — Write TypeScript. Ship a binary.",
    description,
    images: [
      {
        url: absoluteUrl("/opengraph-image"),
        width: 1200,
        height: 630,
        alt: "Tysel — Write TypeScript. Ship a binary.",
      },
    ],
  },
};

const jsonLd = {
  "@context": "https://schema.org",
  "@graph": [
    {
      "@type": "WebSite",
      "@id": `${canonicalUrl()}#website`,
      url: canonicalUrl(),
      name: "Tysel",
      description,
      inLanguage: "en",
    },
    {
      "@type": "SoftwareApplication",
      "@id": `${canonicalUrl()}#software`,
      name: "Tysel",
      description,
      url: canonicalUrl(),
      downloadUrl: absoluteUrl("/install.sh"),
      softwareVersion: "0.1.0",
      applicationCategory: "DeveloperApplication",
      operatingSystem: "Linux, macOS",
      isAccessibleForFree: true,
      license: "https://www.apache.org/licenses/LICENSE-2.0",
      sameAs: githubUrl,
    },
  ],
};

const proofs = [
  {
    title: "Web API first",
    body: "Request, Response, fetch, streams, and Web Crypto. Not a Node.js compatibility layer.",
    href: "/reference/javascript",
  },
  {
    title: "No Node in production",
    body: "The packaged executable embeds the runtime. Targets do not need V8 or node_modules.",
    href: "/docs/getting-started",
  },
  {
    title: "Deny by default",
    body: "Network, secrets, files, and databases stay closed until the manifest grants them.",
    href: "/docs/capabilities",
  },
  {
    title: "Durable resume",
    body: "Steps, effects, sleep, and signals persist. Restarts replay completed work instead of repeating it.",
    href: "/docs/concepts/durable-execution",
  },
];

const contracts = [
  {
    n: "01",
    title: "Ship one executable",
    body: "TypeScript becomes an application package, then a native file. Checksums, SBOM, compatibility, and evidence travel with the release.",
    href: "/docs/getting-started",
  },
  {
    n: "02",
    title: "Bound every capability",
    body: "The manifest requests authority. The execution profile and deployment policy can only reduce it. Linux isolated workers add Landlock, seccomp, and cgroup limits.",
    href: "/docs/concepts/execution-profiles",
  },
  {
    n: "03",
    title: "Resume durable work",
    body: "LLM call → persisted effect → approval → restart → resume → save once. Durability is a runtime primitive, not an exactly-once claim.",
    href: "/docs/concepts/durable-execution",
  },
];

export default function HomePage() {
  return (
    <main>
      <StructuredData data={jsonLd} />
      <section className="relative overflow-hidden border-b border-fd-border">
        <div className="home-atmosphere pointer-events-none absolute inset-0" />
        <div className="home-grid pointer-events-none absolute inset-0" />
        <div className="relative mx-auto grid max-w-6xl gap-12 px-6 pt-16 pb-20 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,0.95fr)] lg:pt-20 lg:pb-28">
          <div className="home-fade-up">
            <h1 className="font-heading text-4xl leading-[1.05] font-medium tracking-tighter text-balance sm:text-6xl sm:leading-[1.05]">
              Write TypeScript.
              <br />
              Ship a <span className="text-tysel-blue">binary</span>.
            </h1>
            <p className="mt-6 max-w-xl text-base leading-7 text-fd-muted-foreground sm:text-lg">
              A native runtime for services and AI agents — one executable,
              explicit capabilities, durable work that survives restarts.
            </p>
            <div className="mt-8 flex flex-wrap gap-3">
              <Link
                href="/docs/getting-started"
                className="bg-fd-foreground px-4 py-2 text-sm font-medium text-fd-background transition-opacity hover:opacity-90"
              >
                Get started
              </Link>
              <Link
                href="/docs"
                className="border border-fd-border px-4 py-2 text-sm font-medium transition-colors hover:bg-fd-accent"
              >
                Read the docs
              </Link>
            </div>
            <div className="mt-8">
              <InstallPanel />
            </div>
          </div>
          <div className="home-fade-up home-fade-up-delay">
            <ArtifactDemo />
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto grid max-w-6xl gap-px bg-fd-border sm:grid-cols-2 lg:grid-cols-4">
          {proofs.map((item) => (
            <Link
              key={item.title}
              href={item.href}
              className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
            >
              <h2 className="text-sm font-medium">{item.title}</h2>
              <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                {item.body}
              </p>
            </Link>
          ))}
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            Three product contracts
          </p>
          <h2 className="font-heading mt-3 max-w-3xl text-3xl font-medium tracking-tight text-balance sm:text-4xl">
            Bun and Deno optimize a general JavaScript toolchain. Tysel ships a
            narrower production contract.
          </h2>
          <div className="mt-12 grid gap-px bg-fd-border lg:grid-cols-3">
            {contracts.map((item) => (
              <Link
                key={item.n}
                href={item.href}
                className="bg-fd-background p-6 transition-colors hover:bg-fd-accent"
              >
                <span className="font-mono text-xs text-tysel-blue">{item.n}</span>
                <h3 className="mt-3 text-xl font-medium">{item.title}</h3>
                <p className="mt-3 text-sm leading-6 text-fd-muted-foreground">
                  {item.body}
                </p>
              </Link>
            ))}
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <div className="flex items-center gap-2">
            <span
              aria-hidden="true"
              className="size-2 shrink-0 bg-tysel-blue"
            />
            <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
              Workloads
            </p>
          </div>
          <h2 className="font-heading mt-4 max-w-3xl text-3xl font-medium tracking-tight text-balance sm:text-4xl">
            Services, agents, and Wasm — each with a named grant.
          </h2>
          <p className="mt-4 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
            A narrow host surface, not a kitchen-sink stdlib. Network, secrets,
            storage, and LLM access stay closed until the manifest opens them.
          </p>
          <div className="mt-10">
            <RuntimeCapabilities />
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            Five minutes with Tysel
          </p>
          <h2 className="font-heading mt-3 text-3xl font-medium tracking-tight text-balance">
            One loop from source to an executable.
          </h2>
          <div className="mt-10">
            <FiveMinutes />
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            Production evidence
          </p>
          <h2 className="font-heading mt-3 max-w-3xl text-3xl font-medium tracking-tight text-balance">
            Claims wait for a named release. The gates are already public.
          </h2>
          <div className="mt-10 grid gap-px bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
            {[
              ["Compatibility and evidence", "/reference/cli/evidence"],
              ["SBOM, licenses, checksums", "/reference/cli/evidence"],
              ["Security and isolation", "/docs/security"],
              ["Benchmark methodology", "/benchmarks"],
              ["Production operations", "/docs/operations/production"],
              ["Limits and defaults", "/reference/limits-and-defaults"],
            ].map(([label, href]) => (
              <Link
                key={label}
                href={href}
                className="bg-fd-background px-4 py-4 text-sm transition-colors hover:bg-fd-accent"
              >
                {label} →
              </Link>
            ))}
          </div>
          <p className="mt-6 max-w-3xl text-sm leading-6 text-fd-muted-foreground">
            Release-admission thresholds include a 20 MiB artifact, 15 ms median
            cold start, 32 MiB idle Linux PSS, 5 ms warm isolate p50, and 10 ms
            durable resume p50. Those are engineering gates, not marketing
            measurements.
          </p>
        </div>
      </section>

      <section>
        <div className="relative overflow-hidden">
          <div className="home-atmosphere pointer-events-none absolute inset-0 opacity-60" />
          <div className="relative mx-auto max-w-6xl px-6 py-24 text-center">
            <h2 className="font-heading text-3xl font-medium tracking-tight text-balance sm:text-5xl">
              From TypeScript source to one production artifact.
            </h2>
            <div className="mt-8 flex justify-center gap-3">
              <Link
                href="/docs/getting-started"
                className="bg-fd-foreground px-4 py-2 text-sm font-medium text-fd-background transition-opacity hover:opacity-90"
              >
                Build your first service
              </Link>
              <Link
                href="/docs/guides/examples"
                className="border border-fd-border px-4 py-2 text-sm font-medium transition-colors hover:bg-fd-accent"
              >
                Run the durable agent example
              </Link>
            </div>
          </div>
        </div>
      </section>
    </main>
  );
}
