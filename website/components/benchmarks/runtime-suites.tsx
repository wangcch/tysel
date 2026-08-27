import { Fragment } from "react";
import {
  formatMeasuredParts,
  type RuntimeSuite,
  type SuiteMetricRow,
} from "@/lib/benchmark-evidence";

function cell(value: number | null, unit: SuiteMetricRow["unit"]): string {
  if (value == null) return "—";
  const parts = formatMeasuredParts(value, unit);
  return parts.fraction ? `${parts.whole}.${parts.fraction}` : parts.whole;
}

export function RuntimeSuites({ suites }: { suites: RuntimeSuite[] }) {
  const hasData = suites.some((suite) =>
    suite.metrics.some((metric) => metric.measured != null),
  );

  if (!hasData) return null;

  return (
    <section className="mt-14">
      <div className="max-w-3xl">
        <p className="text-xs font-medium uppercase tracking-[0.18em] text-fd-muted-foreground">
          Runtime microbenchmarks
        </p>
        <h2 className="font-heading mt-2 text-xl font-medium tracking-tight">
          Same run · not release gates
        </h2>
        <p className="mt-2 text-sm leading-6 text-fd-muted-foreground">
          Observational only — they never fail a release and are not production
          throughput claims. Values in ms.
        </p>
      </div>

      <div className="mt-6 max-w-3xl overflow-x-auto border border-fd-border">
        <table className="w-full min-w-[26rem] table-fixed text-left">
          <colgroup>
            <col />
            <col className="w-16" />
            <col className="w-16" />
            <col className="w-16" />
          </colgroup>
          <thead>
            <tr className="border-b border-fd-border font-mono text-[10px] uppercase tracking-[0.12em] text-fd-muted-foreground">
              <th className="px-4 py-2.5 font-normal sm:px-5">Metric</th>
              <th className="px-2 py-2.5 text-right font-normal">p50</th>
              <th className="px-2 py-2.5 text-right font-normal">p95</th>
              <th className="px-4 py-2.5 text-right font-normal sm:pr-5">
                p99
              </th>
            </tr>
          </thead>
          <tbody>
            {suites.map((suite) => (
              <Fragment key={suite.id}>
                <tr className="bg-fd-muted/25">
                  <td colSpan={4} className="px-4 py-2 sm:px-5">
                    <div className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5">
                      <span className="text-sm font-medium">{suite.title}</span>
                      <span className="text-[11px] text-fd-muted-foreground">
                        {suite.intent}
                      </span>
                    </div>
                  </td>
                </tr>
                {suite.metrics.map((row) => (
                  <tr
                    key={row.id}
                    className="border-b border-fd-border/60 last:border-b-0"
                  >
                    <td className="px-4 py-2 align-middle sm:px-5">
                      <p className="text-sm font-medium leading-5">
                        {row.metric}
                      </p>
                      <p className="mt-0.5 text-[11px] leading-4 text-fd-muted-foreground">
                        {row.detail}
                        {row.samples != null ? ` · n=${row.samples}` : null}
                      </p>
                    </td>
                    <td className="px-2 py-2 text-right align-middle font-mono text-[13px] tabular-nums">
                      {cell(row.measured, row.unit)}
                    </td>
                    <td className="px-2 py-2 text-right align-middle font-mono text-[13px] tabular-nums text-fd-muted-foreground">
                      {cell(row.p95, row.unit)}
                    </td>
                    <td className="px-4 py-2 text-right align-middle font-mono text-[13px] tabular-nums text-fd-muted-foreground sm:pr-5">
                      {cell(row.p99, row.unit)}
                    </td>
                  </tr>
                ))}
              </Fragment>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
