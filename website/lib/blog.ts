import { estimateReadingMinutes } from "./reading-time.mjs";
import { translatedBlog } from "@/lib/i18n/content";
import { sourceLocale, type Locale } from "@/lib/i18n/config";
import type { BlogPage } from "@/lib/source";
import { blog } from "@/lib/source";

export function getBlogDate(page: BlogPage): Date {
  const value = page.data.date;
  return value instanceof Date ? value : new Date(value);
}

export function getBlogUpdatedAt(page: BlogPage): Date {
  const value = page.data.updatedAt;
  if (!value) return getBlogDate(page);
  return value instanceof Date ? value : new Date(value);
}

export function formatBlogDate(page: BlogPage, locale: Locale = sourceLocale): string {
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  }).format(getBlogDate(page));
}

export function getBlogPosts(locale: Locale = sourceLocale): BlogPage[] {
  const pages = locale === sourceLocale ? blog.getPages() : blog.getPages().flatMap((original) => {
    const translated = translatedBlog.getPage(original.slugs, locale);
    return translated ? [{ ...original, ...translated, data: { ...original.data, ...translated.data } }] : [];
  });
  return pages.sort(
    (a, b) => getBlogDate(b).getTime() - getBlogDate(a).getTime(),
  );
}

export function getFeaturedBlogPost(locale: Locale = sourceLocale): BlogPage | undefined {
  return getBlogPosts(locale)[0];
}

export async function getReadingMinutes(page: BlogPage): Promise<number> {
  if (page.data.readingMinutes) return page.data.readingMinutes;
  return estimateReadingMinutes(await page.data.getText("raw"));
}
