import type { AnySourcePage } from "@/lib/source";
import { getPageImageUrl } from "@/lib/source";
import { appName, canonicalUrl, githubUrl, siteUrl } from "@/lib/shared";

export function createArticleJsonLd(
  page: AnySourcePage,
  section: { name: string; url: string },
  presentation: { title?: string; description?: string } = {},
) {
  const url = canonicalUrl(page.url);
  const title = presentation.title ?? page.data.title;
  const description = presentation.description ?? page.data.description;
  const breadcrumbs = [
    { name: "Home", url: canonicalUrl() },
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
        image: `${siteUrl}${getPageImageUrl(page).url}`,
        inLanguage: "en",
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
