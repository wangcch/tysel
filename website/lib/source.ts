import { loader } from "fumadocs-core/source";
import { llms } from "fumadocs-core/source/llms";
import { lucideIconsPlugin } from "fumadocs-core/source/lucide-icons";
import {
  docsContentRoute,
  docsImageRoute,
  docsRoute,
  referenceContentRoute,
  referenceImageRoute,
  referenceRoute,
} from "./shared";
import { defineDocs } from "fumadocs-mdx/macro";
import { metaSchema, pageSchema } from "fumadocs-core/source/schema";

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

export type AnySourcePage =
  | ReturnType<typeof source.getPages>[number]
  | ReturnType<typeof referenceSource.getPages>[number];

export function getPageImageUrl(page: AnySourcePage) {
  const isReference = page.url.startsWith(referenceRoute);
  const imageRoute = isReference ? referenceImageRoute : docsImageRoute;
  const segments = [...page.slugs, "image.png"];

  return {
    segments,
    url: "/" + [page.locale, ...imageRoute.split("/"), ...segments].filter(Boolean).join("/"),
  };
}

export function getPageMarkdownUrl(page: AnySourcePage) {
  const isReference = page.url.startsWith(referenceRoute);
  const contentRoute = isReference ? referenceContentRoute : docsContentRoute;
  const segments = [...page.slugs, "content.md"];

  return {
    segments,
    url: "/" + [page.locale, ...contentRoute.split("/"), ...segments].filter(Boolean).join("/"),
  };
}

export async function getLLMText(page: AnySourcePage) {
  const processed = await page.data.getText("processed");
  return `# ${page.data.title} (${page.url})\n\n${processed}`;
}

export { llms };

export function getPageSourcePath(page: AnySourcePage) {
  return page.url.startsWith(referenceRoute)
    ? `website/content/reference/${page.path}`
    : `website/content/docs/${page.path}`;
}
