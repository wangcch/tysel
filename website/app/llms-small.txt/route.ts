import { referenceSource, source } from "@/lib/source";

export const revalidate = false;

const selectedUrls = new Set([
  "/docs",
  "/docs/getting-started",
  "/docs/guides",
  "/docs/security",
  "/docs/operations/deployment",
  "/docs/operations/production",
  "/reference",
]);

export function GET() {
  const order = [...selectedUrls];
  const pages = [...source.getPages(), ...referenceSource.getPages()]
    .filter((page) => selectedUrls.has(page.url))
    .sort((left, right) => order.indexOf(left.url) - order.indexOf(right.url));

  const entries = pages.map(
    (page) => "- [" + page.data.title + "](" + page.url + "): " + page.data.description,
  );

  return new Response(
    [
      "# Tysel",
      "",
      "> Write TypeScript. Ship a binary.",
      "",
      "Tysel is Web-API-first, not a general Node.js compatibility layer.",
      "",
      ...entries,
      "",
    ].join("\n"),
  );
}
