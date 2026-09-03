import Link from "next/link";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
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
import { blog } from "@/lib/source";
import { absoluteUrl, canonicalUrl, githubUrl } from "@/lib/shared";

export default async function BlogPostPage(
  props: PageProps<"/blog/[slug]">,
) {
  const params = await props.params;
  const page = blog.getPage([params.slug]);
  if (!page) notFound();

  const MDX = page.data.body;
  const jsonLd = createBlogPostingJsonLd(page);
  const minutes = await getReadingMinutes(page);

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
                  Blog
                </Link>
                <span className="mx-2 text-fd-border">/</span>
                <span className="text-fd-foreground">Post</span>
              </nav>

              <p className="home-fade-up mt-8 font-mono text-xs text-tysel-blue">
                {formatBlogDate(page)} · {minutes} min read · {page.data.author}
              </p>

              <h1 className="home-fade-up font-heading mt-4 text-3xl font-medium tracking-tighter text-balance sm:text-5xl sm:leading-[1.08]">
                {page.data.title}
              </h1>

              {page.data.description ? (
                <p className="home-fade-up home-fade-up-delay mt-5 text-base leading-7 text-fd-muted-foreground sm:text-lg">
                  {page.data.description}
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
                <MDX components={getMDXComponents()} />
              </div>
            </div>

            <aside className="hidden xl:block">
              <div className="sticky top-[calc(var(--site-header-height)+1.5rem)] space-y-8">
                <div>
                  <p className="text-xs font-medium uppercase tracking-[0.14em] text-fd-muted-foreground">
                    On this page
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
                <p className="font-mono text-xs text-tysel-blue">Next</p>
                <p className="mt-2 text-sm font-medium">Get started →</p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  Install the toolchain and ship a hello-service binary.
                </p>
              </Link>
              <Link
                href="/docs"
                className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-xs text-tysel-blue">Docs</p>
                <p className="mt-2 text-sm font-medium">Read the docs →</p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  Concepts, capabilities, durable work, and operations.
                </p>
              </Link>
              <Link
                href={githubUrl}
                target="_blank"
                rel="noreferrer"
                className="bg-fd-background px-6 py-6 transition-colors hover:bg-fd-accent"
              >
                <p className="font-mono text-xs text-tysel-blue">Source</p>
                <p className="mt-2 text-sm font-medium">GitHub →</p>
                <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
                  Browse the runtime, examples, and release evidence.
                </p>
              </Link>
            </div>

            <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-6 py-8">
              <Link
                href="/blog"
                className="text-sm text-fd-muted-foreground transition-colors hover:text-fd-foreground"
              >
                ← All posts
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

export function generateStaticParams() {
  return blog.getPages().map((page) => ({
    slug: page.slugs[0],
  }));
}

export async function generateMetadata(
  props: PageProps<"/blog/[slug]">,
): Promise<Metadata> {
  const params = await props.params;
  const page = blog.getPage([params.slug]);
  if (!page) notFound();

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
