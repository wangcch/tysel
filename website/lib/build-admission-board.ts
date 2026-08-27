import type {
  AdmissionBoard,
  AdmissionRow,
  MeasureUnit,
  RuntimeSuite,
  SuiteMetricRow,
} from "./benchmark-evidence";

type GateMeasurement = {
  measured: number;
  limit: number;
  passed: boolean;
};

type MetricReport = {
  name: string;
  unit: string;
  status?: string | null;
  reason?: string | null;
  limit?: number | null;
  passed?: boolean | null;
  samples?: number[];
  sample_count?: number;
  p50?: number | null;
  p95?: number | null;
  p99?: number | null;
};

type SuiteReport = {
  suite: string;
  commit?: string;
  system?: {
    os: string;
    arch: string;
    os_version: string;
    cpu_model: string;
  };
  metrics: MetricReport[];
};

type RawEvidence = {
  evidence_version: number;
  source_commit: string;
  target: string;
  profile: string;
  command: string;
  system: {
    os: string;
    arch: string;
    os_version: string;
    cpu_model: string;
  };
  artifact: {
    sha256: string;
  };
  measurements: {
    cold_start_ms?: number[];
    cold_start_p50_ms: GateMeasurement;
    idle_memory_mb: GateMeasurement;
    artifact_mb: GateMeasurement;
    memory_kind: string;
  };
  suites?: SuiteReport[];
};

type SnapshotProvenance = {
  workflow_run_url?: string | null;
  workflow_run_id?: number | null;
  workflow_path?: string | null;
  artifact_name?: string | null;
  artifact_id?: number | null;
  run_created_at?: string | null;
  fetched_at?: string | null;
  evidence_sha256?: string | null;
  sample?: boolean | null;
};

type Snapshot =
  | { status: "unpublished" }
  | {
      status: "published";
      provenance: SnapshotProvenance;
      evidence: RawEvidence;
    };

const ADMISSION_DEFS = [
  {
    id: "cold_start",
    metric: "Fresh startup",
    detail: "p50 spawn→listen · 2 warm-ups + 11 samples · warm page cache",
    unit: "ms" as const,
    gate: 15,
    fromKb: false,
    fromMeasurements: "cold_start_p50_ms" as const,
    suiteMetric: { suite: "startup", name: "cold_start_p50_ms" },
  },
  {
    id: "idle_memory",
    metric: "Idle PSS",
    detail: "Single PSS sample ~400ms after listen",
    unit: "MiB" as const,
    gate: 32,
    fromKb: false,
    fromMeasurements: "idle_memory_mb" as const,
    suiteMetric: { suite: "memory", name: "idle_memory_mb" },
  },
  {
    id: "artifact",
    metric: "Executable size",
    detail: "Uncompressed on-disk hello-service binary",
    unit: "MiB" as const,
    gate: 20,
    fromKb: false,
    fromMeasurements: "artifact_mb" as const,
    suiteMetric: { suite: "binary-size", name: "artifact_mb" },
  },
  {
    id: "warm_isolate",
    metric: "Isolate create",
    detail: "p50 in-process create · not pool reuse",
    unit: "ms" as const,
    gate: 5,
    fromKb: false,
    fromMeasurements: null,
    suiteMetric: { suite: "isolate", name: "warm_create_ms" },
  },
  {
    id: "isolate_reuse",
    metric: "Reuse growth",
    detail: "Single before/after PSS delta · 1,000 reuses",
    unit: "MiB" as const,
    gate: 16,
    fromKb: true,
    fromMeasurements: null,
    suiteMetric: { suite: "isolate", name: "reuse_1000_growth_kb" },
  },
  {
    id: "task_backpressure",
    metric: "Queue delta",
    detail: "In-memory scheduler PSS delta · 10k tasks",
    unit: "MiB" as const,
    gate: 32,
    fromKb: true,
    fromMeasurements: null,
    suiteMetric: { suite: "task", name: "backpressure_memory_delta_kb" },
  },
  {
    id: "durable_resume",
    metric: "Durable resume",
    detail: "p50 on in-memory SQLite · not disk or Postgres",
    unit: "ms" as const,
    gate: 10,
    fromKb: false,
    fromMeasurements: null,
    suiteMetric: { suite: "durable", name: "resume_ms" },
  },
] as const;

