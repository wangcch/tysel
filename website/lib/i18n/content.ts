import { pageSchema } from "fumadocs-core/source/schema";
import { z } from "zod";
import { defineDocs } from "fumadocs-mdx/macro";
import { loader } from "fumadocs-core/source";
import { configuredLocales, sourceLocale } from "./config";

// Build outputs only. The durable translation source lives under locales/<locale>/content.
const docs = defineDocs({ dir: "content/translations/docs", docs: { postprocess: { includeProcessedMarkdown: true } } });
const reference = defineDocs({ dir: "content/translations/reference", docs: { postprocess: { includeProcessedMarkdown: true } } });
const blog = defineDocs({ dir: "content/translations/blog", docs: {
  schema: pageSchema.extend({
    author: z.string(),
    date: z.union([z.string(), z.date()]),
    cover: z.string().optional(),
    coverAlt: z.string().optional(),
    tags: z.array(z.string()).optional(),
    readingMinutes: z.number().int().positive().optional(),
    updatedAt: z.union([z.string(), z.date()]).optional(),
  }),
  postprocess: { includeProcessedMarkdown: true },
} });
const i18n = { languages: configuredLocales, defaultLanguage: sourceLocale, parser: "dir" as const, fallbackLanguage: null };
export const translatedDocs = loader({ baseUrl: "/docs", source: docs.toFumadocsSource(), i18n });
export const translatedReference = loader({ baseUrl: "/reference", source: reference.toFumadocsSource(), i18n });
export const translatedBlog = loader({ baseUrl: "/blog", source: blog.toFumadocsSource(), i18n });
