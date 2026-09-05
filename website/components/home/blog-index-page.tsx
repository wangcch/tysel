import { sourceLocale, type Locale } from "@/lib/i18n/config";
import { alternates } from "@/lib/i18n/seo";
import { T, SiteLink as Link } from "@/components/locale-provider";
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
    ...alternates("/blog"),
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

export default async function BlogIndexPage({ locale = sourceLocale }: { locale?: Locale } = {}) {
  const posts = getBlogPosts(locale);
  const [featured, ...rest] = posts;
  const jsonLd = createBlogIndexJsonLd(posts, locale);
  const featuredMinutes = featured
    ? await getReadingMinutes(featured)
    : null;
  const restWithMinutes = await Promise.all(
    rest.map(async (post) => ({ post, minutes: await getReadingMinutes(post) })),
  );

  return (
    <div>
      <StructuredData data={jsonLd} />

      <section className="relative overflow-hidden border-b border-fd-border">
        <div className="home-atmosphere pointer-events-none absolute inset-0 opacity-80" />
        <div className="home-grid pointer-events-none absolute inset-0" />
        <div className="relative mx-auto max-w-6xl px-6 pt-16 pb-14 sm:pt-20">
          <p className="home-fade-up text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
            <T id="nav.blog" />
          </p>
          <h1 className="home-fade-up font-heading mt-3 max-w-3xl text-4xl font-medium tracking-tighter text-balance sm:text-5xl">
            <T id="ui.notes.on.shipping.typescript.as.a.binary" />
          </h1>
          <p className="home-fade-up home-fade-up-delay mt-5 max-w-2xl text-base leading-7 text-fd-muted-foreground sm:text-lg">
            <T id="blog.description" />
          </p>
          <div className="home-fade-up home-fade-up-delay mt-6">
            <a
              href="/rss.xml"
              className="font-mono text-xs text-tysel-blue transition-colors hover:text-fd-foreground"
            >
              <T id="ui.rss.feed" />
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
                  <T id="ui.featured" /> {formatBlogDate(featured, locale)} · {featuredMinutes}{" "}
                  <T id="ui.min.read.194" />
                </p>
                <h2 className="font-heading mt-3 text-2xl font-medium tracking-tight text-balance transition-colors group-hover:text-tysel-blue sm:text-3xl">
                  {featured.data.title}
                </h2>
                <p className="mt-4 text-sm leading-7 text-fd-muted-foreground sm:text-base">
                  {featured.data.description}
                </p>
                <span className="mt-6 inline-flex text-sm font-medium text-fd-foreground">
                  <T id="ui.read.the.post" />
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
              <T id="ui.earlier" />
            </p>
            <ul className="mt-6 divide-y divide-fd-border border-y border-fd-border">
              {restWithMinutes.map(({ post, minutes }) => (
                <li key={post.url}>
                  <Link
                    href={post.url}
                    className="group flex items-stretch gap-5 py-6 sm:gap-7"
                  >
                    {/* Thumbnail — stretches to match row height */}
                    {post.data.cover ? (
                      <div className="hidden w-[160px] shrink-0 overflow-hidden border border-fd-border bg-fd-muted sm:block">
                        {/* eslint-disable-next-line @next/next/no-img-element */}
                        <img
                          src={post.data.cover}
                          alt={post.data.coverAlt ?? post.data.title}
                          width={320}
                          height={180}
                          className="h-full w-full object-cover transition-transform duration-500 ease-[cubic-bezier(0.22,1,0.36,1)] group-hover:scale-[1.04] motion-reduce:transition-none motion-reduce:group-hover:scale-100"
                        />
                      </div>
                    ) : (
                      <div className="hidden w-[160px] shrink-0 border border-fd-border bg-fd-muted sm:block" />
                    )}

                    {/* Text */}
                    <div className="min-w-0 flex-1 py-0.5">
                      <p className="font-mono text-xs text-fd-muted-foreground">
                        {formatBlogDate(post, locale)} · {minutes} <T id="ui.min.read.194" />
                      </p>
                      <h2 className="mt-2 text-base font-medium tracking-tight leading-snug transition-colors group-hover:text-tysel-blue sm:text-lg">
                        {post.data.title}
                      </h2>
                      <p className="mt-2 line-clamp-2 text-sm leading-6 text-fd-muted-foreground">
                        {post.data.description}
                      </p>
                    </div>

                    {/* Hover arrow */}
                    <span className="hidden shrink-0 self-center font-mono text-xs text-fd-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 sm:block">
                      →
                    </span>
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
