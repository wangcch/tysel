import fs from "node:fs";
import path from "node:path";
import en from "../../locales/en/messages.json";
import { sourceLocale, isLocale, type Locale } from "./config";
import type { Messages } from "./catalog";

/** Build/server only: target dictionaries never enter every locale's client bundle. */
export function getMessages(locale: Locale): Messages {
  if (!isLocale(locale)) throw new Error(`Unknown locale: ${locale}`);
  if (locale === sourceLocale) return en;
  const translated = JSON.parse(fs.readFileSync(path.join(process.cwd(), "locales", locale, "messages.json"), "utf8"));
  return { ...en, ...translated };
}
export function getUITranslations(messages: Messages): Record<string, string> {
  return Object.fromEntries(Object.entries(messages).filter(([key]) => key.startsWith("fumadocs.")).map(([key, value]) => [key.slice(9), value]));
}
