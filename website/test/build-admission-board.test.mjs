import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  buildAdmissionBoard,
  validatedBenchmarkSnapshot,
} from "../lib/build-admission-board.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function readJson(rel) {
  return JSON.parse(readFileSync(join(root, rel), "utf8"));
}

test("unpublished snapshot keeps empty measured cells", () => {
  const board = buildAdmissionBoard({ status: "unpublished" });
  assert.equal(board.status, "unpublished");
  assert.equal(board.provenance, null);
  assert.equal(board.rows.length, 7);
  assert.ok(board.rows.every((row) => row.measured == null));
  assert.ok(board.rows.every((row) => row.status === "unpublished"));
});

test("sample fixture is marked sample and strips CI run claims", () => {
  const board = buildAdmissionBoard(
    readJson("data/benchmarks/admission-linux-x64.sample.json"),
  );
  assert.equal(board.status, "published");
  assert.ok(board.provenance);
  assert.equal(board.provenance.sample, true);
  assert.equal(board.provenance.workflowRunUrl, null);
  assert.equal(board.rows.length, 7);
  assert.ok(board.rows.some((row) => row.measured != null));
});

test("published evidence without complete provenance fails closed", () => {
  const snapshot = readJson(
    "data/benchmarks/admission-linux-x64.sample.json",
  );
  delete snapshot.provenance;
  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");

  snapshot.provenance = { sample: false };
  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");
});

test("complete production provenance is accepted and bound to its run ID", () => {
  const snapshot = readJson(
    "data/benchmarks/admission-linux-x64.sample.json",
  );
  snapshot.provenance = {
    workflow_run_url:
      "https://github.com/wangcch/tysel/actions/runs/123456789",
    workflow_run_id: 123456789,
    workflow_path: ".github/workflows/ci.yml",
    artifact_name: "benchmark-evidence-linux-x64",
    artifact_id: 987654321,
    run_created_at: "2026-08-26T11:30:00Z",
    fetched_at: "2026-08-26T12:00:00Z",
    evidence_sha256: "a".repeat(64),
    sample: false,
  };
  assert.equal(buildAdmissionBoard(snapshot).status, "published");

  snapshot.provenance.workflow_run_url =
    "https://github.com/wangcch/tysel/actions/runs/111";
  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");
});

test("admission gates include reuse and backpressure; suites stay observational", () => {
  const board = buildAdmissionBoard(
    readJson("data/benchmarks/admission-linux-x64.sample.json"),
  );
  assert.equal(board.status, "published");
  const ids = board.rows.map((row) => row.id);
  assert.deepEqual(ids, [
    "cold_start",
    "idle_memory",
    "artifact",
    "warm_isolate",
    "isolate_reuse",
    "task_backpressure",
    "durable_resume",
  ]);

  const reuse = board.rows.find((row) => row.id === "isolate_reuse");
  assert.ok(reuse);
  assert.ok(reuse.measured != null);
  assert.ok(reuse.status === "pass" || reuse.status === "fail");

  for (const suite of board.suites) {
    for (const metric of suite.metrics) {
      if (metric.measured != null) {
        assert.equal(metric.status, "observed");
        assert.equal(metric.gate, null);
      }
    }
  }
});

test("public gate labels state measurement boundaries", () => {
  const board = buildAdmissionBoard(
    readJson("data/benchmarks/admission-linux-x64.sample.json"),
  );
  assert.equal(board.status, "published");
  const byId = Object.fromEntries(board.rows.map((row) => [row.id, row]));
  assert.match(byId.cold_start.metric, /Fresh startup/i);
  assert.match(byId.cold_start.detail, /warm page cache/i);
  assert.match(byId.warm_isolate.metric, /Isolate create/i);
  assert.match(byId.warm_isolate.detail, /not pool reuse/i);
  assert.match(byId.durable_resume.detail, /in-memory SQLite/i);
  assert.match(byId.artifact.detail, /Uncompressed/i);

  const http = board.suites.find((suite) => suite.id === "http");
  assert.ok(http);
  const json = http.metrics.find((metric) => metric.id === "json_1kb");
  const ws = http.metrics.find((metric) => metric.id === "websocket");
  const batch = http.metrics.find((metric) => metric.id === "http1_c100");
  assert.match(json.detail, /connect/i);
  assert.match(ws.detail, /handshake/i);
  assert.match(batch.detail, /100 concurrent/i);
});

