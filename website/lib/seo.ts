import { sourceLocale, type Locale } from "@/lib/i18n/config";
import { localePath } from "@/lib/i18n/routing";
import { getMessages } from "@/lib/i18n/messages";
import type { AnySourcePage, BlogPage } from "@/lib/source";
import { getPageImageUrl } from "@/lib/source";
import { getBlogDate, getBlogUpdatedAt } from "@/lib/blog";
import { appName, canonicalUrl, githubUrl, siteUrl } from "@/lib/shared";

export function createArticleJsonLd(
  page: AnySourcePage,
  section: { name: string; url: string },
  presentation: { title?: string; description?: string; locale?: Locale } = {},
) {
  const url = canonicalUrl(page.url);
  const locale = presentation.locale ?? sourceLocale;
  const title = presentation.title ?? page.data.title;
  const description = presentation.description ?? page.data.description;
  const breadcrumbs = [
    { name: locale === sourceLocale ? "Home" : getMessages(locale)["nav.home"], url: canonicalUrl(localePath("/", locale)) },
    { name: section.name, url: canonicalUrl(section.url) },
  ];

  if (page.url !== section.url) {
    breadcrumbs.push({ name: title, url });
  }

  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "TechArticle",
        headline: title,
        description,
        url,
        mainEntityOfPage: url,
        image: locale === sourceLocale ? `${siteUrl}${getPageImageUrl(page).url}` : `${siteUrl}/opengraph-image`,
        inLanguage: locale,
        author: {
          "@type": "Organization",
          name: `${appName} contributors`,
          url: githubUrl,
        },
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: breadcrumbs.map((item, index) => ({
          "@type": "ListItem",
          position: index + 1,
          name: item.name,
          item: item.url,
        })),
      },
    ],
  };
}

export function createBlogPostingJsonLd(page: BlogPage, locale: Locale = sourceLocale) {
  const url = canonicalUrl(page.url);
  const image = page.data.cover
    ? page.data.cover.startsWith("http")
      ? page.data.cover
      : `${siteUrl}${page.data.cover}`
    : undefined;
  const datePublished = getBlogDate(page).toISOString();
  const dateModified = getBlogUpdatedAt(page).toISOString();

  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "BlogPosting",
        headline: page.data.title,
        description: page.data.description,
        url,
        mainEntityOfPage: {
          "@type": "WebPage",
          "@id": url,
        },
        image,
        datePublished,
        dateModified,
        inLanguage: locale,
        isPartOf: {
          "@type": "Blog",
          "@id": `${canonicalUrl(localePath("/blog", locale))}#blog`,
          name: `${appName} Blog`,
          url: canonicalUrl(localePath("/blog", locale)),
        },
        author: {
          "@type": "Organization",
          name: page.data.author,
          url: siteUrl,
        },
        publisher: {
          "@type": "Organization",
          name: appName,
          url: siteUrl,
          logo: {
            "@type": "ImageObject",
            url: `${siteUrl}/brand/tysel-mark.svg`,
          },
        },
        keywords: page.data.tags?.join(", "),
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          {
            "@type": "ListItem",
            position: 1,
            name: locale === sourceLocale ? "Home" : getMessages(locale)["nav.home"],
            item: canonicalUrl(localePath("/", locale)),
          },
          {
            "@type": "ListItem",
            position: 2,
            name: getMessages(locale)["nav.blog"],
            item: canonicalUrl(localePath("/blog", locale)),
          },
          {
            "@type": "ListItem",
            position: 3,
            name: page.data.title,
            item: url,
          },
        ],
      },
    ],
  };
}

export function createBlogIndexJsonLd(posts: BlogPage[], locale: Locale = sourceLocale) {
  return {
    "@context": "https://schema.org",
    "@graph": [
      {
        "@type": "Blog",
        "@id": `${canonicalUrl(localePath("/blog", locale))}#blog`,
        name: `${appName} Blog`,
        description:
          getMessages(locale)["blog.description"],
        url: canonicalUrl(localePath("/blog", locale)),
        inLanguage: locale,
        publisher: {
          "@type": "Organization",
          name: appName,
          url: siteUrl,
        },
        blogPost: posts.map((post) => ({
          "@type": "BlogPosting",
          headline: post.data.title,
          url: canonicalUrl(post.url),
          datePublished: getBlogDate(post).toISOString(),
          dateModified: getBlogUpdatedAt(post).toISOString(),
          description: post.data.description,
        })),
      },
      {
        "@type": "BreadcrumbList",
        itemListElement: [
          {
            "@type": "ListItem",
            position: 1,
            name: locale === sourceLocale ? "Home" : getMessages(locale)["nav.home"],
            item: canonicalUrl(localePath("/", locale)),
          },
          {
            "@type": "ListItem",
            position: 2,
            name: getMessages(locale)["nav.blog"],
            item: canonicalUrl(localePath("/blog", locale)),
          },
        ],
      },
    ],
  };
}
