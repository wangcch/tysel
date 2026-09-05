import { notFound } from "next/navigation";
import { createFromSource } from "fumadocs-core/search/server";
import { isLocale, localeConfig, publishedLocales, sourceLocale } from "@/lib/i18n/config";
import { translatedDocs, translatedReference, translatedBlog } from "@/lib/i18n/content";

export const dynamic = "force-static";
export const dynamicParams = false;
export function generateStaticParams() {
  return publishedLocales().filter((lang) => lang !== sourceLocale).map((lang) => ({ lang }));
}
export async function GET(_request: Request, { params }: { params: Promise<{ lang: string }> }) {
  const { lang } = await params;
  if (!isLocale(lang) || !localeConfig[lang].published || lang === sourceLocale) notFound();
  const sources = [translatedDocs, translatedReference, translatedBlog];
  const combined = {
    getPages: () => sources.flatMap((source) => source.getPages(lang)),
    getPageTree: () => ({ name: "Tysel", children: sources.flatMap((source) => source.getPageTree(lang).children) }),
  };
  return createFromSource(combined as unknown as typeof translatedDocs).staticGET();
}
