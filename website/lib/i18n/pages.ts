import { source, referenceSource, blog } from "@/lib/source";
import { localeConfig, publishedLocales, sourceLocale, type Locale } from "./config";
import { translatedDocs, translatedReference, translatedBlog } from "./content";
import { localePath, splitLocale } from "./routing";

export const marketingPaths = ["/", "/examples", "/benchmarks", "/blog"];
export function contentPages(locale: Locale) {
  return locale === sourceLocale
    ? [...source.getPages(), ...referenceSource.getPages(), ...blog.getPages()]
    : [...translatedDocs.getPages(locale), ...translatedReference.getPages(locale), ...translatedBlog.getPages(locale)];
}
export function availablePaths(locale: Locale): string[] {
  if (!localeConfig[locale].published) return [];
  return [...marketingPaths, ...contentPages(locale).map((page) => splitLocale(page.url).pathname)];
}
export function languageChoices() {
  return publishedLocales().map((locale) => ({ locale, name: localeConfig[locale].name, paths: availablePaths(locale) }));
}
export function pageAlternates(pathname: string) {
  const sourcePath = splitLocale(pathname).pathname;
  return Object.fromEntries(publishedLocales().filter((locale) => availablePaths(locale).includes(sourcePath))
    .map((locale) => [locale, localePath(sourcePath, locale)]));
}