const RUNTIME_SUITE_DEFS = [
  {
    id: "isolate",
    title: "Isolates",
    intent:
      "Single-host lifecycle microbenchmarks — process spawn, request dispatch, and crash replacement.",
    metrics: [
      {
        id: "cold_create",
        name: "cold_create_ms",
        metric: "Worker process spawn",
        detail: "p50 supervisor spawn → worker ready",
        unit: "ms" as const,
      },
      {
        id: "pool_acquire",
        name: "warm_pool_acquire_ms",
        metric: "Warm pool request",
        detail: "p50 full request through a pre-warmed isolated pool",
        unit: "ms" as const,
      },
      {
        id: "crash_replace",
        name: "crash_replace_ms",
        metric: "Crash replace",
        detail: "p50 replacement after isolate crash",
        unit: "ms" as const,
      },
    ],
  },
  {
    id: "task",
    title: "Tasks",
    intent:
      "In-memory scheduler microbenchmarks — not database or distributed throughput.",
    metrics: [
      {
        id: "enqueue_10000",
        name: "enqueue_10000_ms",
        metric: "Enqueue 10k",
        detail: "p50 time to enqueue 10,000 in-memory tasks",
        unit: "ms" as const,
      },
      {
        id: "claim_commit",
        name: "claim_commit_1000_ms",
        metric: "Claim + commit",
        detail: "p50 claim/commit for 1,000 in-memory tasks",
        unit: "ms" as const,
      },
      {
        id: "crash_requeue",
        name: "crash_requeue_ms",
        metric: "Crash requeue",
        detail: "p50 requeue after worker crash",
        unit: "ms" as const,
      },
    ],
  },
  {
    id: "durable",
    title: "Durable",
    intent:
      "In-memory SQLite except the explicitly file-backed restart recovery case.",
    metrics: [
      {
        id: "sqlite_append",
        name: "sqlite_append_ms",
        metric: "SQLite append · 32 events",
        detail: "p50 append on in-memory SQLite",
        unit: "ms" as const,
      },
      {
        id: "replay_1000",
        name: "replay_1000_effects_ms",
        metric: "Replay · 1,000 effects",
        detail: "p50 replay on in-memory SQLite",
        unit: "ms" as const,
      },
      {
        id: "restart_recovery",
        name: "restart_recovery_ms",
        metric: "File-backed restart",
        detail: "p50 reopen + replay of 16 events on temporary SQLite",
        unit: "ms" as const,
      },
    ],
  },
  {
    id: "http",
    title: "HTTP",
    intent:
      "Loopback microbenchmarks with explicit connection and batch boundaries — not external throughput.",
    metrics: [
      {
        id: "json_1kb",
        name: "json_1kb_ms",
        metric: "JSON 1 KiB · new H1",
        detail: "p50 TCP connect + request + full response",
        unit: "ms" as const,
      },
      {
        id: "websocket",
        name: "websocket_echo_ms",
        metric: "WebSocket connect + echo",
        detail: "p50 handshake + one echo + close",
        unit: "ms" as const,
      },
      {
        id: "http1_c100",
        name: "http1_concurrency_100_ms",
        metric: "H1 · 100-request batch",
        detail:
          "p50 wall time until all 100 concurrent new-connection requests finish",
        unit: "ms" as const,
      },
    ],
  },
] as const;

function sampleCount(metric: MetricReport | null | undefined): number | null {
  if (!metric) return null;
  if (Number.isInteger(metric.sample_count) && metric.sample_count! >= 0) {
    return metric.sample_count!;
  }
  if (Array.isArray(metric.samples)) return metric.samples.length;
  return null;
}

function displayUnit(raw: string, fallback: MeasureUnit): MeasureUnit {
  if (raw === "ms") return "ms";
  if (raw === "KB" || raw === "KiB") return "KiB";
  if (raw === "MB" || raw === "MiB") return "MiB";
  return fallback;
}

function emptyRows(): AdmissionRow[] {
  return ADMISSION_DEFS.map((def) => ({
    id: def.id,
    metric: def.metric,
    detail: def.detail,
    measured: null,
    unit: def.unit,
    gate: def.gate,
    status: "unpublished",
    samples: null,
    p95: null,
    p99: null,
  }));
}

function emptySuites(): RuntimeSuite[] {
  return RUNTIME_SUITE_DEFS.map((suite) => ({
    id: suite.id,
    title: suite.title,
    intent: suite.intent,
    metrics: suite.metrics.map((metric) => ({
      id: metric.id,
      metric: metric.metric,
      detail: metric.detail,
      measured: null,
      unit: metric.unit,
      gate: null,
      status: "unpublished",
      samples: null,
      p95: null,
      p99: null,
    })),
  }));
}

