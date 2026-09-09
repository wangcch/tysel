import { configuredLocales, sourceLocale, type Locale } from "./config";

/** Asset, feed, search, install, and external URLs are language-neutral. */
export function splitLocale(path: string): { locale: Locale; pathname: string } {
  for (const locale of configuredLocales) {
    if (path === `/${locale}` || path.startsWith(`/${locale}/`)) {
      return { locale, pathname: path.slice(locale.length + 1) || "/" };
    }
  }
  return { locale: sourceLocale, pathname: path };
}

export function localePath(path: string, locale: Locale): string {
  if (!path.startsWith("/") || path.startsWith("//")) return path;
  const match = path.match(/^([^?#]*)(.*)$/)!;
  const pathname = splitLocale(match[1]).pathname.replace(/\/$/, "") || "/";
  const suffix = match[2];
  return (locale === sourceLocale ? pathname : `/${locale}${pathname === "/" ? "" : pathname}`) + suffix;
}

/** Only link to pages that actually exist. Missing translations link to English. */
export function availableLocalePath(path: string, locale: Locale, available: readonly string[]): string {
  if (!path.startsWith("/") || path.startsWith("//")) return path;
  const sourcePath = localePath(path, sourceLocale);
  const pathname = sourcePath.split(/[?#]/, 1)[0];
  return available.includes(pathname) ? localePath(sourcePath, locale) : sourcePath;
}
