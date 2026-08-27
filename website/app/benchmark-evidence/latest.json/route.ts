import { validatedBenchmarkSnapshot } from "@/lib/build-admission-board";
import { loadBenchmarkSnapshot } from "@/lib/load-benchmark-snapshot";

export const dynamic = "force-static";

/** Build-time copy of the exact evidence envelope consumed by /benchmarks. */
export function GET() {
  return Response.json(
    validatedBenchmarkSnapshot(loadBenchmarkSnapshot()),
  );
}
