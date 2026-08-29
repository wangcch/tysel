import { llms, referenceSource, source } from "@/lib/source";

export const revalidate = false;

export function GET() {
  const docsIndex = llms(source).index();
  const referenceIndex = llms(referenceSource).index().replace(/^# Reference$/m, "## Reference");
  return new Response(`${docsIndex}\n\n${referenceIndex}`);
}
