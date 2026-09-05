import { LocaleOriginalLink } from "@/components/locale-original-link";
import { StructuredData } from "@/components/seo/structured-data";
import { createArticleJsonLd } from "@/lib/seo";
import BlogArticle, { blogMetadata } from "@/components/blog/article";
import { getBlogPosts } from "@/lib/blog";
import { source, referenceSource, blog } from "@/lib/source";
import type { Metadata } from "next";
import { notFound } from "next/navigation";
import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { DocsPage, DocsTitle, DocsDescription, DocsBody } from "fumadocs-ui/layouts/docs/page";
import HomePage from "@/components/home/home-page";
import ExamplesPage from "@/app/(site)/(home)/examples/page";
import BenchmarksPage from "@/app/(site)/(home)/benchmarks/page";
import BlogIndexPage from "@/components/home/blog-index-page";
import HomeLayout from "@/app/(site)/(home)/layout";
import { getMDXComponents } from "@/components/mdx";
import { SiteLink } from "@/components/locale-provider";
import { baseOptions } from "@/lib/layout.shared";
import { isLocale, localeConfig, publishedLocales, sourceLocale } from "@/lib/i18n/config";
import { availablePaths, pageAlternates } from "@/lib/i18n/pages";
import { localePath } from "@/lib/i18n/routing";
import { getMessages } from "@/lib/i18n/messages";
import { translatedDocs, translatedReference, translatedBlog } from "@/lib/i18n/content";
import { absoluteUrl, canonicalUrl } from "@/lib/shared";

export const dynamic = "force-static";
export const dynamicParams = false;
type Props = { params: Promise<{ lang: string; path?: string[] }> };
export function generateStaticParams() {
  return publishedLocales().filter((locale) => locale !== sourceLocale).flatMap((lang) =>
    availablePaths(lang).map((pathname) => ({ lang, path: pathname.split("/").filter(Boolean) })));
}
async function resolve(props: Props) {
  const { lang, path = [] } = await props.params;
  if (!isLocale(lang) || lang === sourceLocale || !localeConfig[lang].published) notFound();
  const pathname = `/${path.join("/")}`;
  if (!availablePaths(lang).includes(pathname)) notFound();
  const collection = path[0] === "docs" ? translatedDocs : path[0] === "reference" ? translatedReference : translatedBlog;
  const page = path.length > 1 || path[0] === "docs" || path[0] === "reference" ? collection.getPage(path.slice(1), lang) : undefined;
  return { lang, path, pathname, collection, page };
}
export default async function Page(props: Props) {
  const { lang, pathname, collection, page } = await resolve(props);
  if (page) {
    const MDX = page.data.body;
    const originalCollection = pathname.startsWith("/docs") ? source : pathname.startsWith("/reference") ? referenceSource : blog;
    const original = originalCollection.getPage(page.slugs);
    const aliases = new Map<string, string>();
    if (!original || original.data.toc.length !== page.data.toc.length) throw new Error(`Translated heading structure differs: ${pathname}`);
    page.data.toc.forEach((item, index) => {
      const sourceItem = original.data.toc[index];
      if (item.depth !== sourceItem.depth) throw new Error(`Translated heading depth differs: ${pathname}`);
      if (item.url !== sourceItem.url) aliases.set(item.url.slice(1), sourceItem.url.slice(1));
    });
    const components = getMDXComponents();
    const headings = Object.fromEntries((["h1", "h2", "h3", "h4", "h5", "h6"] as const).map((tag) => {
      const Heading = (components[tag] ?? tag) as React.ElementType<React.ComponentProps<"h2">>;
      return [tag, ({ id, children, ...props }: React.ComponentProps<"h2">) => <Heading {...props} id={id}>
        {id && aliases.has(id) && <span id={aliases.get(id)} className="scroll-mt-28" aria-hidden="true" />}
        {children}
      </Heading>];
    }));
    if (pathname.startsWith("/blog/")) {
      const post = getBlogPosts(lang).find(item => item.url === page.url);
      if (!post) notFound();
      return <HomeLayout params={Promise.resolve({})}><BlogArticle page={post} locale={lang} originalUrl={localePath(pathname, sourceLocale)} components={getMDXComponents({ ...headings, a: ({ href = "", ...props }) => <SiteLink {...props} href={href} /> })} /></HomeLayout>;
    }
    return <DocsLayout tree={collection.getPageTree(lang)} {...baseOptions()} nav={{ enabled: false }}>
      <StructuredData data={createArticleJsonLd(page, { name: getMessages(lang)[pathname.startsWith("/docs") ? "nav.docs" : "nav.reference"], url: localePath(pathname.startsWith("/docs") ? "/docs" : "/reference", lang) }, { locale: lang })} />
      <DocsPage toc={page.data.toc}>
        <DocsTitle>{page.data.title}</DocsTitle>
        <DocsDescription className="mb-0">{page.data.description}</DocsDescription>
        <div className="flex flex-row items-center gap-2 border-b pb-6">
          <LocaleOriginalLink href={localePath(pathname, sourceLocale)} label={getMessages(lang)["locale.original"]} />
        </div>
        <DocsBody><MDX components={getMDXComponents({ ...headings, a: ({ href = "", ...props }) => <SiteLink {...props} href={(collection as typeof translatedDocs).resolveHref(href, page)} /> })} /></DocsBody>
      </DocsPage>
    </DocsLayout>;
  }
  const content = pathname === "/" ? <HomePage locale={lang} /> : pathname === "/examples" ? <ExamplesPage />
    : pathname === "/benchmarks" ? <BenchmarksPage /> : <BlogIndexPage locale={lang} />;
  return <HomeLayout params={Promise.resolve({})}>{content}</HomeLayout>;
}
export async function generateMetadata(props: Props): Promise<Metadata> {
  const { lang, pathname, page } = await resolve(props);
  const messages = getMessages(lang);
  const title = page?.data.title ?? (pathname === "/" ? messages["site.title"] : messages[`nav.${pathname.slice(1)}` as "nav.examples" | "nav.benchmarks" | "nav.blog"]);
  const url = canonicalUrl(localePath(pathname, lang));
  if (pathname.startsWith("/blog/")) {
    const post = getBlogPosts(lang).find(item => item.url === page?.url);
    if (!post) notFound();
    const metadata = blogMetadata(post);
    return { ...metadata, alternates: { ...metadata.alternates, canonical: url, languages: Object.fromEntries(Object.entries(pageAlternates(pathname)).map(([locale, path]) => [locale, canonicalUrl(path)])) }, openGraph: { ...metadata.openGraph, locale: localeConfig[lang].ogLocale } };
  }
  return { title, description: page?.data.description ?? messages["site.description"],
    alternates: { canonical: url, languages: Object.fromEntries(Object.entries(pageAlternates(pathname)).map(([locale, path]) => [locale, canonicalUrl(path)])) },
    openGraph: { url, title, description: page?.data.description ?? messages["site.description"], locale: localeConfig[lang].ogLocale, images: [absoluteUrl("/opengraph-image")] },
  };
}
