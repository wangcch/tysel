export type GateStatus = "pass" | "fail" | "unpublished" | "skipped";

export type MeasureUnit = "ms" | "MiB" | "KiB";

export type AdmissionRow = {
  id: string;
  metric: string;
  detail: string;
  measured: number | null;
  unit: MeasureUnit;
  gate: number;
  status: GateStatus;
  samples: number | null;
  p95: number | null;
  p99: number | null;
};

export type SuiteMetricRow = {
  id: string;
  metric: string;
  detail: string;
  measured: number | null;
  unit: MeasureUnit;
  gate: number | null;
  status: GateStatus | "observed";
  samples: number | null;
  p95: number | null;
  p99: number | null;
};

export type RuntimeSuite = {
  id: string;
  title: string;
  intent: string;
  metrics: SuiteMetricRow[];
};

export type EvidenceProvenance = {
  sourceCommit: string;
  target: string;
  profile: string;
  command: string;
  os: string;
  arch: string;
  osVersion: string;
  cpuModel: string;
  memoryKind: string;
  artifactSha256: string;
  evidenceVersion: number;
  workflowRunUrl: string | null;
  workflowRunId: number | null;
  workflowPath: string | null;
  artifactName: string | null;
  artifactId: number | null;
  runCreatedAt: string | null;
  fetchedAt: string | null;
  evidenceSha256: string | null;
  sample: boolean;
};

export type AdmissionBoard =
  | {
      status: "unpublished";
      rows: AdmissionRow[];
      suites: RuntimeSuite[];
      provenance: null;
    }
  | {
      status: "published";
      rows: AdmissionRow[];
      suites: RuntimeSuite[];
      provenance: EvidenceProvenance;
    };

export function formatMeasured(
  value: number | null,
  unit: MeasureUnit,
): string {
  if (value == null) return "—";
  const parts = formatMeasuredParts(value, unit);
  return parts.fraction
    ? `${parts.whole}.${parts.fraction} ${parts.unit}`
    : `${parts.whole} ${parts.unit}`;
}

export function formatMeasuredParts(
  value: number | null,
  unit: MeasureUnit,
): { whole: string; fraction: string | null; unit: string; digits: number } {
  if (value == null) {
    return { whole: "—", fraction: null, unit, digits: 0 };
  }
  const digits =
    unit === "ms" ? (value < 10 ? 2 : 1) : unit === "KiB" ? 0 : 1;
  const fixed = value.toFixed(digits);
  const [whole, fraction = null] = fixed.split(".");
  return { whole, fraction, unit, digits };
}

export function formatGate(limit: number, unit: MeasureUnit): string {
  return `≤ ${limit} ${unit}`;
}

/** Fraction of the gate used by the measured value (0..1+). */
export function gateUtilization(
  measured: number | null,
  gate: number | null,
): number | null {
  if (measured == null || gate == null || gate <= 0) return null;
  return measured / gate;
}

export function shortCommit(commit: string): string {
  return commit.slice(0, 7);
}
