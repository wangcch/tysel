#!/usr/bin/env python3
"""Offline O4 schedule, selection boundaries and observed-evidence regressions."""
import copy
import gzip
import importlib.util
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("dispatch_ab", HERE / "dispatch-ab.py")
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)
FIXTURES = HERE / "fixtures/dispatch"
PLAN = json.loads((FIXTURES / "o4-predeclared.json").read_bytes())


def pairs():
    return [dict(pair=row["pair"], size=row["size"], adapterDelayMs=row["adapterDelayMs"],
                 concurrency=row["concurrency"], throughputRatio=1.15 if row["concurrency"] == 4 else .95,
                 p95Ratio=1.05, controlRequestsPerJobRatio=2.0)
            for row in driver.schedule(PLAN)[::2]]


def all_passed(rows):
    return all(cell["passed"] for cell in driver.paired_selection(rows, PLAN))


def synthetic_evidence():
    # Historical samples provide realistic audit/read shapes; all performance
    # values and identities below are deliberately synthetic test inputs.
    archive = FIXTURES / "o3-measurements.json.gz"
    with gzip.open(archive, "rt") as stream:
        old = json.load(stream)
    templates = {(row["size"], row["concurrency"], row["adapterDelayMs"]): row
                 for row in old["warm"] if row["variant"] == "lookup"}
    result = dict(status="complete", supportClosed=True, plan=copy.deepcopy(PLAN), planSha256=driver.PLAN_SHA256,
                  protocol=old["protocol"], protocolSha256=old["protocolSha256"], rounds=[], processCleanup=[])
    for identity in driver.schedule(PLAN):
        row = copy.deepcopy(templates[identity["size"], identity["concurrency"], identity["adapterDelayMs"]])
        row.update(identity, namespaceRemoved=True)
        for index, sample in enumerate(row["samples"]):
            sample["jobId"] = f"job-{row['sequence']}-{index}"
            sample["e2eMs"] = 400.0
            sample["excludingAdapterSleepMs"] = 400.0 - sample["actualSleepMs"]
            for event in sample["audit"]:
                event["job_id"] = sample["jobId"]
        row["durationMs"] = 16000.0 if row["concurrency"] == 1 else (4000.0 if row["sourceVariant"] == "baseline" else 4000 / 1.2)
        row["jobsPerSec"] = 32000 / row["durationMs"]
        row["p95Ms"] = 400.0
        row["stats"] = dict(samples=[400.0] * 32, p50=400.0, p95=400.0, p99=None, p50Ci95=None)
        job_ids = [sample["jobId"] for sample in row["samples"]] + [f"warm-{row['sequence']}-{index}" for index in range(8)]
        observer = driver.Observer(None)
        for job in job_ids:
            for kind, status, method in (("admission", 202, "POST"), ("status", 200, "GET"), ("dispatch", 200, "POST")):
                observer.calls.append(dict(index=len(observer.calls), kind=kind, method=method, status=status,
                                           jobId=job, durationMs=1.0, duringStop=False))
        observer.calls.append(dict(index=len(observer.calls), kind="pending", method="GET", status=200, durationMs=1.0, duringStop=False))
        observer.next_index = len(observer.calls)
        row["httpObservation"] = observer.report()
        row["controlRequestsPerJob"] = 41 / 40
        result["rounds"].append(row)
        result["processCleanup"].append(dict(crash=False, liveProcessesRemaining=0,
                                            pids=sorted({pid for sample in row["memory"] for pid in sample["pids"]})))
    return result


