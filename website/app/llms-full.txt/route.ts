import { getBlogPosts } from "@/lib/blog";
import { getLLMText, referenceSource, source } from "@/lib/source";

export const revalidate = false;

export async function GET() {
  const pages = [...source.getPages(), ...referenceSource.getPages(), ...getBlogPosts()];
  const scanned = await Promise.all(pages.map(getLLMText));
  return new Response(scanned.join("\n\n"));
}
