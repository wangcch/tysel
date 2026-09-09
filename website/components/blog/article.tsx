import { T } from "@/components/locale-provider";
import { SiteLink as Link } from "@/components/locale-provider";
import { LocaleOriginalLink } from "@/components/locale-original-link";
import { BlogTable } from "@/components/blog/blog-table";
import type { Metadata } from "next";
import { BlogMobileToc } from "@/components/blog/mobile-toc";
import { getMDXComponents } from "@/components/mdx";
import { StructuredData } from "@/components/seo/structured-data";
import {
  formatBlogDate,
  getBlogDate,
  getBlogUpdatedAt,
  getReadingMinutes,
} from "@/lib/blog";
import { createBlogPostingJsonLd } from "@/lib/seo";
import type { BlogPage } from "@/lib/source";
import type { MDXComponents } from "mdx/types";
import { sourceLocale, type Locale } from "@/lib/i18n/config";
import { getMessages } from "@/lib/i18n/messages";
import { absoluteUrl, canonicalUrl, githubUrl } from "@/lib/shared";

function blogMdxComponents(components?: MDXComponents): MDXComponents {
  return getMDXComponents({ ...components, table: BlogTable });
}

export default async function BlogArticle({ page, locale = sourceLocale, components, originalUrl }: { page: BlogPage; locale?: Locale; components?: MDXComponents; originalUrl?: string }) {
  const MDX = page.data.body;
  const jsonLd = createBlogPostingJsonLd(page, locale);
  const minutes = await getReadingMinutes(page);
  const mdxComponents = blogMdxComponents(components);

  return (
    <>
      <StructuredData data={jsonLd} />
      <div>
        <article>
          <header className="relative overflow-hidden border-b border-fd-border">
            <div className="home-atmosphere pointer-events-none absolute inset-0 opacity-70" />
            <div className="relative mx-auto max-w-3xl px-6 pt-14 pb-10 sm:pt-16 sm:pb-12">
              <nav className="home-fade-up text-sm text-fd-muted-foreground">
                <Link
                  href="/blog"
                  className="transition-colors hover:text-fd-foreground"
                >
                  <T id="nav.blog" />
                </Link>
                <span className="mx-2 text-fd-border">/</span>
                <span className="text-fd-foreground"><T id="ui.post" /></span>
              </nav>

              <p className="home-fade-up mt-8 font-mono text-xs text-tysel-blue">
                {formatBlogDate(page, locale)} · {minutes} <T id="ui.min.read" /> {page.data.author}
              </p>

              <h1 className="home-fade-up font-heading mt-4 text-3xl font-medium tracking-tighter text-balance sm:text-5xl sm:leading-[1.08]">
                {page.data.title}
              </h1>

              {page.data.description ? (
                <p className="home-fade-up home-fade-up-delay mt-5 text-base leading-7 text-fd-muted-foreground sm:text-lg">
                  {page.data.description}
                </p>
              ) : null}

              {originalUrl ? (
                <p className="home-fade-up home-fade-up-delay mt-4">
                  <LocaleOriginalLink href={originalUrl} label={getMessages(locale)["locale.original"]} />
                </p>
              ) : null}

              {page.data.tags && page.data.tags.length > 0 ? (
                <ul className="home-fade-up home-fade-up-delay mt-6 flex flex-wrap gap-2">
                  {page.data.tags.map((tag) => (
                    <li
                      key={tag}
                      className="border border-fd-border px-2 py-0.5 font-mono text-[11px] uppercase tracking-[0.12em] text-fd-muted-foreground"
                    >
                      {tag}
                    </li>
                  ))}
                </ul>
              ) : null}
            </div>

            {page.data.cover ? (
              <div className="home-fade-up home-fade-up-delay border-t border-fd-border bg-fd-muted">
                <div className="mx-auto max-w-5xl">
                  {/* eslint-disable-next-line @next/next/no-img-element */}
                  <img
                    src={page.data.cover}
                    alt={page.data.coverAlt ?? page.data.title}
                    width={1600}
                    height={900}
                    className="aspect-[16/9] w-full object-cover"
                  />
                </div>
              </div>
            ) : null}
          </header>

          <div className="mx-auto grid max-w-6xl gap-10 px-6 py-12 lg:grid-cols-[minmax(0,42rem)_1fr] lg:justify-center lg:py-16 xl:grid-cols-[minmax(0,42rem)_14rem] xl:justify-between">
            <div className="min-w-0 lg:justify-self-end">
              <BlogMobileToc items={page.data.toc} />
              <div className="blog-prose prose min-w-0 max-w-[72ch]">
                <MDX components={mdxComponents} />
              </div>
            </div>

            <aside className="hidden xl:block">
              <div className="sticky top-[calc(var(--site-header-height)+1.5rem)] space-y-8">
                <div>
                  <p className="text-xs font-medium uppercase tracking-[0.14em] text-fd-muted-foreground">
                    <T id="ui.on.this.page" />
                  </p>
                  <nav className="mt-3 space-y-2">
                    {page.data.toc
                      .filter((item) => item.depth <= 2)
                      .map((item) => (
                        <a
                          key={item.url}
                          href={item.url}
                          className="block text-sm leading-5 text-fd-muted-foreground transition-colors hover:text-fd-foreground"
                        >
                          {item.title}
                        </a>
                      ))}
                  </nav>
                </div>
              </div>
            </aside>
          </div>

          <footer className="border-t border-fd-border">
            <div className="mx-auto grid max-w-6xl gap-px bg-fd-border sm:grid-cols-3">
              <Link
                href="/docs/getting-started"
                className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-xs text-tysel-blue"><T id="ui.next" /></p>
                <p className="mt-2 text-sm font-medium"><T id="ui.get.started" /></p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  <T id="ui.install.the.toolchain.and.ship.a.hello.service" />
                </p>
              </Link>
              <Link
                href="/docs"
                className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-xs text-tysel-blue"><T id="nav.docs" /></p>
                <p className="mt-2 text-sm font-medium"><T id="ui.read.the.docs.186" /></p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  <T id="ui.concepts.capabilities.durable.work.and.operations" />
                </p>
              </Link>
              <Link
                href={githubUrl}
                target="_blank"
                rel="noreferrer"
                className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-xs text-tysel-blue"><T id="ui.source" /></p>
                <p className="mt-2 text-sm font-medium">GitHub →</p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  <T id="ui.browse.the.runtime.examples.and.release.evidence" />
                </p>
              </Link>
            </div>

            <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-6 py-8">
              <Link
                href="/blog"
                className="text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground"
              >
                <T id="ui.all.posts.189" />
              </Link>
              <a
                href="/rss.xml"
                className="font-mono text-xs text-tysel-blue transition-colors hover:text-fd-foreground"
              >
                RSS
              </a>
            </div>
          </footer>
        </article>
      </div>
    </>
  );
}

export function blogMetadata(page: BlogPage): Metadata {
  const url = canonicalUrl(page.url);
  const published = getBlogDate(page).toISOString();
  const modified = getBlogUpdatedAt(page).toISOString();
  const cover = page.data.cover
    ? page.data.cover.startsWith("http")
      ? page.data.cover
      : absoluteUrl(page.data.cover)
    : absoluteUrl("/opengraph-image");

  return {
    title: page.data.title,
    description: page.data.description,
    authors: [{ name: page.data.author, url: absoluteUrl() }],
    alternates: {
      canonical: url,
      types: {
        "application/rss+xml": absoluteUrl("/rss.xml"),
      },
    },
    openGraph: {
      type: "article",
      url,
      title: page.data.title,
      description: page.data.description,
      publishedTime: published,
      modifiedTime: modified,
      authors: [page.data.author],
      tags: page.data.tags,
      images: [
        {
          url: cover,
          width: 1600,
          height: 900,
          alt: page.data.coverAlt ?? page.data.title,
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title: page.data.title,
      description: page.data.description,
      images: [cover],
    },
  };
}
