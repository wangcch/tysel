import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Benchmarks",
  description:
    "Tysel benchmark methodology and release-admission gates, not marketing numbers.",
};

const gates = [
  ["Median cold start", "≤ 15 ms", "Linux benchmark job"],
  ["Idle memory", "≤ 32 MiB PSS", "Linux PSS"],
  ["Packaged executable", "≤ 20 MiB", "Built artifact size"],
  ["Warm isolate creation p50", "≤ 5 ms", "In-process isolate suite"],
  ["Durable task resume p50", "≤ 10 ms", "Durable resume suite"],
];

export default function BenchmarksPage() {
  return (
    <main className="mx-auto w-full max-w-3xl px-6 py-16">
      <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
        Benchmarks
      </p>
      <h1 className="font-heading mt-3 text-4xl font-medium tracking-tighter text-balance">
        Evidence before comparison charts.
      </h1>
      <p className="mt-4 leading-7 text-fd-muted-foreground">
        Unlike marketing homepages that lead with speed multipliers, Tysel does
        not publish point measurements as homepage claims. Size, startup,
        memory, isolate, and durable numbers belong with the release, hardware,
        workload, and command that produced them.
      </p>
      <h2 className="mt-12 text-xl font-medium">Release-admission gates</h2>
      <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
        These are engineering thresholds, not capacity promises.
      </p>
      <div className="mt-6 overflow-hidden border border-fd-border">
        <table className="w-full text-sm">
          <thead className="bg-fd-muted text-left">
            <tr>
              <th className="px-4 py-3 font-medium">Metric</th>
              <th className="px-4 py-3 font-medium">Gate</th>
              <th className="px-4 py-3 font-medium">Record</th>
            </tr>
          </thead>
          <tbody>
            {gates.map(([metric, gate, record]) => (
              <tr key={metric} className="border-t border-fd-border">
                <td className="px-4 py-3">{metric}</td>
                <td className="px-4 py-3 font-mono">{gate}</td>
                <td className="px-4 py-3 text-fd-muted-foreground">{record}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <pre className="home-terminal mt-8 overflow-x-auto border border-fd-border bg-tysel-ink p-4 font-mono text-[13px] leading-6 text-white/90">
        <code>{`tysel bench startup
tysel bench all --format json
tysel bench all --evidence dist/benchmark-evidence.json`}</code>
      </pre>
      <p className="mt-6 text-sm leading-6 text-fd-muted-foreground">
        Linux PSS is the memory result of record. macOS RSS is a development
        proxy. Read the{" "}
        <Link href="/docs/performance" className="underline underline-offset-4">
          performance documentation
        </Link>{" "}
        and the{" "}
        <Link
          href="https://github.com/wangcch/tysel/tree/main/benchmarks"
          className="underline underline-offset-4"
        >
          benchmark harness
        </Link>{" "}
        before quoting a number.
      </p>
    </main>
  );
}
