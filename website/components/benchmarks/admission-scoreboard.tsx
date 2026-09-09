"use client";

import { T, SourceText } from "@/components/locale-provider";
import { SiteLink as Link } from "@/components/locale-provider";
import {
  formatGate,
  formatMeasured,
  formatMeasuredParts,
  gateUtilization,
  shortCommit,
  type AdmissionRow,
  type EvidenceProvenance,
  type GateStatus,
} from "@/lib/benchmark-evidence";
import { githubUrl } from "@/lib/shared";
import { useRunProgress } from "./use-run-progress";

function statusCopy(
  status: GateStatus,
  demo: boolean,
): string {
  if (demo) return "DEMO";
  switch (status) {
    case "pass":
      return "PASS";
    case "fail":
      return "FAIL";
    case "skipped":
      return "SKIP";
    default:
      return "OPEN";
  }
}

function MeasuredValue({
  value,
  unit,
  progress,
  published,
}: {
  value: number | null;
  unit: AdmissionRow["unit"];
  progress: number;
  published: boolean;
}) {
  if (!published || value == null) {
    return (
      <div className="flex w-full items-baseline justify-end gap-2 sm:gap-3">
        <p className="font-heading text-5xl leading-none tracking-tighter text-white/20 tabular-nums sm:text-6xl">
          —
        </p>
        <span className="w-10 shrink-0 font-mono text-sm text-white/20 sm:w-12 sm:text-base">
          {unit}
        </span>
      </div>
    );
  }

  // Digits always show the final measured value; progress only fades them in.
  // (Count-up through rAF/timers can stall in background tabs and leave "0.00 + PASS".)
  const parts = formatMeasuredParts(value, unit);

  return (
    <div
      className="flex w-full items-baseline justify-end gap-2 sm:gap-3"
      style={{ opacity: 0.2 + progress * 0.8 }}
    >
      <p className="font-heading flex items-baseline leading-none tracking-tighter text-white tabular-nums">
        <span className="text-5xl sm:text-6xl">{parts.whole}</span>
        {parts.fraction != null ? (
          <span className="text-[1.55rem] text-white/45 sm:text-[1.85rem]">
            .{parts.fraction}
          </span>
        ) : null}
      </p>
      <span className="w-10 shrink-0 font-mono text-sm text-white/40 sm:w-12 sm:text-base">
        {unit}
      </span>
    </div>
  );
}

function MetricRow({
  row,
  index,
  demo,
  compact,
}: {
  row: AdmissionRow;
  index: number;
  demo: boolean;
  compact: boolean;
}) {
  const progress = useRunProgress(60 + index * 70, 750);
  const util = gateUtilization(row.measured, row.gate);
  const published = row.measured != null;
  const barWidth = util == null ? "0%" : `${Math.min(util, 1) * 100}%`;
  const barTone =
    !demo && row.status === "fail"
      ? "bg-red-400"
      : util != null && util <= 0.5
        ? "bg-tysel-lime"
        : "bg-tysel-blue";
  const status = statusCopy(row.status, demo);

  return (
    <article
      className={`grid gap-3 border-b border-white/10 px-5 last:border-b-0 sm:grid-cols-[minmax(0,1.2fr)_minmax(10rem,1fr)_4.5rem] sm:items-end sm:gap-6 sm:px-8 ${
        compact ? "py-4 sm:py-4" : "py-5 sm:py-6"
      }`}
      aria-label={
        published
          ? `${row.metric}: ${formatMeasured(row.measured, row.unit)}, gate ${formatGate(row.gate, row.unit)}, ${status}`
          : `${row.metric}: not published, gate ${formatGate(row.gate, row.unit)}`
      }
    >
      <div className="min-w-0">
        <div className="flex items-baseline justify-between gap-3 sm:block">
          <p className="text-sm font-medium tracking-tight text-white/85">
            <SourceText text={row.metric} />
          </p>
          <p
            className={`font-mono text-[11px] tracking-[0.14em] sm:hidden ${
              demo
                ? "text-white/35"
                : row.status === "pass"
                  ? "text-tysel-lime"
                  : row.status === "fail"
                    ? "text-red-400"
                    : "text-white/30"
            }`}
            style={{ opacity: 0.35 + progress * 0.65 }}
          >
            <SourceText text={status} />
          </p>
        </div>
        <p className="mt-1 max-w-md text-xs leading-5 text-white/40">
          <SourceText text={row.detail} />
        </p>
        <div className="mt-3 max-w-sm">
          <div className="flex items-baseline justify-between gap-3 font-mono text-[10px] uppercase tracking-[0.12em] text-white/35">
            <span>{formatGate(row.gate, row.unit)}</span>
            <span>
              {util == null ? "—" : `${Math.round(util * 100)}%`}
            </span>
          </div>
          <div className="mt-1.5 h-1 w-full bg-white/10">
            <div
              className={`h-full origin-left ${barTone}`}
              style={{
                width: barWidth,
                transform: `scaleX(${0.08 + progress * 0.92})`,
              }}
            />
          </div>
        </div>
      </div>

      <div className="min-w-0 sm:justify-self-stretch">
        <MeasuredValue
          value={row.measured}
          unit={row.unit}
          progress={progress}
          published={published}
        />
      </div>

      <p
        className={`hidden font-mono text-xs font-medium tracking-[0.14em] sm:block sm:pb-1 sm:text-right ${
          demo
            ? "text-white/35"
            : row.status === "pass"
              ? "text-tysel-lime"
              : row.status === "fail"
                ? "text-red-400"
                : "text-white/30"
        }`}
        style={{ opacity: 0.35 + progress * 0.65 }}
      >
        <SourceText text={status} />
      </p>
    </article>
  );
}

