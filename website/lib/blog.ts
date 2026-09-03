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

export function formatBlogDate(page: BlogPage): string {
  return new Intl.DateTimeFormat("en", {
    year: "numeric",
    month: "long",
    day: "numeric",
    timeZone: "UTC",
  }).format(getBlogDate(page));
}

export function getBlogPosts(): BlogPage[] {
  return [...blog.getPages()].sort(
    (a, b) => getBlogDate(b).getTime() - getBlogDate(a).getTime(),
  );
}

export function getFeaturedBlogPost(): BlogPage | undefined {
  return getBlogPosts()[0];
}

/** Words per minute for reading-time estimates. */
const WORDS_PER_MINUTE = 220;

export async function getReadingMinutes(page: BlogPage): Promise<number> {
  if (page.data.readingMinutes) return page.data.readingMinutes;

  const raw = await page.data.getText("raw");
  const body = raw.startsWith("---")
    ? raw.slice(raw.indexOf("\n---", 3) + 4)
    : raw;
  const words = body
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/!\[[^\]]*]\([^)]+\)/g, " ")
    .replace(/\[[^\]]*]\([^)]+\)/g, " ")
    .replace(/[#>*`|_~\-={}]/g, " ")
    .split(/\s+/)
    .filter(Boolean).length;

  return Math.max(1, Math.round(words / WORDS_PER_MINUTE));
}
