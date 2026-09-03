import { getBlogPosts } from "@/lib/blog";
import { absoluteUrl } from "@/lib/shared";
import { llms, referenceSource, source } from "@/lib/source";

export const revalidate = false;

export function GET() {
  const docsIndex = llms(source).index();
  const referenceIndex = llms(referenceSource).index().replace(/^# Reference$/m, "## Reference");
  const blogPosts = getBlogPosts()
    .map((post) => `- [${post.data.title}](${absoluteUrl(post.url)}): ${post.data.description ?? ""}`)
    .join("\n");
  const blogIndex = `# Blog\n\n${blogPosts || "- No posts yet."}`;
  return new Response(`${docsIndex}\n\n${referenceIndex}\n\n${blogIndex}`);
}
