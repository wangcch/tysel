import { getMessages } from "@/lib/i18n/messages";
import { localePath } from "@/lib/i18n/routing";
import { sourceLocale, type Locale } from "@/lib/i18n/config";
import { alternates } from "@/lib/i18n/seo";
import { T, SourceText } from "@/components/locale-provider";
import { SiteLink as Link } from "@/components/locale-provider";
import type { Metadata } from "next";
import { ArtifactDemo } from "@/components/home/artifact-demo";
import { FiveMinutes } from "@/components/home/five-minutes";
import { InstallPanel } from "@/components/home/install-panel";
import { RuntimeCapabilities } from "@/components/home/runtime-capabilities";
import { StructuredData } from "@/components/seo/structured-data";
import { formatBlogDate, getFeaturedBlogPost } from "@/lib/blog";
import { absoluteUrl, canonicalUrl, githubUrl } from "@/lib/shared";

const description =
  "A native TypeScript runtime for services and agents. One executable, explicit capabilities, and durable work that survives restarts.";

export const metadata: Metadata = {
  alternates: alternates(),
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
      softwareVersion: "0.2.0",
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

export default function HomePage({ locale = sourceLocale }: { locale?: Locale } = {}) {
  const featuredPost = getFeaturedBlogPost(locale);

  return (
    <main>
      <StructuredData data={{ ...jsonLd, "@graph": jsonLd["@graph"].map(item => ({ ...item, url: canonicalUrl(localePath("/", locale)), "@id": `${canonicalUrl(localePath("/", locale))}#${item["@type"] === "WebSite" ? "website" : "software"}`, description: getMessages(locale)["site.description"], inLanguage: locale })) }} />
      <section className="relative overflow-hidden border-b border-fd-border">
        <div className="home-atmosphere pointer-events-none absolute inset-0" />
        <div className="home-grid pointer-events-none absolute inset-0" />
        <div className="relative mx-auto grid grid-cols-1 max-w-6xl gap-12 px-6 pt-16 pb-20 lg:grid-cols-[minmax(0,1.05fr)_minmax(0,0.95fr)] lg:pt-20 lg:pb-28">
          <div className="home-fade-up">
            <h1 className="font-heading text-4xl leading-[1.05] font-medium tracking-tighter text-balance sm:text-6xl sm:leading-[1.05]">
              <T id="home.hero.title" />
              <br />
              <T id="home.hero.ship" /> <span className="text-tysel-blue"><T id="home.hero.binary" /></span>.
            </h1>
            <p className="mt-6 max-w-xl text-base leading-7 text-fd-muted-foreground sm:text-lg">
              <T id="ui.a.native.runtime.for.services.and.ai.agents" />
            </p>
            <div className="mt-8 flex flex-wrap gap-3">
              <Link
                href="/docs/getting-started"
                className="bg-fd-foreground px-4 py-2 text-sm font-medium text-fd-background transition-opacity hover:opacity-90"
              >
                <T id="common.getStarted" />
              </Link>
              <Link
                href="/docs"
                className="border border-fd-border px-4 py-2 text-sm font-medium transition-colors hover:bg-fd-accent"
              >
                <T id="ui.read.the.docs" />
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
              <h2 className="text-sm font-medium"><SourceText text={item.title} /></h2>
              <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                <SourceText text={item.body} />
              </p>
            </Link>
          ))}
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            <T id="ui.three.product.contracts" />
          </p>
          <h2 className="font-heading mt-3 max-w-3xl text-3xl font-medium tracking-tight text-balance sm:text-4xl">
            <T id="ui.bun.and.deno.optimize.a.general.javascript.toolchain" />
          </h2>
          <div className="mt-12 grid gap-px bg-fd-border lg:grid-cols-3">
            {contracts.map((item) => (
              <Link
                key={item.n}
                href={item.href}
                className="bg-fd-background p-6 transition-colors hover:bg-fd-accent"
              >
                <span className="font-mono text-xs text-tysel-blue">{item.n}</span>
                <h3 className="mt-3 text-xl font-medium"><SourceText text={item.title} /></h3>
                <p className="mt-3 text-sm leading-6 text-fd-muted-foreground">
                  <SourceText text={item.body} />
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
              <T id="ui.workloads" />
            </p>
          </div>
          <h2 className="font-heading mt-4 max-w-3xl text-3xl font-medium tracking-tight text-balance sm:text-4xl">
            <T id="ui.services.agents.and.wasm.each.with.a.named" />
          </h2>
          <p className="mt-4 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
            <T id="ui.a.narrow.host.surface.not.a.kitchen.sink" />
          </p>
          <div className="mt-10">
            <RuntimeCapabilities />
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            <T id="ui.five.minutes.with.tysel" />
          </p>
          <h2 className="font-heading mt-3 text-3xl font-medium tracking-tight text-balance">
            <T id="ui.one.loop.from.source.to.an.executable" />
          </h2>
          <div className="mt-10">
            <FiveMinutes />
          </div>
        </div>
      </section>

      <section className="border-b border-fd-border">
        <div className="mx-auto max-w-6xl px-6 py-20">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            <T id="ui.ship.with.proof" />
          </p>
          <h2 className="font-heading mt-3 max-w-3xl text-3xl font-medium tracking-tight text-balance">
            <T id="ui.every.release.carries.evidence.not.just.a.binary" />
          </h2>
          <p className="mt-4 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
            <T id="ui.compatibility.reports.sboms.checksums.and.ci.admission.gates" />
          </p>
          <div className="mt-10 grid gap-px bg-fd-border sm:grid-cols-2 lg:grid-cols-4">
            {[
              ["Compatibility & evidence", "/reference/cli/evidence"],
              ["Security model", "/docs/security"],
              ["Production operations", "/docs/operations/production"],
              ["CI admission", "/benchmarks"],
            ].map(([label, href]) => (
              <Link
                key={label}
                href={href}
                className="bg-fd-background px-4 py-4 text-sm transition-colors hover:bg-fd-accent"
              >
                <SourceText text={label} /> →
              </Link>
            ))}
          </div>
        </div>
      </section>

      {featuredPost ? (
        <section className="border-b border-fd-border">
          <div className="mx-auto max-w-6xl px-6 py-20">
            <div className="flex items-end justify-between gap-6">
              <div>
                <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
                  <T id="ui.from.the.blog" />
                </p>
                <h2 className="font-heading mt-3 max-w-2xl text-3xl font-medium tracking-tight text-balance">
                  <T id="ui.why.tysel.ships.a.narrower.production.contract" />
                </h2>
              </div>
              <Link
                href="/blog"
                className="hidden shrink-0 text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground sm:inline"
              >
                <T id="ui.all.posts" />
              </Link>
            </div>
            <Link
              href={featuredPost.url}
              className="group mt-10 grid gap-8 border border-fd-border transition-colors hover:bg-fd-accent/40 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]"
            >
              {featuredPost.data.cover ? (
                <div className="overflow-hidden bg-fd-muted">
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={featuredPost.data.cover}
                    alt={
                      featuredPost.data.coverAlt ?? featuredPost.data.title
                    }
                    width={1600}
                    height={900}
                    className="aspect-[16/9] w-full object-cover transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] group-hover:scale-[1.02] motion-reduce:transition-none motion-reduce:group-hover:scale-100"
                  />
                </div>
              ) : null}
              <div className="flex flex-col justify-center px-6 py-6 lg:pr-8">
                <p className="font-mono text-xs text-tysel-blue">
                  <T id="ui.release" /> {formatBlogDate(featuredPost, locale)}
                </p>
                <h3 className="font-heading mt-3 text-xl font-medium tracking-tight text-balance sm:text-2xl">
                  {featuredPost.data.title}
                </h3>
                <p className="mt-3 text-sm leading-6 text-fd-muted-foreground">
                  {featuredPost.data.description}
                </p>
                <span className="mt-6 text-sm font-medium">
                  <T id="ui.read.the.announcement" />
                </span>
              </div>
            </Link>
          </div>
        </section>
      ) : null}

      <section>
        <div className="relative overflow-hidden">
          <div className="home-atmosphere pointer-events-none absolute inset-0 opacity-60" />
          <div className="relative mx-auto max-w-6xl px-6 py-24 text-center">
            <h2 className="font-heading text-3xl font-medium tracking-tight text-balance sm:text-5xl">
              <T id="ui.from.typescript.source.to.one.production.artifact" />
            </h2>
            <div className="mt-8 flex justify-center gap-3">
              <Link
                href="/docs/getting-started"
                className="bg-fd-foreground px-4 py-2 text-sm font-medium text-fd-background transition-opacity hover:opacity-90"
              >
                <T id="ui.build.your.first.service" />
              </Link>
              <Link
                href="/docs/guides/examples"
                className="border border-fd-border px-4 py-2 text-sm font-medium transition-colors hover:bg-fd-accent"
              >
                <T id="ui.run.the.durable.agent.example" />
              </Link>
            </div>
          </div>
        </div>
      </section>
    </main>
  );
}
