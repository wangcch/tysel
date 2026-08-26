import type { Metadata } from "next";
import { Geist_Mono, Inter, Inter_Tight } from "next/font/google";
import { RootProvider } from "fumadocs-ui/provider/next";
import { ScrollToTop } from "@/components/scroll-to-top";
import { SiteHeader } from "@/components/site-header";
import { appName, siteUrl } from "@/lib/shared";
import "./global.css";

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
  },
  twitter: {
    card: "summary_large_image",
  },
  icons: {
    icon: "/brand/tysel-mark.svg",
  },
};

export default function Layout({ children }: LayoutProps<"/">) {
  return (
    <html
      lang="en"
      className={`${inter.variable} ${interTight.variable} ${geistMono.variable} dark`}
      suppressHydrationWarning
    >
      <body className="flex min-h-screen flex-col font-sans">
        <RootProvider
          search={{ options: { type: "static", api: "/api/search" } }}
          theme={{ defaultTheme: "dark", enableSystem: true }}
        >
          <ScrollToTop />
          <SiteHeader />
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