test("nested malformed evidence fails closed instead of throwing", () => {
  const board = buildAdmissionBoard({
    status: "published",
    evidence: {
      evidence_version: 2,
      source_commit: "0".repeat(40),
      measurements: {},
    },
  });
  assert.equal(board.status, "unpublished");
});

test("malformed observational samples fail closed instead of throwing", () => {
  const snapshot = readJson("data/benchmarks/admission-linux-x64.sample.json");
  const http = snapshot.evidence.suites.find(
    (suite) => suite.suite === "http",
  );
  http.metrics[0].samples = "not-an-array";
  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");
});

test("missing one admission metric invalidates the complete snapshot", () => {
  const snapshot = readJson("data/benchmarks/admission-linux-x64.sample.json");
  const isolate = snapshot.evidence.suites.find(
    (suite) => suite.suite === "isolate",
  );
  isolate.metrics = isolate.metrics.filter(
    (metric) => metric.name !== "warm_create_ms",
  );

  const board = buildAdmissionBoard(snapshot);
  assert.equal(board.status, "unpublished");
  assert.ok(board.rows.every((row) => row.status === "unpublished"));
});

test("inconsistent gate decisions invalidate the snapshot", () => {
  const snapshot = readJson("data/benchmarks/admission-linux-x64.sample.json");
  const isolate = snapshot.evidence.suites.find(
    (suite) => suite.suite === "isolate",
  );
  const warm = isolate.metrics.find((metric) => metric.name === "warm_create_ms");
  warm.passed = false;
  warm.status = "fail";

  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");
});

test("suite provenance must match the evidence commit and system", () => {
  const snapshot = readJson("data/benchmarks/admission-linux-x64.sample.json");
  snapshot.evidence.suites[0].commit = "f".repeat(40);

  assert.equal(buildAdmissionBoard(snapshot).status, "unpublished");
});

test("every publicly rendered runtime suite and metric must be unique", () => {
  const duplicateSuite = readJson(
    "data/benchmarks/admission-linux-x64.sample.json",
  );
  const http = duplicateSuite.evidence.suites.find(
    (suite) => suite.suite === "http",
  );
  duplicateSuite.evidence.suites.push(structuredClone(http));
  assert.equal(buildAdmissionBoard(duplicateSuite).status, "unpublished");

  const duplicateMetric = readJson(
    "data/benchmarks/admission-linux-x64.sample.json",
  );
  const duplicateHttp = duplicateMetric.evidence.suites.find(
    (suite) => suite.suite === "http",
  );
  duplicateHttp.metrics.push(structuredClone(duplicateHttp.metrics[0]));
  assert.equal(buildAdmissionBoard(duplicateMetric).status, "unpublished");
});

test("legacy unwrapped evidence is not treated as a published claim", () => {
  const snapshot = readJson("data/benchmarks/admission-linux-x64.sample.json");
  assert.equal(buildAdmissionBoard(snapshot.evidence).status, "unpublished");
});

test("raw publication uses the same fail-closed decision as the page", () => {
  const valid = readJson("data/benchmarks/admission-linux-x64.sample.json");
  assert.equal(validatedBenchmarkSnapshot(valid), valid);

  const invalid = structuredClone(valid);
  invalid.evidence.suites.push(
    structuredClone(
      invalid.evidence.suites.find((suite) => suite.suite === "http"),
    ),
  );
  assert.deepEqual(validatedBenchmarkSnapshot(invalid), {
    status: "unpublished",
  });
});
