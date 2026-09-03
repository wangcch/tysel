import Link from "next/link";
import type { Metadata } from "next";
import { StructuredData } from "@/components/seo/structured-data";
import {
  formatBlogDate,
  getBlogPosts,
  getReadingMinutes,
} from "@/lib/blog";
import { createBlogIndexJsonLd } from "@/lib/seo";
import { absoluteUrl, canonicalUrl } from "@/lib/shared";

const description =
  "Release notes, runtime design notes, and production guidance from the Tysel team.";

export const metadata: Metadata = {
  title: "Blog",
  description,
  alternates: {
    canonical: canonicalUrl("/blog"),
    types: {
      "application/rss+xml": absoluteUrl("/rss.xml"),
    },
  },
  openGraph: {
    url: canonicalUrl("/blog"),
    title: "Tysel Blog",
    description,
    type: "website",
    images: [absoluteUrl("/opengraph-image")],
  },
};

export default async function BlogIndexPage() {
  const posts = getBlogPosts();
  const [featured, ...rest] = posts;
  const jsonLd = createBlogIndexJsonLd(posts);
  const featuredMinutes = featured
    ? await getReadingMinutes(featured)
    : null;

  return (
    <div>
      <StructuredData data={jsonLd} />

      <section className="relative overflow-hidden border-b border-fd-border">
        <div className="home-atmosphere pointer-events-none absolute inset-0 opacity-80" />
        <div className="home-grid pointer-events-none absolute inset-0" />
        <div className="relative mx-auto max-w-6xl px-6 pt-16 pb-14 sm:pt-20">
          <p className="home-fade-up text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            Blog
          </p>
          <h1 className="home-fade-up font-heading mt-3 max-w-3xl text-4xl font-medium tracking-tighter text-balance sm:text-5xl">
            Notes on shipping TypeScript as a binary.
          </h1>
          <p className="home-fade-up home-fade-up-delay mt-5 max-w-2xl text-base leading-7 text-fd-muted-foreground sm:text-lg">
            {description}
          </p>
          <div className="home-fade-up home-fade-up-delay mt-6">
            <a
              href="/rss.xml"
              className="font-mono text-xs text-tysel-blue transition-colors hover:text-fd-foreground"
            >
              RSS feed →
            </a>
          </div>
        </div>
      </section>

      {featured ? (
        <section className="border-b border-fd-border">
          <div className="mx-auto max-w-6xl px-6 py-12 sm:py-16">
            <Link
              href={featured.url}
              className="group grid gap-8 lg:grid-cols-[minmax(0,1.15fr)_minmax(0,0.85fr)] lg:items-center"
            >
              {featured.data.cover ? (
                <div className="overflow-hidden border border-fd-border bg-fd-muted">
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={featured.data.cover}
                    alt={featured.data.coverAlt ?? featured.data.title}
                    width={1600}
                    height={900}
                    className="aspect-[16/9] w-full object-cover transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] group-hover:scale-[1.02] motion-reduce:transition-none motion-reduce:group-hover:scale-100"
                  />
                </div>
              ) : null}
              <div>
                <p className="font-mono text-xs text-tysel-blue">
                  Featured · {formatBlogDate(featured)} · {featuredMinutes}{" "}
                  min read
                </p>
                <h2 className="font-heading mt-3 text-2xl font-medium tracking-tight text-balance transition-colors group-hover:text-tysel-blue sm:text-3xl">
                  {featured.data.title}
                </h2>
                <p className="mt-4 text-sm leading-7 text-fd-muted-foreground sm:text-base">
                  {featured.data.description}
                </p>
                <span className="mt-6 inline-flex text-sm font-medium text-fd-foreground">
                  Read the post →
                </span>
              </div>
            </Link>
          </div>
        </section>
      ) : null}

      {rest.length > 0 ? (
        <section>
          <div className="mx-auto max-w-6xl px-6 py-12 sm:py-16">
            <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
              Earlier
            </p>
            <ul className="mt-6 divide-y divide-fd-border border-y border-fd-border">
              {rest.map((post) => (
                <li key={post.url}>
                  <Link
                    href={post.url}
                    className="group flex flex-col gap-2 py-6 transition-colors sm:flex-row sm:items-baseline sm:justify-between sm:gap-8"
                  >
                    <div className="min-w-0">
                      <h2 className="text-lg font-medium tracking-tight transition-colors group-hover:text-tysel-blue">
                        {post.data.title}
                      </h2>
                      <p className="mt-2 max-w-2xl text-sm leading-6 text-fd-muted-foreground">
                        {post.data.description}
                      </p>
                    </div>
                    <p className="shrink-0 font-mono text-xs text-fd-muted-foreground">
                      {formatBlogDate(post)}
                    </p>
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        </section>
      ) : null}
    </div>
  );
}
