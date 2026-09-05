import { ImageResponse } from "next/og";
import { SocialCard } from "@/components/seo/social-card";

export const dynamic = "force-static";
export function GET() {
  return new ImageResponse(<SocialCard />, { width: 1200, height: 630 });
}
