import { canonicalUrl } from "@/lib/shared";
import { pageAlternates } from "./pages";
export function alternates(pathname = "/") {
  return {
    canonical: canonicalUrl(pathname),
    languages: Object.fromEntries(Object.entries(pageAlternates(pathname)).map(([locale, path]) => [locale, canonicalUrl(path)])),
  };
}
