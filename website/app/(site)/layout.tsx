import { SiteDocument, metadata } from "@/components/site-document";
export { metadata };
export default function Layout({ children }: { children: React.ReactNode }) {
  return <SiteDocument locale="en">{children}</SiteDocument>;
}
