import type { Metadata } from "next";
import { Geist_Mono, Inter, Inter_Tight } from "next/font/google";
import { LocaleProvider } from "@/components/locale-provider";
import { getMessages, getUITranslations } from "@/lib/i18n/messages";
import { type Locale } from "@/lib/i18n/config";
import { languageChoices, availablePaths } from "@/lib/i18n/pages";
import { ScrollToTop } from "@/components/scroll-to-top";
import { SiteHeader } from "@/components/site-header";
import { appName, siteUrl } from "@/lib/shared";
import "@/app/global.css";

/** Body / UI — matches vite.dev (`Inter`). */
const inter = Inter({
  subsets: ["latin"],
  variable: "--font-inter",
  display: "swap",
});

/**
 * Marketing headings — stand-in for vite.dev’s proprietary `APK Protocol`.
 * Inter Tight keeps the Inter family while tightening display metrics.
 */
const interTight = Inter_Tight({
  subsets: ["latin"],
  variable: "--font-inter-tight",
  display: "swap",
});

/** Code — stand-in for vite.dev’s proprietary `KH Teka Mono`. */
const geistMono = Geist_Mono({
  subsets: ["latin"],
  variable: "--font-geist-mono",
  display: "swap",
});

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  applicationName: appName,
  title: {
    default: "Tysel — Write TypeScript. Ship a binary.",
    template: "%s · Tysel",
  },
  description:
    "A native TypeScript runtime for services and agents. One executable, explicit capabilities, and durable work that survives restarts.",
  authors: [{ name: "Tysel contributors", url: "https://github.com/wangcch/tysel" }],
  creator: "Tysel contributors",
  publisher: appName,
  category: "Developer tools",
  openGraph: {
    siteName: appName,
    locale: "en_US",
    type: "website",
    images: [{ url: `${siteUrl}/opengraph-image`, width: 1200, height: 630 }],
  },
  twitter: {
    card: "summary_large_image",
    images: [`${siteUrl}/twitter-image`],
  },
  icons: {
    icon: "/brand/tysel-mark.svg",
  },
};

export function SiteDocument({ children, locale }: { children: React.ReactNode; locale: Locale }) {
  const messages = getMessages(locale);
  return (
    <html
      lang={locale}
      className={`${inter.variable} ${interTight.variable} ${geistMono.variable} dark`}
      suppressHydrationWarning
    >
      <body className="flex min-h-screen flex-col font-sans">
        <LocaleProvider locale={locale} messages={messages} paths={availablePaths(locale)} uiTranslations={getUITranslations(messages)}>
          <ScrollToTop />
          <SiteHeader languages={languageChoices()} />
          {children}
        </LocaleProvider>
      </body>
    </html>
  );
}
