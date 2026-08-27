import currentSnapshot from "@/data/benchmarks/admission-linux-x64.json";
import sampleSnapshot from "@/data/benchmarks/admission-linux-x64.sample.json";

/**
 * Server/build-time snapshot selection. CI never sets the preview flag, so a
 * sample cannot silently replace canonical benchmark evidence in production.
 */
export function loadBenchmarkSnapshot(): unknown {
  return process.env.TYSEL_BENCHMARK_SAMPLE === "1"
    ? sampleSnapshot
    : currentSnapshot;
}
