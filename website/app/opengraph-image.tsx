import { ImageResponse } from "next/og";
import { SocialCard } from "@/components/seo/social-card";

export const dynamic = "force-static";
export const alt = "Tysel — Write TypeScript. Ship a binary.";
export const size = { width: 1200, height: 630 };
export const contentType = "image/png";

export default function OpenGraphImage() {
  return new ImageResponse(<SocialCard />, size);
}
