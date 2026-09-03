import { getBlogDate, getBlogPosts } from "@/lib/blog";
import { getLLMText } from "@/lib/source";
import { absoluteUrl, appName, siteUrl } from "@/lib/shared";

export const revalidate = false;

function escapeXml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

export async function GET() {
  const posts = getBlogPosts();
  const items = await Promise.all(
    posts.map(async (post) => {
      const content = await getLLMText(post);
      const link = absoluteUrl(post.url);
      const date = getBlogDate(post).toUTCString();
      return `    <item>
      <title>${escapeXml(post.data.title)}</title>
      <link>${link}</link>
      <guid isPermaLink="true">${link}</guid>
      <pubDate>${date}</pubDate>
      <description>${escapeXml(post.data.description ?? "")}</description>
      <content:encoded><![CDATA[${content}]]></content:encoded>
    </item>`;
    }),
  );

  const xml = `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>${appName} Blog</title>
    <link>${absoluteUrl("/blog")}</link>
    <description>Release notes, runtime design notes, and production guidance from the Tysel team.</description>
    <language>en</language>
    <lastBuildDate>${new Date().toUTCString()}</lastBuildDate>
    <atom:link href="${absoluteUrl("/rss.xml")}" rel="self" type="application/rss+xml"/>
    <generator>${appName}</generator>
    <image>
      <url>${siteUrl}/brand/tysel-mark.svg</url>
      <title>${appName} Blog</title>
      <link>${absoluteUrl("/blog")}</link>
    </image>
${items.join("\n")}
  </channel>
</rss>
`;

  return new Response(xml, {
    headers: {
      "Content-Type": "application/rss+xml; charset=utf-8",
      "Cache-Control": "public, max-age=3600",
    },
  });
}