export function AdmissionScoreboard({
  rows,
  provenance,
  published,
}: {
  rows: AdmissionRow[];
  provenance: EvidenceProvenance | null;
  published: boolean;
}) {
  const headerProgress = useRunProgress(0, 600);
  const sample = provenance?.sample === true;
  const claim = published && !sample;
  const compact = !published;
  const passed = rows.filter((row) => row.status === "pass").length;
  const shownPassed = Math.round(passed * headerProgress);

  return (
    <section className="home-terminal overflow-hidden bg-tysel-ink text-white">
      <div className="grid gap-4 border-b border-white/10 px-5 py-5 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end sm:gap-6 sm:px-8 sm:py-6">
        <div>
          <p className="font-mono text-[10px] uppercase tracking-[0.16em] text-white/40">
            <T id="ui.ci.admission.linux.x64" />
          </p>
          {claim ? (
            <p className="font-heading mt-2 text-3xl tracking-tighter sm:text-4xl">
              <span className="text-tysel-lime">{shownPassed}</span>
              <span className="text-white/35">/{rows.length}</span>
              <span className="ml-2 text-xl text-white/65 sm:text-2xl">
                <T id="ui.clear" />
              </span>
            </p>
          ) : sample ? (
            <p className="font-heading mt-2 text-3xl tracking-tighter text-white/55 sm:text-4xl">
              <T id="ui.sample.layout" />
            </p>
          ) : (
            <p className="font-heading mt-2 text-3xl tracking-tighter text-white/35 sm:text-4xl">
              <T id="ui.waiting.on.ci" />
            </p>
          )}
          {sample ? (
            <p className="mt-1.5 font-mono text-[10px] uppercase tracking-[0.12em] text-white/30">
              <T id="ui.not.a.ci.claim" />
            </p>
          ) : null}
        </div>

        {provenance ? (
          <div className="space-y-0.5 font-mono text-[11px] text-white/40 sm:text-right">
            <p>
              {provenance.target} · {provenance.profile} ·{" "}
              {provenance.memoryKind.toUpperCase()}
            </p>
            {sample ? (
              <p className="text-white/30"><T id="ui.illustrative.snapshot" /></p>
            ) : (
              <>
                <p>
                  <Link
                    href={`${githubUrl}/commit/${provenance.sourceCommit}`}
                    className="text-white/70 underline-offset-4 hover:text-white hover:underline"
                  >
                    {shortCommit(provenance.sourceCommit)}
                  </Link>
                  {provenance.workflowRunUrl ? (
                    <>
                      {" · "}
                      <Link
                        href={provenance.workflowRunUrl}
                        className="text-white/70 underline-offset-4 hover:text-white hover:underline"
                      >
                        <T id="ui.ci.run" />
                      </Link>
                    </>
                  ) : null}
                </p>
                {provenance.fetchedAt ? (
                  <p className="text-white/30">
                    <T id="ui.fetched" /> {provenance.fetchedAt}
                  </p>
                ) : null}
              </>
            )}
          </div>
        ) : (
          <p className="max-w-xs font-mono text-[11px] leading-5 text-white/30 sm:text-right">
            <T id="ui.gates.stay.public.measured.cells.fill.from" />{" "}
            <span className="text-white/50">benchmark-evidence-linux-x64</span>.
          </p>
        )}
      </div>

      <div>
        {rows.map((row, index) => (
          <MetricRow
            key={row.id}
            row={row}
            index={index}
            demo={sample}
            compact={compact}
          />
        ))}
      </div>

      <div className="border-t border-white/10 px-5 py-2.5 font-mono text-[10px] leading-5 text-white/30 sm:flex sm:justify-between sm:gap-6 sm:px-8">
        <p><T id="ui.admit.on.p50.linux.pss.regression.budget.not" /></p>
        {provenance && !sample ? (
          <p className="mt-1 truncate sm:mt-0 sm:max-w-md sm:text-right">
            {provenance.os} {provenance.arch}
            {provenance.cpuModel ? ` · ${provenance.cpuModel}` : ""}
          </p>
        ) : null}
      </div>
    </section>
  );
}
