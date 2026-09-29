#!/usr/bin/env python3
"""Replay retained cost evidence and reject inconsistent copies without running Tysel."""
import copy
import gzip
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
FIXTURES = HERE / "fixtures/cost"
spec = importlib.util.spec_from_file_location("agent_cost_report", HERE / "report.py")
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


def pretend_pass(data):
    data["status"] = "passed"
    data["analysis"]["passed"] = True
    for check in data["analysis"]["checks"]:
        check["passed"] = True


def lookup(data):
    return next(row for row in data["warm"] if row["variant"] == "lookup")


def impossible_recovery(data):
    for row in data["recovery"]:
        row["recoveryMs"] = 1
    for check in data["analysis"]["checks"]:
        if check["metric"] == "recoveryMaxMs":
            check["actual"] = 1


class ReportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.archives = {
            phase: json.loads(gzip.decompress((FIXTURES / phase /
                                              "measurements.json.gz").read_bytes()))
            for phase in ("p5", "p51", "p52")
        }

    def test_historical_results_are_byte_identical(self):
        for phase, misses in (("p5", 14), ("p51", 15), ("p52", 0)):
            with self.subTest(phase=phase):
                data = copy.deepcopy(self.archives[phase])
                summary = report.summarize(data)
                self.assertEqual(len(summary["budgetMisses"]), misses)
                self.assertEqual(summary["numericalChecks"], 132)
                self.assertEqual(summary["status"], "budget_miss" if misses else "passed")
                directory = FIXTURES / phase
                self.assertEqual(json.dumps(summary, indent=2) + "\n", (directory / "summary.json").read_text())
                self.assertEqual(report.markdown(summary), (directory / "results.md").read_text())
                self.assertEqual(data, self.archives[phase], "verification must not rewrite its input")

    def test_saved_pass_flags_cannot_hide_real_budget_misses(self):
        data = copy.deepcopy(self.archives["p5"])
        pretend_pass(data)
        with self.assertRaises(ValueError):
            report.summarize(data)

    def test_rejects_inconsistent_evidence(self):
        mutations = {
            "empty_checks": lambda d: d["analysis"].update(checks=[]),
            "missing_check": lambda d: d["analysis"]["checks"].pop(),
            "duplicate_check": lambda d: d["analysis"]["checks"].append(copy.deepcopy(d["analysis"]["checks"][0])),
            "wrong_check_actual": lambda d: d["analysis"]["checks"][0].update(actual=0),
            "wrong_check_limit": lambda d: d["analysis"]["checks"][0].update(limit=999999),
            "wrong_check_relation": lambda d: d["analysis"]["checks"][0].update(relation=">="),
            "wrong_overall_status": lambda d: d.update(status="budget_miss"),
            "wrong_analysis_status": lambda d: d["analysis"].update(passed=False),
            "declared_error": lambda d: d.update(error="measurement failed"),
            "missing_report_metadata": lambda d: d.pop("sourceCommit"),
            "missing_round": lambda d: d["warm"].pop(),
            "duplicate_round": lambda d: d["warm"].__setitem__(1, copy.deepcopy(d["warm"][0])),
            "missing_sample": lambda d: d["warm"][0]["samples"].pop(),
            "duplicate_cold": lambda d: d["cold"].__setitem__(1, copy.deepcopy(d["cold"][0])),
            "missing_cold": lambda d: d["cold"].pop(),
            "duplicate_recovery": lambda d: d["recovery"].__setitem__(1, copy.deepcopy(d["recovery"][0])),
            "wrong_rate": lambda d: d["warm"][0].update(jobsPerSec=999999),
            "wrong_duration": lambda d: d["warm"][0].update(durationMs=1),
            "wrong_peak_memory": lambda d: d["warm"][0].update(peakPssKiB=0),
            "missing_memory": lambda d: d["warm"][0].update(memory=[]),
            "wrong_memory_kind": lambda d: d["warm"][0]["memory"][0].update(kind="rss"),
            "nan_sample": lambda d: d["warm"][0]["samples"][0].update(e2eMs=float("nan")),
            "infinite_sample": lambda d: d["warm"][0]["samples"][0].update(e2eMs=float("inf")),
            "negative_sample": lambda d: d["warm"][0]["samples"][0].update(e2eMs=-1),
            "invalid_client": lambda d: d["warm"][0]["samples"][0].update(client=99),
            "wrong_distribution": lambda d: next(iter(d["analysis"]["distributions"].values())).update(p95=999999),
            "missing_distribution": lambda d: d["analysis"]["distributions"].pop(next(iter(d["analysis"]["distributions"]))),
            "extra_distribution": lambda d: d["analysis"]["distributions"].update(unexpected={}),
            "wrong_protocol_hash": lambda d: d.update(protocolSha256="0" * 64),
            "changed_budget": lambda d: d["protocol"]["budgets"]["lookup"].update(warmP95Ms=999999),
            "changed_polling": lambda d: d["protocol"].update(statusPollMs=500),
            "missing_cleanup": lambda d: d.update(processCleanup=[]),
            "live_process": lambda d: d["processCleanup"][0].update(liveProcessesRemaining=1),
            "cleanup_error": lambda d: d["processCleanup"][0].update(error="cleanup failed"),
            "missing_denial": lambda d: d["denials"]["probes"].pop(),
            "denial_reached_adapter": lambda d: d["denials"].update(physicalReads=1),
            "healthy_extra_read": lambda d: lookup(d).update(physicalReadsIncludingWarmup=41),
            "unauthorized_read": lambda d: lookup(d)["samples"][0]["adapterAttempts"][0].update(authorized=False),
            "wrong_read_scope": lambda d: lookup(d)["samples"][0]["adapterAttempts"][0].update(customerId="other"),
            "wrong_lease": lambda d: d["recovery"][0].update(defaultLeaseMs=100),
            "reset_deadline": lambda d: d["recovery"][0].update(originalDeadlinePreserved=False),
            "missing_recovery_read": lambda d: d["recovery"][0]["physicalReads"].pop(),
            "duplicate_recovery_read": lambda d: d["recovery"][0]["physicalReads"].__setitem__(
                1, copy.deepcopy(d["recovery"][0]["physicalReads"][0])),
            "impossible_recovery_time": impossible_recovery,
            "missing_attempt_owner": lambda d: d["recovery"][0]["slots"][0].update(owner=None),
        }
        for name, mutate in mutations.items():
            with self.subTest(mutation=name):
                data = copy.deepcopy(self.archives["p52"])
                mutate(data)
                with self.assertRaises(ValueError):
                    report.summarize(data)

    def run_cli(self, args, optimized=False):
        return subprocess.run([sys.executable, *(["-O"] if optimized else []), str(HERE / "report.py"), *args],
                              capture_output=True, text=True, timeout=20)

    def test_cli_rejects_bad_input_without_writing_reports(self):
        with tempfile.TemporaryDirectory(prefix="agent-report-test-") as tmp:
            root = Path(tmp)
            invalid = copy.deepcopy(self.archives["p52"])
            invalid["analysis"]["checks"] = []
            cases = [("bad.json", json.dumps(invalid).encode()), ("broken.json", b"{"),
                     ("broken.json.gz", b"not gzip"),
                     ("truncated.json.gz", gzip.compress(json.dumps(invalid).encode())[:20]),
                     ("duplicate.json", ('{"status":"passed",' + json.dumps(self.archives["p52"])[1:]).encode())]
            missing_metadata = copy.deepcopy(self.archives["p52"])
            missing_metadata.pop("sourceCommit")
            cases.append(("missing-metadata.json", json.dumps(missing_metadata).encode()))
            for name, payload in cases:
                with self.subTest(input=name):
                    path = root / name
                    path.write_bytes(payload)
                    output = root / (name + "-output")
                    result = self.run_cli(["--input", str(path), "--output", str(output)])
                    self.assertNotEqual(result.returncode, 0)
                    self.assertNotIn("Traceback", result.stderr)
                    self.assertFalse(output.exists())
            output = root / "existing"
            output.mkdir()
            (output / "summary.json").write_text("original evidence")
            result = self.run_cli(["--input", str(root / "bad.json"), "--output", str(output)], optimized=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertNotIn("Traceback", result.stderr)
            self.assertEqual((output / "summary.json").read_text(), "original evidence")
            self.assertEqual(sorted(p.name for p in output.iterdir()), ["summary.json"])

    def test_cli_optimized_python_preserves_valid_budget_miss(self):
        with tempfile.TemporaryDirectory(prefix="agent-report-test-") as tmp:
            output = Path(tmp) / "report"
            original = FIXTURES / "p5"
            result = self.run_cli(["--input", str(original / "measurements.json.gz"), "--output", str(output)], optimized=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((output / "summary.json").read_bytes(), (original / "summary.json").read_bytes())
            self.assertEqual((output / "results.md").read_bytes(), (original / "results.md").read_bytes())


if __name__ == "__main__":
    unittest.main(verbosity=2)
