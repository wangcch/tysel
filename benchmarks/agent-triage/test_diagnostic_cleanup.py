#!/usr/bin/env python3
"""Offline fault injection for diagnostic finalization; no workload evidence.

Exercise real storage/profile entry points with tiny synthetic observations.
The optimized-mode entry guards have their own subprocess tests in test_sampling;
only those guards are patched here so the finalization tests also run under -O.
"""
from contextlib import ExitStack, redirect_stdout
import copy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location("cleanup_test_" + name, HERE / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def exception_chain(error):
    pending, seen = [error], set()
    while pending:
        item = pending.pop()
        if item is None or id(item) in seen:
            continue
        seen.add(id(item))
        yield item
        pending.extend((item.__cause__, item.__context__))
        pending.extend(getattr(item, "exceptions", ()))


class DiagnosticCleanupTests(unittest.TestCase):
    def run_driver(self, kind, *, measurement_error=None, close_error=None,
                   save_error=None, log_error=None, system_error=None):
        driver = load(kind)
        with tempfile.TemporaryDirectory(prefix="diagnostic-cleanup-test-") as temp:
            root = Path(temp)
            args = SimpleNamespace(
                bin_dir=root / "bin", baseline_release=root / "baseline/lookup",
                candidate_release=root / "candidate/lookup", bind_root=root / "bind",
                volume_root=root / "volume", output=root / "output", plan=root / "plan.json",
                source=root / "source", state_root=None)
            args.source.mkdir()
            protocol = dict(rounds=1, sizes=[dict(name="small"), dict(name="bounded")])
            plan = dict(originalProtocolSha256="protocol-sha", pairedBlocks=4,
                        sizes=["small", "bounded"],
                        callerArtifactSha256=dict(baseline="caller-sha", candidate="caller-sha"),
                        layouts=[dict(name=name, volume=[0, 0, 0]) for name in
                                 ("all-bind", "config-volume", "jobs-volume", "durable-volume", "all-volume")])
            args.plan.write_text(json.dumps(plan))
            snapshots, attempts, calls, closed = [], [], [], []

            class Support:
                cleanup = []

                def __init__(self, *unused):
                    pass

                def call(self, **kwargs):
                    calls.append(kwargs["op"])
                    if kwargs["op"] == "system" and system_error is not None:
                        raise system_error
                    return {"e2e": {"p95": 1}} if kwargs["op"] == "stats" else {"test": True}

                def close(self):
                    closed.append(True)
                    if close_error is not None:
                        raise close_error

            class Response:
                status = 200
                headers = {"x-p51-profile": json.dumps([dict(category="business", ms=1)])}

                def __enter__(self):
                    return self

                def __exit__(self, *unused):
                    pass

                def read(self):
                    return b"{}"

            def warm_round(*unused):
                if measurement_error is not None:
                    raise measurement_error
                if kind == "profile-caller":
                    driver.bench.deploy.http("http://offline.invalid", "/test")
                return dict(round=0, jobsPerSec=1, samples=[dict(e2eMs=1)])

            def save(path, data):
                snapshot = copy.deepcopy(data)
                attempts.append(snapshot)
                if save_error is not None and data.get("status") in ("complete", "error"):
                    raise save_error
                snapshots.append(snapshot)

            read_text, read_bytes, write_text = Path.read_text, Path.read_bytes, Path.write_text

            def mock_read_text(path, *a, **kw):
                if path == HERE / "protocol.json":
                    return json.dumps(protocol)
                if path == Path("/proc/self/mountinfo"):
                    return "synthetic mount metadata"
                return read_text(path, *a, **kw)

            def mock_read_bytes(path):
                return json.dumps(protocol).encode() if path == HERE / "protocol.json" else read_bytes(path)

            def mock_write_text(path, *a, **kw):
                if path == args.output.resolve() / "process.log" and log_error is not None:
                    raise log_error
                return write_text(path, *a, **kw)

            original_application = driver.bench.application
            original_process = driver.bench.deploy.Process
            original_http = driver.bench.deploy.http
            with ExitStack() as stack:
                for obj, name, value in (
                    (driver.argparse.ArgumentParser, "parse_args", lambda _: args),
                    (Path, "read_text", mock_read_text),
                    (Path, "read_bytes", mock_read_bytes),
                    (Path, "write_text", mock_write_text),
                    (driver.bench, "require_assertions", lambda: None),
                    (driver.bench, "sha", lambda _: "protocol-sha"),
                    (driver.bench.deploy, "verify", lambda _: {"artifacts": {"caller": {"sha256": "caller-sha"}}}),
                    (driver.bench, "Support", Support),
                    (driver.bench, "warm_round", warm_round),
                    (driver.bench, "save", save),
                ):
                    stack.enter_context(patch.object(obj, name, value))
                if kind == "storage-ab":
                    stack.enter_context(patch.object(driver, "comparisons", return_value={"targetDecision": []}))
                else:
                    stack.enter_context(patch.object(driver, "instrument"))
                    stack.enter_context(patch.object(driver.bench.package, "package", return_value={"test": True}))
                    stack.enter_context(patch.object(driver.urllib.request, "build_opener",
                                                    return_value=SimpleNamespace(open=lambda *a, **kw: Response())))
                raised = None
                try:
                    with redirect_stdout(io.StringIO()):
                        driver.main()
                except BaseException as error:
                    raised = error
            self.assertIs(driver.bench.application, original_application)
            self.assertIs(driver.bench.deploy.Process, original_process)
            self.assertIs(driver.bench.deploy.http, original_http)
            return SimpleNamespace(raised=raised, snapshots=snapshots, attempts=attempts,
                                   calls=calls, closed=closed)

    def assert_in_chain(self, raised, expected):
        self.assertIsNotNone(raised)
        self.assertTrue(any(item is expected for item in exception_chain(raised)),
                        f"{expected!r} missing from exception chain")

    def assert_in_diagnostic(self, raised, expected):
        self.assertIsNotNone(raised)
        self.assertIn(str(expected), "\n".join(str(item) for item in exception_chain(raised)))

    def test_success_remains_complete(self):
        for kind in ("storage-ab", "profile-caller"):
            with self.subTest(kind=kind):
                result = self.run_driver(kind)
                self.assertIsNone(result.raised)
                self.assertEqual(result.closed, [True])
                self.assertEqual(result.snapshots[-1]["status"], "complete")
                self.assertFalse(result.snapshots[-1].get("error"))
                self.assertFalse(result.snapshots[-1].get("cleanupErrors"))

    def test_close_failure_invalidates_success(self):
        for kind in ("storage-ab", "profile-caller"):
            with self.subTest(kind=kind):
                shutdown = RuntimeError("injected support shutdown failure")
                result = self.run_driver(kind, close_error=shutdown)
                self.assert_in_diagnostic(result.raised, shutdown)
                self.assertEqual(result.closed, [True])
                saved = result.snapshots[-1]
                self.assertEqual(saved["status"], "error")
                self.assertIn("shutdown", saved["error"])
                if kind == "storage-ab":
                    with self.assertRaisesRegex(ValueError, "incomplete/error"):
                        load("storage-report").summarize(saved)

    def test_measurement_and_close_failures_are_both_preserved(self):
        for kind in ("storage-ab", "profile-caller"):
            with self.subTest(kind=kind):
                primary = ValueError("injected measurement failure")
                shutdown = RuntimeError("injected support shutdown failure")
                result = self.run_driver(kind, measurement_error=primary, close_error=shutdown)
                self.assert_in_chain(result.raised, primary)
                self.assert_in_diagnostic(result.raised, shutdown)
                saved = result.snapshots[-1]
                self.assertEqual(saved["status"], "error")
                self.assertIn("measurement", saved["error"])
                self.assertIn("shutdown", json.dumps(saved["cleanupErrors"]))

    def test_final_save_failure_preserves_prior_failures(self):
        for kind in ("storage-ab", "profile-caller"):
            for primary_failed, shutdown_failed in ((True, False), (False, True), (True, True)):
                with self.subTest(kind=kind, primary=primary_failed, shutdown=shutdown_failed):
                    primary = ValueError("injected measurement failure") if primary_failed else None
                    shutdown = RuntimeError("injected support shutdown failure") if shutdown_failed else None
                    save = OSError("injected final evidence save failure")
                    result = self.run_driver(kind, measurement_error=primary, close_error=shutdown, save_error=save)
                    for error in (primary, shutdown, save):
                        if error is not None:
                            self.assert_in_diagnostic(result.raised, error)
                    if primary is not None:
                        self.assert_in_chain(result.raised, primary)
                    self.assertEqual(result.closed, [True])
                    self.assertTrue(result.attempts)
                    self.assertEqual(result.attempts[-1]["status"], "error")

    def test_final_save_failure_cannot_publish_a_success(self):
        for kind in ("storage-ab", "profile-caller"):
            with self.subTest(kind=kind):
                save = OSError("injected final evidence save failure")
                result = self.run_driver(kind, save_error=save)
                self.assert_in_diagnostic(result.raised, save)
                self.assertEqual(result.closed, [True])
                self.assertTrue(result.attempts)
                self.assertTrue(all(row.get("status") == "running" for row in result.snapshots))
                if kind == "profile-caller":
                    self.assertEqual(result.snapshots, [])

    def test_profile_log_failure_is_recorded_before_final_json(self):
        for primary_failed in (False, True):
            with self.subTest(primary=primary_failed):
                primary = ValueError("injected measurement failure") if primary_failed else None
                log = OSError("injected process.log write failure")
                result = self.run_driver("profile-caller", measurement_error=primary, log_error=log)
                self.assert_in_diagnostic(result.raised, log)
                if primary is not None:
                    self.assert_in_chain(result.raised, primary)
                self.assertEqual(result.closed, [True])
                saved = result.snapshots[-1]
                self.assertEqual(saved["status"], "error")
                self.assertIn("process.log", json.dumps(saved))

    def test_profile_log_and_save_failures_do_not_hide_measurement_or_close(self):
        primary = ValueError("injected measurement failure")
        shutdown = RuntimeError("injected support shutdown failure")
        log = OSError("injected process.log write failure")
        save = OSError("injected final evidence save failure")
        result = self.run_driver("profile-caller", measurement_error=primary,
                                 close_error=shutdown, log_error=log, save_error=save)
        for error in (primary, shutdown, log, save):
            self.assert_in_diagnostic(result.raised, error)
        self.assert_in_chain(result.raised, primary)
        self.assertEqual(result.closed, [True])
        self.assertTrue(result.attempts)
        self.assertEqual(result.attempts[-1]["status"], "error")
        self.assertIn("process.log", json.dumps(result.attempts[-1]))

    def test_storage_system_failure_closes_helper_and_saves_error(self):
        primary = OSError("injected system inspection failure")
        result = self.run_driver("storage-ab", system_error=primary)
        self.assert_in_chain(result.raised, primary)
        self.assertEqual(result.closed, [True])
        self.assertEqual(result.snapshots[-1]["status"], "error")
        self.assertIn("system inspection", result.snapshots[-1]["error"])


if __name__ == "__main__":
    unittest.main()
