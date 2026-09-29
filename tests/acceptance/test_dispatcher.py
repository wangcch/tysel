#!/usr/bin/env python3
"""Offline supervisor scheduling tests using real threads and Futures.

HTTP and child processes are fakes. These tests cover bounded dispatch and
fallback behavior, not application/Durable correctness or throughput claims.
No containers, sockets or executables are started.
"""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import importlib.util
import json
from pathlib import Path
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[2]


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, REPO / "examples/agent-triage" / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PACKAGED = load("dispatcher_packaged", "deploy.py")
DEMO = load("dispatcher_demo", "run.py")
DRIVERS = (("packaged", PACKAGED, PACKAGED.Deployment), ("demo", DEMO, DEMO.Demo))


class RecordedEvent:
    """Record the requested waits without changing real Event semantics."""
    def __init__(self):
        self.event = threading.Event()
        self.waits = []
        self.lock = threading.Lock()

    def set(self):
        self.event.set()

    def clear(self):
        self.event.clear()

    def is_set(self):
        return self.event.is_set()

    def wait(self, timeout=None):
        with self.lock:
            self.waits.append((time.monotonic(), timeout))
        return self.event.wait(timeout)


class FakeChild:
    def __init__(self, *args, **kwargs):
        self.origin = "http://offline.invalid"
        self.logs = []
        self.exit_code = None
        self.process = SimpleNamespace(poll=lambda: self.exit_code)

    def close(self, *args, **kwargs):
        self.exit_code = 0


class Harness:
    def __init__(self, kind, module, cls):
        self.kind, self.module = kind, module
        self.app = cls.__new__(cls)
        self.app.caller, self.app.plugin = FakeChild(), FakeChild()
        self.app.origin = self.app.caller.origin
        self.app.config = {"dispatchToken": "offline-dispatch-token"}
        self.app.driver = None
        self.app.stop = RecordedEvent()
        self.app.driver_stop = self.app.stop
        self.app.errors = []
        self.app.driver_errors = self.app.errors
        self.app.dispatch_observations = []
        self.events, self.futures, self.scans, self.dispatches = [], [], [], []
        self.gates = []
        self.lock = threading.Lock()
        self.pending = lambda: (200, [])
        self.dispatch = lambda job: (200, {"status": "completed"})

    def new_event(self):
        event = RecordedEvent()
        self.events.append(event)
        return event

    def gate(self):
        gate = threading.Event()
        self.gates.append(gate)
        return gate

    def http(self, origin, path, method="GET", body=None, token=None, **kwargs):
        if token != self.app.config["dispatchToken"]:
            raise RuntimeError("dispatcher omitted authentication")
        if path == "/internal/pending":
            with self.lock:
                self.scans.append(time.monotonic())
            return self.pending()
        if path == "/internal/dispatch":
            if method != "POST":
                raise RuntimeError("dispatch did not use POST")
            with self.lock:
                self.dispatches.append((time.monotonic(), body["jobId"]))
            return self.dispatch(body["jobId"])
        raise RuntimeError("unexpected HTTP path " + path)

    @contextmanager
    def patched(self):
        harness = self

        class Pool(ThreadPoolExecutor):
            def __init__(self, max_workers):
                if max_workers != 8:
                    raise RuntimeError("dispatcher slot limit changed")
                super().__init__(max_workers=max_workers)

            def submit(self, fn, *args, **kwargs):
                future = super().submit(fn, *args, **kwargs)
                harness.futures.append(future)
                return future

        facade = SimpleNamespace(Event=self.new_event, Thread=threading.Thread)
        with patch.object(self.module, "http", side_effect=self.http), \
                patch.object(self.module, "ThreadPoolExecutor", Pool), \
                patch.object(self.module, "threading", facade):
            try:
                yield self
            finally:
                self.app.stop.set()
                self.app.driver_stop.set()
                for gate in self.gates:
                    gate.set()
                for event in self.events:
                    event.set()
                if self.app.driver:
                    self.app.driver.join(timeout=3)
                    if self.app.driver.is_alive():
                        raise RuntimeError("offline dispatcher thread did not stop")

    def start(self):
        self.app.start_driver()

    def stop(self):
        self.app.stop.set()
        self.app.driver_stop.set()
        self.app.driver.join(timeout=2)
        if self.app.driver.is_alive():
            raise RuntimeError("offline dispatcher thread did not stop")

    def waits(self):
        return sorted(wait for event in [self.app.stop] + self.events for wait in event.waits)


