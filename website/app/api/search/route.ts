import { referenceSource, source } from "@/lib/source";
import { createFromSource } from "fumadocs-core/search/server";

const docsSearch = createFromSource(source);
const referenceSearch = createFromSource(referenceSource);

function readOptions(url: URL) {
  const limit = url.searchParams.has("limit") ? Number(url.searchParams.get("limit")) : undefined;

  return {
    tag: url.searchParams.get("tag")?.split(","),
    locale: url.searchParams.get("locale"),
    limit: Number.isInteger(limit) ? limit : undefined,
    mode: url.searchParams.get("mode") === "vector" ? ("vector" as const) : ("full" as const),
  };
}

export async function GET(request: Request) {
  const url = new URL(request.url);
  const query = url.searchParams.get("query");
  if (!query) return Response.json([]);

  const options = readOptions(url);
  const [docsResults, referenceResults] = await Promise.all([
    docsSearch.search(query, options),
    referenceSearch.search(query, options),
  ]);

  const combined = [...docsResults, ...referenceResults].slice(0, options.limit ?? 20);

  return Response.json(combined);
}
