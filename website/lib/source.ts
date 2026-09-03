import { loader } from "fumadocs-core/source";
import { llms } from "fumadocs-core/source/llms";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import {
  blogContentRoute,
  blogRoute,
  docsContentRoute,
  docsImageRoute,
  docsRoute,
  referenceContentRoute,
  referenceImageRoute,
  referenceRoute,
} from "./shared";
import { defineCollections, defineDocs } from "fumadocs-mdx/macro";
import { metaSchema, pageSchema } from "fumadocs-core/source/schema";
import { z } from "zod";

const docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: pageSchema,
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

const reference = defineDocs({
  dir: "content/reference",
  docs: {
    schema: pageSchema,
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
  meta: {
    schema: metaSchema,
  },
});

const blogPosts = defineCollections({
  type: "doc",
  dir: "content/blog",
  schema: pageSchema.extend({
    author: z.string(),
    date: z.union([z.string(), z.date()]),
    cover: z.string().optional(),
    coverAlt: z.string().optional(),
    tags: z.array(z.string()).optional(),
    readingMinutes: z.number().int().positive().optional(),
    updatedAt: z.union([z.string(), z.date()]).optional(),
  }),
  postprocess: {
    includeProcessedMarkdown: true,
  },
});

export const source = loader({
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
  plugins: [lucideIconsPlugin()],
});

export const referenceSource = loader({
  baseUrl: referenceRoute,
  source: reference.toFumadocsSource(),
  plugins: [lucideIconsPlugin()],
});

export const blog = loader({
  baseUrl: blogRoute,
  source: blogPosts.toFumadocsSource(),
});

export type DocsPage = ReturnType<typeof source.getPages>[number];
export type ReferencePage = ReturnType<typeof referenceSource.getPages>[number];
export type BlogPage = ReturnType<typeof blog.getPages>[number];
export type AnySourcePage = DocsPage | ReferencePage;

export function getPageImageUrl(page: AnySourcePage) {
  const isReference = page.url.startsWith(referenceRoute);
  const imageRoute = isReference ? referenceImageRoute : docsImageRoute;
  const segments = [...page.slugs, "image.png"];

  return {
    segments,
    url: "/" + [page.locale, ...imageRoute.split("/"), ...segments].filter(Boolean).join("/"),
  };
}

export function getPageMarkdownUrl(page: AnySourcePage | BlogPage) {
  if (page.url.startsWith(blogRoute)) {
    const segments = [...page.slugs, "content.md"];
    return {
      segments,
      url: "/" + [page.locale, ...blogContentRoute.split("/"), ...segments].filter(Boolean).join("/"),
    };
  }

  const isReference = page.url.startsWith(referenceRoute);
  const contentRoute = isReference ? referenceContentRoute : docsContentRoute;
  const segments = [...page.slugs, "content.md"];

  return {
    segments,
    url: "/" + [page.locale, ...contentRoute.split("/"), ...segments].filter(Boolean).join("/"),
  };
}

/** Strip YAML frontmatter so RSS / LLM feeds get readable Markdown body. */
function stripFrontmatter(source: string): string {
  if (!source.startsWith("---")) return source.trim();
  const end = source.indexOf("\n---", 3);
  if (end === -1) return source.trim();
  return source.slice(end + 4).replace(/^\r?\n+/, "").trim();
}

/**
 * Prefer filesystem source over `processed` Markdown.
 * Processed output currently escapes emphasis markers and replaces images with
 * internal placeholders (`__img0`), which breaks RSS and LLM feeds.
 */
export async function getLLMText(page: AnySourcePage | BlogPage) {
  const raw = await page.data.getText("raw");
  return `# ${page.data.title} (${page.url})\n\n${stripFrontmatter(raw)}`;
}

export { llms };

export function getPageSourcePath(page: AnySourcePage | BlogPage) {
  if (page.url.startsWith(blogRoute)) {
    return `website/content/blog/${page.path}`;
  }

  return page.url.startsWith(referenceRoute)
    ? `website/content/reference/${page.path}`
    : `website/content/docs/${page.path}`;
}
