import config from "../../locales/config.json";

export type Locale = keyof typeof config.locales;
export const sourceLocale = config.sourceLocale as Locale;
export const localeConfig = config.locales;
export const configuredLocales = Object.keys(localeConfig) as Locale[];
export function isLocale(value: string): value is Locale {
  return Object.hasOwn(localeConfig, value);
}
export function publishedLocales(): Locale[] {
  return configuredLocales.filter((locale) => localeConfig[locale].published);
}