function findSuiteMetric(
  suites: SuiteReport[] | undefined,
  suite: string,
  name: string,
): MetricReport | null {
  const report = suites?.find(
    (item) => isRecord(item) && item.suite === suite,
  );
  if (!report || !Array.isArray(report.metrics)) return null;
  return (
    report.metrics.find(
      (metric) => isRecord(metric) && metric.name === name,
    ) ?? null
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function isSha256(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function isPositiveInteger(value: unknown): value is number {
  return Number.isSafeInteger(value) && (value as number) > 0;
}

function isIsoTimestamp(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$/.test(value) &&
    Number.isFinite(Date.parse(value))
  );
}

function isWorkflowRunUrl(value: unknown, runId: number): value is string {
  if (typeof value !== "string") return false;
  const match = value.match(
    /^https:\/\/github\.com\/[^/]+\/[^/]+\/actions\/runs\/([1-9][0-9]*)$/,
  );
  return match != null && Number(match[1]) === runId;
}

function isValidProvenance(value: unknown): value is SnapshotProvenance {
  if (!isRecord(value)) return false;

  // Explicit samples are always rendered as demonstrations and never as CI
  // claims. Real publications require the complete artifact identity below.
  if (value.sample === true) return true;
  if (value.sample !== false || !isPositiveInteger(value.workflow_run_id)) {
    return false;
  }

  return (
    isWorkflowRunUrl(value.workflow_run_url, value.workflow_run_id) &&
    value.workflow_path === ".github/workflows/ci.yml" &&
    value.artifact_name === "benchmark-evidence-linux-x64" &&
    isPositiveInteger(value.artifact_id) &&
    isIsoTimestamp(value.run_created_at) &&
    isIsoTimestamp(value.fetched_at) &&
    isSha256(value.evidence_sha256)
  );
}

function isGateMeasurement(
  value: unknown,
  expectedLimit: number,
): value is GateMeasurement {
  if (!isRecord(value)) return false;
  const measured = value.measured;
  const limit = value.limit;
  const passed = value.passed;
  return (
    isFiniteNonNegative(measured) &&
    isFiniteNonNegative(limit) &&
    limit === expectedLimit &&
    typeof passed === "boolean" &&
    passed === (measured <= limit)
  );
}

function hasValidGateMetric(
  suites: SuiteReport[],
  def: (typeof ADMISSION_DEFS)[number],
): boolean {
  const matchingSuites = suites.filter(
    (suite) => isRecord(suite) && suite.suite === def.suiteMetric.suite,
  );
  if (matchingSuites.length !== 1 || !Array.isArray(matchingSuites[0].metrics)) {
    return false;
  }
  const matchingMetrics = matchingSuites[0].metrics.filter(
    (metric) => isRecord(metric) && metric.name === def.suiteMetric.name,
  );
  if (matchingMetrics.length !== 1) return false;
  const metric = matchingMetrics[0];

  const expectedUnit = def.fromKb ? "KB" : def.unit === "MiB" ? "MB" : "ms";
  const expectedLimit = def.fromKb ? def.gate * 1024 : def.gate;
  return (
    metric.unit === expectedUnit &&
    isFiniteNonNegative(metric.p50) &&
    metric.limit === expectedLimit &&
    typeof metric.passed === "boolean" &&
    metric.passed === (metric.p50 <= expectedLimit) &&
    metric.status === (metric.passed ? "pass" : "fail")
  );
}

function hasConsistentSuiteProvenance(
  suites: SuiteReport[],
  sourceCommit: string,
  system: RawEvidence["system"],
): boolean {
  return suites.every(
    (suite) =>
      isRecord(suite) &&
      suite.commit === sourceCommit &&
      isRecord(suite.system) &&
      suite.system.os === system.os &&
      suite.system.arch === system.arch &&
      suite.system.os_version === system.os_version &&
      suite.system.cpu_model === system.cpu_model,
  );
}

function hasValidRuntimeSuite(
  suites: SuiteReport[],
  def: (typeof RUNTIME_SUITE_DEFS)[number],
): boolean {
  const matchingSuites = suites.filter(
    (suite) => isRecord(suite) && suite.suite === def.id,
  );
  if (matchingSuites.length !== 1 || !Array.isArray(matchingSuites[0].metrics)) {
    return false;
  }

  return def.metrics.every((metricDef) => {
    const matches = matchingSuites[0].metrics.filter(
      (metric) => isRecord(metric) && metric.name === metricDef.name,
    );
    if (matches.length !== 1) return false;
    const metric = matches[0];
    if (metric.unit !== metricDef.unit) return false;
    if (metric.status === "skipped") return true;
    if (!isFiniteNonNegative(metric.p50)) return false;
    if (metric.p95 != null && !isFiniteNonNegative(metric.p95)) return false;
    if (metric.p99 != null && !isFiniteNonNegative(metric.p99)) return false;
    if (metric.p95 != null && metric.p95 < metric.p50) return false;
    if (metric.p99 != null && metric.p99 < (metric.p95 ?? metric.p50)) {
      return false;
    }
    if (
      metric.sample_count != null &&
      (!Number.isInteger(metric.sample_count) || metric.sample_count < 0)
    ) {
      return false;
    }
    return (
      metric.samples == null ||
      (Array.isArray(metric.samples) &&
        metric.samples.every(isFiniteNonNegative))
    );
  });
}

function rowFromEvidence(
  def: (typeof ADMISSION_DEFS)[number],
  evidence: RawEvidence,
): AdmissionRow {
  const suiteMetric = findSuiteMetric(
    evidence.suites,
    def.suiteMetric.suite,
    def.suiteMetric.name,
  );

  if (suiteMetric?.status === "skipped") {
    return {
      id: def.id,
      metric: def.metric,
      detail: def.detail,
      measured: null,
      unit: def.unit,
      gate: def.gate,
      status: "skipped",
      samples: null,
      p95: null,
      p99: null,
    };
  }

  if (suiteMetric?.p50 != null) {
    const passed =
      suiteMetric.passed ??
      (suiteMetric.limit != null ? suiteMetric.p50 <= suiteMetric.limit : null);
    const scale = def.fromKb ? 1 / 1024 : 1;
    return {
      id: def.id,
      metric: def.metric,
      detail: def.detail,
      measured: suiteMetric.p50 * scale,
      unit: def.unit,
      gate:
        suiteMetric.limit != null ? suiteMetric.limit * scale : def.gate,
      status: passed == null ? "unpublished" : passed ? "pass" : "fail",
      samples: sampleCount(suiteMetric),
      p95: isFiniteNonNegative(suiteMetric.p95)
        ? suiteMetric.p95 * scale
        : null,
      p99: isFiniteNonNegative(suiteMetric.p99)
        ? suiteMetric.p99 * scale
        : null,
    };
  }

  if (def.fromMeasurements) {
    const gate = evidence.measurements[def.fromMeasurements];
    return {
      id: def.id,
      metric: def.metric,
      detail: def.detail,
      measured: gate.measured,
      unit: def.unit,
      gate: gate.limit,
      status: gate.passed ? "pass" : "fail",
      samples:
        def.fromMeasurements === "cold_start_p50_ms"
          ? (evidence.measurements.cold_start_ms?.length ?? null)
          : null,
      p95: null,
      p99: null,
    };
  }

  return {
    id: def.id,
    metric: def.metric,
    detail: def.detail,
    measured: null,
    unit: def.unit,
    gate: def.gate,
    status: "unpublished",
    samples: null,
    p95: null,
    p99: null,
  };
}

function suiteMetricFromEvidence(
  suiteId: string,
  def: (typeof RUNTIME_SUITE_DEFS)[number]["metrics"][number],
  evidence: RawEvidence,
): SuiteMetricRow {
  const raw = findSuiteMetric(evidence.suites, suiteId, def.name);
  if (!raw || raw.status === "skipped" || !isFiniteNonNegative(raw.p50)) {
    return {
      id: def.id,
      metric: def.metric,
      detail: def.detail,
      measured: null,
      unit: def.unit,
      gate: raw?.limit ?? null,
      status: raw?.status === "skipped" ? "skipped" : "unpublished",
      samples: null,
      p95: null,
      p99: null,
    };
  }

  const unit = displayUnit(raw.unit, def.unit);
  // Evidence JSON labels units as KB/MB but values are already 1024-based (KiB/MiB).
  // Map labels only — do not rescale the numeric values.
  const status: SuiteMetricRow["status"] =
    raw.status === "skipped" ? "skipped" : "observed";

  return {
    id: def.id,
    metric: def.metric,
    detail: def.detail,
    measured: raw.p50,
    unit,
    gate: null,
    status,
    samples: sampleCount(raw),
    p95: isFiniteNonNegative(raw.p95) ? raw.p95 : null,
    p99: isFiniteNonNegative(raw.p99) ? raw.p99 : null,
  };
}

function suitesFromEvidence(evidence: RawEvidence): RuntimeSuite[] {
  return RUNTIME_SUITE_DEFS.map((suite) => ({
    id: suite.id,
    title: suite.title,
    intent: suite.intent,
    metrics: suite.metrics.map((metric) =>
      suiteMetricFromEvidence(suite.id, metric, evidence),
    ),
  }));
}

function isRawEvidence(value: unknown): value is RawEvidence {
  if (!isRecord(value)) return false;
  const system = value.system;
  const artifact = value.artifact;
  const measurements = value.measurements;
  const suites = value.suites;
  if (
    value.evidence_version !== 2 ||
    typeof value.source_commit !== "string" ||
    !/^[0-9a-f]{40}$/.test(value.source_commit) ||
    value.target !== "linux-x64" ||
    value.profile !== "release" ||
    !isNonEmptyString(value.command) ||
    !isRecord(system) ||
    system.os !== "linux" ||
    system.arch !== "x86_64" ||
    !isNonEmptyString(system.os_version) ||
    !isNonEmptyString(system.cpu_model) ||
    !isRecord(artifact) ||
    !isSha256(artifact.sha256) ||
    !isRecord(measurements) ||
    measurements.memory_kind !== "pss" ||
    !isGateMeasurement(measurements.cold_start_p50_ms, 15) ||
    !isGateMeasurement(measurements.idle_memory_mb, 32) ||
    !isGateMeasurement(measurements.artifact_mb, 20) ||
    !Array.isArray(suites)
  ) {
    return false;
  }

  const typedSuites = suites as SuiteReport[];
  return (
    hasConsistentSuiteProvenance(
      typedSuites,
      value.source_commit,
      system as RawEvidence["system"],
    ) &&
    ADMISSION_DEFS.every((def) => hasValidGateMetric(typedSuites, def)) &&
    RUNTIME_SUITE_DEFS.every((def) => hasValidRuntimeSuite(typedSuites, def))
  );
}

function parseSnapshot(value: unknown): Snapshot {
  if (!value || typeof value !== "object") {
    return { status: "unpublished" };
  }
  const record = value as Record<string, unknown>;
  if (record.status === "unpublished") {
    return { status: "unpublished" };
  }
  if (
    record.status === "published" &&
    isRawEvidence(record.evidence) &&
    isValidProvenance(record.provenance)
  ) {
    return {
      status: "published",
      provenance: record.provenance,
      evidence: record.evidence,
    };
  }
  return { status: "unpublished" };
}

/** Pure builder used by the page loader and unit tests. */
export function buildAdmissionBoard(raw: unknown): AdmissionBoard {
  const snapshot = parseSnapshot(raw);
  if (snapshot.status === "unpublished") {
    return {
      status: "unpublished",
      rows: emptyRows(),
      suites: emptySuites(),
      provenance: null,
    };
  }

  const { evidence } = snapshot;
  const rows = ADMISSION_DEFS.map((def) => rowFromEvidence(def, evidence));
  if (rows.some((row) => row.status !== "pass" && row.status !== "fail")) {
    return {
      status: "unpublished",
      rows: emptyRows(),
      suites: emptySuites(),
      provenance: null,
    };
  }
  const sample = snapshot.provenance.sample === true;

  return {
    status: "published",
    rows,
    suites: suitesFromEvidence(evidence),
    provenance: {
      sourceCommit: evidence.source_commit,
      target: evidence.target,
      profile: evidence.profile,
      command: evidence.command,
      os: evidence.system.os,
      arch: evidence.system.arch,
      osVersion: evidence.system.os_version,
      cpuModel: evidence.system.cpu_model,
      memoryKind: evidence.measurements.memory_kind,
      artifactSha256: evidence.artifact.sha256,
      evidenceVersion: evidence.evidence_version,
      // Sample fixtures must not surface as CI claims.
      workflowRunUrl: sample
        ? null
        : (snapshot.provenance.workflow_run_url ?? null),
      workflowRunId: snapshot.provenance.workflow_run_id ?? null,
      workflowPath: snapshot.provenance.workflow_path ?? null,
      artifactName: snapshot.provenance.artifact_name ?? null,
      artifactId: snapshot.provenance.artifact_id ?? null,
      runCreatedAt: snapshot.provenance.run_created_at ?? null,
      fetchedAt: snapshot.provenance.fetched_at ?? null,
      evidenceSha256: snapshot.provenance.evidence_sha256 ?? null,
      sample,
    },
  };
}

/**
 * Preserve the exact evidence envelope only when the same validator used by
 * the page accepts it. Public raw evidence must fail closed with the page.
 */
export function validatedBenchmarkSnapshot(raw: unknown): unknown {
  return buildAdmissionBoard(raw).status === "published"
    ? raw
    : { status: "unpublished" };
}
