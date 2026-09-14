#!/usr/bin/env python3
"""Failure-injection tests for the acceptance gate; no runtime or network required."""
import contextlib
import io
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import run as harness


class AcceptanceRunnerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="tysel-acceptance-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.output = self.root / "evidence"
        self.output.mkdir()

    def case(self, source, name="injected", timeout=5):
        script = self.root / f"{name}.py"
        script.write_text(source)
        return harness.Case(name, str(script), timeout=timeout)

    def test_assertion_failure_blocks_the_gate_and_retains_diagnostics(self):
        case = self.case("""import os
from pathlib import Path
root = Path(os.environ['TMPDIR'])
(root / 'service.log').write_text('recovery never completed')
(root / 'app').write_bytes(b'not for upload')
assert False, 'injected recovery regression'
""")
        report = {"cases": []}
        with contextlib.redirect_stdout(io.StringIO()):
            code = harness.execute_cases([case, self.case("raise SystemExit(0)", "later")],
                                         self.root, self.output, report)
        self.assertEqual(code, 1)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(len(report["cases"]), 1)
        self.assertIn("injected recovery regression", (self.output / "injected/runner.log").read_text())
        self.assertEqual((self.output / "injected/fixtures/service.log").read_text(), "recovery never completed")
        self.assertFalse((self.output / "injected/fixtures/app").exists())
        self.assertEqual(json.loads((self.output / "injected/case.json").read_text())["exitCode"], 1)

    def test_timeout_kills_the_entire_case_process_group(self):
        case = self.case("""import subprocess, sys, time
from pathlib import Path
child = subprocess.Popen([sys.executable, '-c',
    'import signal,time;signal.signal(signal.SIGTERM,signal.SIG_IGN);time.sleep(60)'])
Path('child.pid').write_text(str(child.pid))
print('fixture reached hang', flush=True)
time.sleep(60)
""", timeout=1)
        result = harness.run_case(case, self.root, self.output)
        self.assertEqual(result["status"], "timed_out")
        self.assertLess(result["elapsedSeconds"], 5)
        pid = (self.output / "injected/child.pid").read_text()
        state = subprocess.run(["ps", "-p", pid, "-o", "stat="], capture_output=True, text=True).stdout.strip()
        # A killed orphan can briefly be a zombie until the host's init reaps it.
        self.assertTrue(not state or state.startswith("Z"), state)
        self.assertIn("fixture reached hang", (self.output / "injected/runner.log").read_text())

    def test_host_environment_cannot_disable_assertions_or_reduce_coverage(self):
        case = self.case("""import os
assert 'PYTHONOPTIMIZE' not in os.environ
assert 'TYSEL_GATE_STANDALONE_ONLY' not in os.environ
assert 'TYSEL_P1_USE_RUN' not in os.environ
assert os.environ['TYSEL_GATE_CRASH_MODES'] == 'after_commit'
assert os.environ['OTEL_SDK_DISABLED'] == 'true'
""")
        case.modes = ("after_commit",)
        with patch.dict(os.environ, {"PYTHONOPTIMIZE": "1", "TYSEL_GATE_STANDALONE_ONLY": "1",
                                     "TYSEL_P1_USE_RUN": "1", "TYSEL_GATE_CRASH_MODES": "before_effect"}):
            result = harness.run_case(case, self.root, self.output)
        self.assertEqual(result["status"], "passed")

    def test_cleanup_failure_cannot_turn_into_a_pass(self):
        case = self.case("print('finished assertions')")
        with patch.object(harness, "stop_group", side_effect=OSError("injected cleanup failure")):
            result = harness.run_case(case, self.root, self.output)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["exitCode"], 0)
        self.assertIn("cleanup failure", result["error"])

    def test_missing_runtime_writes_failed_evidence_and_exits_nonzero(self):
        result = subprocess.run([sys.executable, str(Path(harness.__file__)), "--suite", "smoke",
                                 "--bin-dir", str(self.root / "missing"), "--profile", "debug",
                                 "--target", "linux-x64", "--output", str(self.output)],
                                capture_output=True, text=True, timeout=15)
        self.assertNotEqual(result.returncode, 0)
        evidence = json.loads((self.output / "evidence.json").read_text())
        self.assertEqual(evidence["status"], "failed")
        self.assertIn("source", evidence)

    def test_binary_provenance_rejects_stale_or_mixed_tools(self):
        def tools(commit):
            for name in harness.BINARIES:
                path = self.root / name
                info = {"schemaVersion": 1, "binary": name, "version": "0.3.0",
                        "target": "linux-x64", "sourceCommit": commit, "releaseId": "0.3.0"}
                path.write_text(f"#!{sys.executable}\nprint({json.dumps(info)!r})\n")
                path.chmod(0o755)
        tools("a" * 40)
        with self.assertRaisesRegex(ValueError, "checked-out commit"):
            harness.binary_identity(self.root, "linux-x64", "b" * 40, True)
        result = harness.binary_identity(self.root, "linux-x64", "a" * 40, True)
        self.assertEqual(len(result), 3)
        worker = self.root / "tysel-worker"
        worker.write_text(worker.read_text().replace("0.3.0", "0.4.0"))
        with self.assertRaisesRegex(ValueError, "same build identity"):
            harness.binary_identity(self.root, "linux-x64", "a" * 40, True)

    def test_release_matrix_includes_every_crash_window_and_backend(self):
        cases = harness.cases_for("release")
        crashes = [case for case in cases if case.modes]
        self.assertEqual({(case.backend, case.instances) for case in crashes},
                         {("sqlite", 1), ("postgres", 1), ("postgres", 2)})
        for case in crashes:
            self.assertEqual(len(case.modes), 6)
            self.assertEqual(set(case.modes), set(harness.CRASH_MODES))
        self.assertEqual([case.name for case in harness.cases_for("full")],
                         [case.name for case in cases])

    def test_empty_plan_cannot_pass(self):
        report = {"cases": []}
        self.assertEqual(harness.execute_cases([], self.root, self.output, report), 1)

    def test_release_admission_rejects_partial_or_unverified_plans(self):
        for flags, dirty in [(["--case", "workflows"], False), ([], True), ([], False)]:
            output = self.root / f"preflight-{len(list(self.root.iterdir()))}"
            args = ["run.py", "--suite", "release", "--bin-dir", str(self.root),
                    "--profile", "release", "--target", "linux-x64", "--output", str(output),
                    "--require-build-commit", *flags]
            with patch.object(sys, "argv", args), patch.object(harness, "binary_identity", return_value={}), \
                 patch.object(harness, "source_identity", return_value={"commit": "a" * 40, "dirty": dirty}), \
                 patch.dict(os.environ, {"TYSEL_GATE_POSTGRES_URL": ""}), \
                 patch.object(harness, "execute_cases") as execute, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(harness.main(), 1)
                execute.assert_not_called()
            self.assertEqual(json.loads((output / "evidence.json").read_text())["status"], "failed")

    def test_postgres_client_uses_environment_and_schema_cleanup_on_failure(self):
        url = "postgres://fixture:p%40ss@127.0.0.1:15432/acceptance?sslmode=disable"
        with patch.dict(os.environ, {"TYSEL_GATE_POSTGRES_URL": url}), \
             patch("tempfile.mkdtemp", return_value=str(self.root)), contextlib.redirect_stdout(io.StringIO()):
            module = runpy.run_path(str(harness.REPO / "tests/p1/crash_recovery.py"))
        with patch("subprocess.run", return_value=SimpleNamespace(returncode=0, stdout="ok", stderr="")) as client:
            self.assertEqual(module["pg_query"]("SELECT 1"), "ok")
            args, kwargs = client.call_args
            self.assertNotIn(url, args[0])
            self.assertEqual(kwargs["env"]["PGDATABASE"], "acceptance")
            self.assertEqual(kwargs["env"]["PGPASSWORD"], "p@ss")
            self.assertEqual(kwargs["env"]["PGPORT"], "15432")
            self.assertEqual(kwargs["env"]["PGSSLMODE"], "disable")
            self.assertEqual(kwargs["input"], "SELECT 1")
        commands = []
        with patch.dict(module["database_schema"].__wrapped__.__globals__, {"pg_query": commands.append}):
            with self.assertRaisesRegex(AssertionError, "injected"):
                with module["database_schema"]():
                    raise AssertionError("injected")
        self.assertEqual(len(commands), 2)
        schema = commands[0].removeprefix("CREATE SCHEMA ")
        self.assertEqual(commands[1], "DROP SCHEMA " + schema + " CASCADE")


if __name__ == "__main__":
    unittest.main()
