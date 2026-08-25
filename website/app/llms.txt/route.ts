import { llms, referenceSource, source } from "@/lib/source";

export const revalidate = false;

export function GET() {
  return new Response(`${llms(source).index()}\n\n${llms(referenceSource).index()}`);
}