class DispatchPairs(unittest.TestCase):
    def test_volume_protocol_loads_without_historical_archive_and_rejects_changes(self):
        source = (HERE / "protocol-volume.json").read_bytes()
        with tempfile.TemporaryDirectory(prefix="dispatch-protocol-") as tmp:
            root = Path(tmp)
            protocol_path = root / "protocol-volume.json"
            protocol_path.write_bytes(source)
            with patch.object(driver, "HERE", root):
                path, protocol = driver.load_volume_protocol(PLAN)
                self.assertEqual(path, protocol_path)
                self.assertEqual(protocol, json.loads(source))
                self.assertEqual(driver.bench.sha(path), PLAN["volumeProtocolSha256"])
                # Still valid JSON, but no longer the predeclared bytes.
                protocol_path.write_bytes(source + b"\n")
                with self.assertRaisesRegex(ValueError, "volume protocol changed"):
                    driver.load_volume_protocol(PLAN)

    def test_only_supervisor_artifact_may_differ(self):
        base = dict(sources={"src/index.ts": "source"}, toolchain={"releaseId": "id"},
                    artifacts={name: dict(sha256=name, bytes=10) for name in ("caller", "plugin", "tysel-worker", "deploy.py")})
        packages = {name: copy.deepcopy(base) for name in ("baseline", "candidate")}
        packages["candidate"]["artifacts"]["deploy.py"]["sha256"] = "candidate"
        driver.package_pair_check(packages)
        for name in ("caller", "plugin", "tysel-worker"):
            with self.subTest(artifact=name):
                changed = copy.deepcopy(packages)
                changed["candidate"]["artifacts"][name]["sha256"] = "different"
                with self.assertRaisesRegex(ValueError, "binary changed"):
                    driver.package_pair_check(changed)

    def test_schedule_is_adjacent_balanced_rotated_and_frozen(self):
        rows = driver.schedule(PLAN)
        self.assertEqual(len(rows), 48)
        self.assertEqual(len({row["pair"] for row in rows}), 24)
        self.assertEqual([row["sequence"] for row in rows], list(range(48)))
        for index in range(0, 48, 2):
            first, second = rows[index:index + 2]
            self.assertEqual(first["pair"], second["pair"])
            self.assertEqual({first["sourceVariant"], second["sourceVariant"]}, {"baseline", "candidate"})
            cell = PLAN["sizes"].index(first["size"]) * 2 + PLAN["adapterDelayMs"].index(first["adapterDelayMs"])
            self.assertEqual(first["sourceVariant"], "baseline" if (first["round"] + cell) % 2 == 0 else "candidate")
        self.assertEqual((rows[8]["size"], rows[8]["adapterDelayMs"]), ("small", 20))
        changed = copy.deepcopy(PLAN)
        changed["selection"]["eachC4Cell"]["medianThroughputRatioAtLeast"] = 1
        with self.assertRaisesRegex(ValueError, "plan changed"):
            driver.schedule(changed)

    def test_inclusive_thresholds_and_each_fail_side(self):
        self.assertTrue(all_passed(pairs()))
        for concurrency, key, bad in ((4, "throughputRatio", 1.15 - 1e-8),
                                      (1, "throughputRatio", .95 - 1e-8),
                                      (4, "p95Ratio", 1.05 + 1e-8), (1, "p95Ratio", 1.05 + 1e-8),
                                      (4, "controlRequestsPerJobRatio", 2 + 1e-8),
                                      (1, "controlRequestsPerJobRatio", 2 + 1e-8)):
            with self.subTest(concurrency=concurrency, key=key):
                rows = pairs()
                for row in rows:
                    if row["concurrency"] == concurrency and row["size"] == "small" and row["adapterDelayMs"] == 0:
                        row[key] = bad
                self.assertFalse(all_passed(rows))
        rows = pairs()
        cell = [row for row in rows if row["concurrency"] == 4 and row["size"] == "small" and row["adapterDelayMs"] == 0]
        for row, value in zip(cell, (1.0, 1.15, 1.15, 1.3)):
            row["throughputRatio"] = value
        self.assertTrue(all_passed(rows))
        cell[0]["throughputRatio"] = 1 - 1e-8
        self.assertFalse(all_passed(rows))

    def test_pair_shape_nonfinite_and_identity_rejected(self):
        for mutation in (lambda rows: rows.pop(), lambda rows: rows.append(copy.deepcopy(rows[0])),
                         lambda rows: rows[1].update(pair=rows[0]["pair"]),
                         lambda rows: rows[0].update(size="unknown"),
                         lambda rows: rows[0].update(throughputRatio=float("nan")),
                         lambda rows: rows[0].update(p95Ratio=0)):
            with self.subTest(mutation=mutation):
                rows = pairs()
                mutation(rows)
                with self.assertRaises(ValueError):
                    driver.paired_selection(rows, PLAN)

    def test_exact_package_module_and_observer_restored_on_exception(self):
        original = driver.bench.deploy
        modules = [types.SimpleNamespace(http=lambda *args: (200, {}), Deployment=type(name, (), {"close": lambda self: None}))
                   for name in ("Baseline", "Candidate")]
        for module in modules:
            http = module.http
            with self.assertRaisesRegex(RuntimeError, "injected"):
                with driver.packaged_deploy(module) as observer:
                    self.assertIs(driver.bench.deploy, module)
                    self.assertIs(module.http, observer)
                    self.assertIs(driver.bench.deploy.Deployment, module.Deployment)
                    module.http("http://local", "/internal/pending", token="secret", key="private-key")
                    raise RuntimeError("injected")
            self.assertIs(module.http, http)
            self.assertIs(driver.bench.deploy, original)
            text = json.dumps(observer.report())
            self.assertNotIn("secret", text)
            self.assertNotIn("private-key", text)
            self.assertNotIn("http://local", text)

    def test_observer_counts_success_error_and_only_safe_identity(self):
        def http(origin, path, *args):
            if path == "/fail":
                raise OSError("do not retain credentials")
            return 202, {"jobId": "job-1", "token": "not-retained"}
        observer = driver.Observer(http)
        observer("local", "/jobs", "POST", {"secret": "not-retained"}, "not-retained")
        with self.assertRaises(OSError):
            observer("local", "/fail")
        record = observer.report()
        self.assertEqual(record["started"], 2)
        self.assertEqual(record["calls"][0]["jobId"], "job-1")
        self.assertEqual(record["calls"][1]["errorType"], "OSError")
        self.assertNotIn("not-retained", json.dumps(record))
        self.assertNotIn("credentials", json.dumps(record))

    def test_full_evidence_validation_and_budget_gate(self):
        data = synthetic_evidence()
        result = driver.comparisons(data)
        self.assertTrue(result["pairedCriteriaPassed"])
        self.assertTrue(result["eligibleForFullValidation"])
        self.assertFalse(result["retainCandidate"])
        self.assertEqual(len(result["roundBudgets"]), 48)
        # A consistent PSS budget miss is valid evidence but cannot advance.
        row = data["rounds"][0]
        for sample in row["memory"]:
            sample["valueKiB"] = 524289
        row["idlePssKiB"] = row["peakPssKiB"] = 524289
        self.assertFalse(driver.comparisons(data)["eligibleForFullValidation"])

    def test_helper_float_roundtrip_preserves_length_order_and_numeric_tolerance(self):
        data = synthetic_evidence()
        row = data["rounds"][0]
        sample = row["samples"][0]
        # Actual Python/Rust JSON round-trip pair observed in the retained AB1
        # failure. The archived failure itself is never modified by this test.
        sample["e2eMs"] = 407.65244903741404
        sample["excludingAdapterSleepMs"] = sample["e2eMs"] - sample["actualSleepMs"]
        row["stats"]["samples"][0] = 407.6524490374141
        self.assertTrue(driver.comparisons(data)["eligibleForFullValidation"])
        for mutation in (lambda values: values.pop(), lambda values: values.append(400.0),
                         lambda values: values.__setitem__(slice(0, 2), values[:2][::-1]),
                         lambda values: values.__setitem__(0, values[0] + .001)):
            with self.subTest(mutation=mutation):
                changed = copy.deepcopy(data)
                mutation(changed["rounds"][0]["stats"]["samples"])
                with self.assertRaises(ValueError):
                    driver.comparisons(changed)

    def test_control_503_and_shutdown_cancel_are_counted_without_weakening_live_failure(self):
        data = synthetic_evidence()
        row = data["rounds"][0]
        observation = row["httpObservation"]
        # Keep identical total attempted controls, including the stop boundary.
        pending = observation["calls"][-1]
        for status, stopping, error, passes in ((503, False, None, True),
                                               (None, True, "ConnectionResetError", True),
                                               (None, False, "ConnectionResetError", False),
                                               (500, False, None, False)):
            with self.subTest(status=status, stopping=stopping):
                pending.update(status=status, duringStop=stopping)
                pending.pop("errorType", None)
                if error:
                    pending["errorType"] = error
                observation["statusCounts"] = dict(driver.Counter(str(call["status"]) for call in observation["calls"]))
                if passes:
                    driver.observation_check(row, 8, 32)
                else:
                    with self.assertRaises(ValueError):
                        driver.observation_check(row, 8, 32)

    def test_incomplete_or_inconsistent_raw_evidence_cannot_advance(self):
        data = synthetic_evidence()
        mutations = (
            lambda value: value["rounds"].pop(),
            lambda value: value["rounds"][0].update(sourceVariant="candidate"),
            lambda value: value["rounds"][0]["samples"].pop(),
            lambda value: value["rounds"][0].update(physicalReadsIncludingWarmup=39),
            lambda value: value["rounds"][0].update(namespaceRemoved=False),
            lambda value: value["processCleanup"][0].update(liveProcessesRemaining=1),
            lambda value: value["rounds"][0].update(jobsPerSec=1000),
            lambda value: value["rounds"][0].update(p95Ms=1),
            lambda value: value["rounds"][0].update(controlRequestsPerJob=1),
            lambda value: value["rounds"][0]["httpObservation"]["calls"].pop(),
            lambda value: value["rounds"][0]["samples"][0]["adapterAttempts"][0].update(authorized=False),
            lambda value: value.update(cleanupErrors=["helper close failed"]),
            lambda value: value.update(error="primary failure"),
            lambda value: value.update(status="running"),
            lambda value: value.update(status="error"),
            lambda value: value.update(supportClosed=False),
            lambda value: value.update(inFlightRound={}),
        )
        for index, mutate in enumerate(mutations):
            with self.subTest(case=index):
                candidate = copy.deepcopy(data)
                mutate(candidate)
                with self.assertRaises(ValueError):
                    driver.comparisons(candidate)


if __name__ == "__main__":
    unittest.main(verbosity=2)
