import type { Metadata } from "next";
import { Geist_Mono, Inter, Inter_Tight } from "next/font/google";
import { RootProvider } from "fumadocs-ui/provider/next";
import { SiteHeader } from "@/components/site-header";
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
  metadataBase: new URL("https://tysel.dev"),
  title: {
    default: "Tysel — Write TypeScript. Ship a binary.",
    template: "%s · Tysel",
  },
  description:
    "A native TypeScript runtime for services and agents. One executable, explicit capabilities, and durable work that survives restarts.",
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
        <RootProvider theme={{ defaultTheme: "dark", enableSystem: true }}>
          <SiteHeader />
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