class DispatcherTests(unittest.TestCase):
    def until(self, predicate, description, timeout=2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.002)
        self.fail("timed out waiting for " + description)

    def wait_for_sleep(self, harness, seconds, after=0):
        self.until(lambda: any(at >= after and abs(wait - seconds) < .001
                               for at, wait in harness.waits() if wait is not None),
                   str(seconds) + " second supervisor wait")

    def test_completed_future_wakes_wait_and_enters_short_refill_window(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), Harness(kind, module, cls).patched() as h:
                release = h.gate()
                h.pending = lambda: (200, [{"id": "a"}] if not release.is_set() else [])
                h.dispatch = lambda job: (release.wait(2) and 200, {"status": "completed"})
                h.start()
                self.until(lambda: len(h.dispatches) == 1, "first dispatch")
                self.wait_for_sleep(h, .25)
                before = len(h.scans)
                completed_at = time.monotonic()
                release.set()
                self.until(lambda: len(h.scans) > before, "completion-driven refill")
                self.assertLess(h.scans[before] - completed_at, .20,
                                "completed dispatch waited for the old 250 ms cadence")
                self.wait_for_sleep(h, .05, completed_at)
                self.assertEqual(h.app.errors, [])

    def test_noncompleted_outcomes_keep_the_retry_delay(self):
        outcomes = ((503, {"error": "busy"}), (503, {"status": "completed"}), (200, {"status": "accepted"}),
                    (200, {"status": "suspended"}), (200, {"dispatched": False}))
        for kind, module, cls in DRIVERS:
            for status, body in outcomes:
                with self.subTest(driver=kind, outcome=body), Harness(kind, module, cls).patched() as h:
                    h.pending = lambda: (200, [{"id": "a"}])
                    h.dispatch = lambda job: (status, body)
                    h.start()
                    self.until(lambda: len(h.dispatches) >= 2, "delayed retry")
                    self.assertGreaterEqual(h.dispatches[1][0] - h.dispatches[0][0], .235)
                    self.assertFalse(any(wait == .05 for _, wait in h.waits()))
                    self.assertEqual(h.app.errors, [])

    def test_slots_and_duplicate_pending_rows_never_duplicate_inflight_job(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), Harness(kind, module, cls).patched() as h:
                release = h.gate()
                h.pending = lambda: (200, [{"id": str(i)} for i in range(12) for _ in range(2)])
                h.dispatch = lambda job: (release.wait(2) and 200, {"status": "accepted"})
                h.start()
                self.until(lambda: len(h.dispatches) == 8, "all eight slots occupied")
                self.until(lambda: len(h.scans) >= 3, "multiple scans with blocked jobs")
                self.assertEqual(len(h.dispatches), 8)
                self.assertEqual(len({job for _, job in h.dispatches}), 8)
                self.assertEqual(len(h.futures), 8)

    def test_pending_slow_job_does_not_extend_completion_burst(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), Harness(kind, module, cls).patched() as h:
                release = h.gate()
                h.pending = lambda: (200, [{"id": "slow"}] if len(h.dispatches) else [{"id": "fast"}, {"id": "slow"}])
                h.dispatch = lambda job: (200, {"status": "completed"}) if job == "fast" else (release.wait(2) and 200, {"status": "accepted"})
                h.start()
                self.wait_for_sleep(h, .05)
                first_short = next(at for at, wait in h.waits() if wait == .05)
                self.wait_for_sleep(h, .25, first_short + .20)
                last_wait = h.waits()[-1]
                self.assertLess(last_wait[0] - first_short, .55)
                self.assertEqual([job for _, job in h.dispatches].count("slow"), 1)
                self.assertEqual(h.app.errors, [])

    def test_other_completions_do_not_hot_retry_an_unresolved_job(self):
        for kind, module, cls in DRIVERS:
            for outcome in ((503, {"error": "busy"}), (200, {"status": "accepted"}), (200, {"status": "suspended"})):
                with self.subTest(driver=kind, outcome=outcome), Harness(kind, module, cls).patched() as h:
                    completed = set()
                    def pending():
                        return 200, [{"id": "retry"}] + [{"id": "fast-" + str(i)} for i in range(3) if i not in completed]
                    def dispatch(job):
                        if job == "retry":
                            return outcome
                        time.sleep(.06)
                        completed.add(int(job.split("-")[1]))
                        return 200, {"status": "completed"}
                    h.pending, h.dispatch = pending, dispatch
                    h.start()
                    self.until(lambda: sum(job == "retry" for _, job in h.dispatches) >= 2, "retry amid completions")
                    attempts = [at for at, job in h.dispatches if job == "retry"]
                    self.assertGreaterEqual(attempts[1] - attempts[0], .235)
                    self.assertTrue(any(wait == .05 for _, wait in h.waits()))

    def test_idle_and_unavailable_pending_lookup_use_fallback_wait(self):
        for kind, module, cls in DRIVERS:
            for result in ((200, []), (503, {"error": "unavailable"})):
                with self.subTest(driver=kind, status=result[0]), Harness(kind, module, cls).patched() as h:
                    h.pending = lambda: result
                    h.start()
                    self.until(lambda: len(h.scans) >= 3, "fallback scans")
                    self.assertTrue(all(b - a >= .235 for a, b in zip(h.scans, h.scans[1:])))
                    self.assertEqual(h.dispatches, [])
                    self.assertEqual(h.app.errors, [])

    def test_lookup_and_future_errors_are_reported(self):
        for kind, module, cls in DRIVERS:
            for fault in ("lookup-status", "lookup-exception", "dispatch-status", "dispatch-exception"):
                with self.subTest(driver=kind, fault=fault), Harness(kind, module, cls).patched() as h:
                    def lookup():
                        if fault == "lookup-exception":
                            raise OSError("offline injected lookup failure")
                        return (401, {}) if fault == "lookup-status" else (200, [{"id": "a"}])
                    def dispatch(job):
                        if fault == "dispatch-exception":
                            raise OSError("offline injected dispatch failure")
                        return 401, {}
                    h.pending, h.dispatch = lookup, dispatch
                    h.start()
                    self.until(lambda: bool(h.app.errors), "visible dispatcher failure")
                    self.until(lambda: not h.app.driver.is_alive(), "failed dispatcher to exit")
                    self.assertEqual(len(h.app.errors), 1)
                    self.assertIn("401" if fault.endswith("status") else "offline injected", h.app.errors[0])

    def test_unavailable_lookup_after_completion_exits_fast_refill(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), Harness(kind, module, cls).patched() as h:
                release = h.gate()
                h.pending = lambda: (503, {"error": "busy"}) if release.is_set() else (200, [{"id": "a"}])
                h.dispatch = lambda job: (release.wait(2) and 200, {"status": "completed"})
                h.start()
                self.until(lambda: len(h.dispatches) == 1, "first dispatch")
                self.wait_for_sleep(h, .25)
                before = len(h.scans)
                release.set()
                self.until(lambda: len(h.scans) >= before + 3, "unavailable fallback scans")
                following = h.scans[before:]
                self.assertTrue(all(b - a >= .235 for a, b in zip(following, following[1:])),
                                "pending lookup failure kept the fast scan cadence")
                self.assertEqual(h.app.errors, [])

    def test_packaged_dispatcher_retains_child_liveness_checks(self):
        for child in ("caller", "plugin"):
            with self.subTest(child=child), Harness("packaged", PACKAGED, PACKAGED.Deployment).patched() as h:
                getattr(h.app, child).exit_code = 9
                h.start()
                self.until(lambda: bool(h.app.errors), "deployment child failure")
                self.until(lambda: not h.app.driver.is_alive(), "failed deployment driver to exit")
                self.assertIn("deployment process exited", h.app.errors[0])
                self.assertEqual(h.scans, [])
                self.assertEqual(h.futures, [])

    def test_restart_uses_a_fresh_completion_event(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), Harness(kind, module, cls).patched() as h:
                h.start()
                self.wait_for_sleep(h, .25)
                self.assertEqual(len(h.events), 1, "completion wake Event must be local to one driver run")
                previous = h.events[0]
                h.stop()
                restarted_at = time.monotonic()
                h.start()
                self.wait_for_sleep(h, .25, restarted_at)
                self.assertEqual(len(h.events), 2)
                self.assertIsNot(previous, h.events[1])
                before = len(h.scans)
                previous.set()
                time.sleep(.10)
                self.assertEqual(len(h.scans), before, "old completion Event woke the restarted dispatcher")
                self.assertTrue(h.app.driver.is_alive())

    def test_dispatch_false_preserves_admission_only_control(self):
        for kind, module, cls in DRIVERS:
            with self.subTest(driver=kind), tempfile.TemporaryDirectory(prefix="tysel-offline-dispatch-") as temp:
                root = Path(temp)
                calls = []
                def http(origin, path, *args, **kwargs):
                    calls.append(path)
                    return (200, {"ready": True}) if path == "/health" else (202, {"jobId": "admitted"})
                if kind == "packaged":
                    state = root / "state"
                    (state / "caller/config").mkdir(parents=True)
                    metadata = {"artifacts": {name: {"sha256": str(i) * 64} for i, name in enumerate(module.ARTIFACTS)}}
                    binding = module.artifact_binding(metadata)
                    (state / "binding.json").write_text(json.dumps({"artifacts": binding}))
                    (state / "caller/config/service.json").write_text(json.dumps({"callerDigest": binding["caller"], "pluginDigest": binding["plugin"]}))
                    with patch.object(module, "verify", return_value=metadata), patch.object(module, "Process", FakeChild), patch.object(module, "http", side_effect=http):
                        with cls(root / "release", state, "offline-secret", dispatch=False) as app:
                            self.assertIsNone(app.driver)
                            module.http(app.origin, "/jobs", "POST", {})
                else:
                    class Fixture:
                        origin = "http://offline.invalid"
                        customers = []
                        gates = {}
                        release = threading.Event()
                        def close(self):
                            pass
                    with patch.object(module, "Fixture", Fixture), patch.object(module, "Child", FakeChild), patch.object(module, "http", side_effect=http):
                        with cls(root / "unused-binary", root / "unused-worker", dispatch=False) as app:
                            self.assertIsNone(app.driver)
                            self.assertEqual(app.submit("offline-admission")[0], 202)
                self.assertEqual(calls, ["/health", "/jobs"])


if __name__ == "__main__":
    unittest.main()
