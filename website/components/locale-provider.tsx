"use client";

import { createContext, useContext, type ComponentProps, type ReactNode } from "react";
import NextLink from "next/link";
import { RootProvider } from "fumadocs-ui/provider/next";
import { sourceLocale, type Locale } from "@/lib/i18n/config";
import { availableLocalePath } from "@/lib/i18n/routing";
import { translateSource, type MessageKey, type Messages } from "@/lib/i18n/catalog";

type LocaleValue = { locale: Locale; messages: Messages; paths: string[] };
const Context = createContext<LocaleValue | null>(null);
export function LocaleProvider({ children, locale, messages, paths, uiTranslations = {} }: LocaleValue & {
  children: ReactNode;
  uiTranslations?: Record<string, string>;
}) {
  return (
    <Context value={{ locale, messages, paths }}>
      <RootProvider
        i18n={{ locale, translations: uiTranslations }}
        search={{ options: { type: "static", api: locale === sourceLocale ? "/api/search" : `/${locale}/search.json` } }}
        theme={{ defaultTheme: "dark", enableSystem: true }}
      >{children}</RootProvider>
    </Context>
  );
}
export function useLocale() {
  const context = useContext(Context);
  if (!context) throw new Error("useLocale requires LocaleProvider");
  return { ...context, t: (id: MessageKey) => context.messages[id] };
}
export function T({ id }: { id: MessageKey }) {
  const { t } = useLocale();
  return <>{t(id)}</>;
}
export function SourceText({ text }: { text: string }) {
  const { messages } = useLocale();
  return <>{translateSource(text, messages)}</>;
}
export function SiteLink({ href, ...props }: ComponentProps<typeof NextLink>) {
  const { locale, paths } = useLocale();
  return <NextLink {...props} href={typeof href === "string" ? availableLocalePath(href, locale, paths) : href} />;
}
