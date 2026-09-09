import { notFound } from "next/navigation";
import { SiteDocument, metadata } from "@/components/site-document";
import { isLocale, localeConfig, sourceLocale } from "@/lib/i18n/config";
export { metadata };
export default async function Layout({ children, params }: {
  children: React.ReactNode; params: Promise<{ lang: string }>;
}) {
  const { lang } = await params;
  if (!isLocale(lang) || lang === sourceLocale || !localeConfig[lang].published) notFound();
  return <SiteDocument locale={lang}>{children}</SiteDocument>;
}
