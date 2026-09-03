import type { MetadataRoute } from "next";
import { getBlogPosts, getBlogUpdatedAt } from "@/lib/blog";
import { referenceSource, source } from "@/lib/source";
import { canonicalUrl } from "@/lib/shared";

export const dynamic = "force-static";

const productPages: MetadataRoute.Sitemap = [
  { url: canonicalUrl(), changeFrequency: "weekly", priority: 1 },
  { url: canonicalUrl("/blog"), changeFrequency: "weekly", priority: 0.9 },
  { url: canonicalUrl("/examples"), changeFrequency: "monthly", priority: 0.8 },
  { url: canonicalUrl("/benchmarks"), changeFrequency: "monthly", priority: 0.6 },
];

export default function sitemap(): MetadataRoute.Sitemap {
  const contentPages = [...source.getPages(), ...referenceSource.getPages()].map((page) => ({
    url: canonicalUrl(page.url),
    changeFrequency: "monthly" as const,
    priority: page.url === "/docs" || page.url === "/reference" ? 0.9 : 0.7,
  }));

  const blogPages = getBlogPosts().map((page) => ({
    url: canonicalUrl(page.url),
    lastModified: getBlogUpdatedAt(page),
    changeFrequency: "monthly" as const,
    priority: 0.85,
  }));

  return [...productPages, ...blogPages, ...contentPages];
}
