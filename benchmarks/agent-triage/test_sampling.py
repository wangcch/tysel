#!/usr/bin/env python3
"""Offline sampling-driver failure tests; no Tysel build, container or network.

Small Python helpers exercise the actual process boundary. Main-loop tests use
synthetic observations, not performance evidence. AGENT_SAMPLING_TEST_SOURCE can
select a retained driver copy for before/after regression evidence.
"""
from contextlib import ExitStack, redirect_stdout
import importlib.util
import io
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import traceback
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


HERE = Path(__file__).resolve().parent
DRIVER = HERE / "run.py"
SPEC = importlib.util.spec_from_file_location("sampling_test_driver", DRIVER)
bench = importlib.util.module_from_spec(SPEC)
# Keep __file__/repository imports at their original locations for a retained
# baseline; only the selected driver's source bytes differ.
SOURCE = Path(os.environ.get("AGENT_SAMPLING_TEST_SOURCE", str(DRIVER)))
exec(compile(SOURCE.read_bytes(), str(SOURCE), "exec"), bench.__dict__)


def diagnostic(error):
    return "".join(traceback.format_exception(type(error), error, error.__traceback__))


def exception_chain(error):
    pending, seen = [error], set()
    while pending:
        current = pending.pop()
        if id(current) in seen:
            continue
        seen.add(id(current))
        yield current
        pending.extend(item for item in (current.__cause__, current.__context__) if item is not None)


class SamplingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="agent-sampling-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.helpers = []

    def helper(self, source):
        path = self.root / ("helper-" + str(len(self.helpers)) + ".py")
        ready = path.with_suffix(".ready")
        # Python startup on a developer host is not the request timeout under
        # test. Start the injected 0.2 s pipe deadline only after this barrier.
        path.write_text("#!" + sys.executable + "\nfrom pathlib import Path\nPath(" + repr(str(ready)) + ").touch()\n" + source)
        path.chmod(0o700)
        support = bench.Support(path, timeout=.2)
        self.helpers.append(support)
        self.addCleanup(self.rescue_helper, support)
        deadline = time.monotonic() + 5
        while not ready.exists() and time.monotonic() < deadline and support.process.poll() is None:
            time.sleep(.005)
        self.assertTrue(ready.exists(), "Python helper did not reach its startup barrier")
        return support

    def rescue_helper(self, support):
        # A failing regression must never strand a helper. Tests check production
        # cleanup before this test-only fallback is invoked.
        process = support.process
        if process.poll() is None:
            process.kill()
        process.wait(timeout=3)
        for stream in (process.stdin, process.stdout):
            if stream and not stream.closed:
                stream.close()

    def bounded(self, support, call, limit=3):
        completed = queue.Queue()
        def run():
            try:
                completed.put((call(), None))
            except BaseException as error:
                completed.put((None, error))
        thread = threading.Thread(target=run, daemon=True)
        started = time.monotonic()
        thread.start()
        thread.join(limit)
        if thread.is_alive():
            self.rescue_helper(support)
            thread.join(3)
            self.fail("support operation exceeded its test watchdog")
        self.assertLess(time.monotonic() - started, limit)
        return completed.get_nowait()

    def assert_reaped(self, support):
        self.assertIsNotNone(support.process.poll(), "support process was not reaped")
        self.assertTrue(support.process.stdin.closed, "support stdin remained open")
        self.assertTrue(support.process.stdout.closed, "support stdout remained open")

    def close_helper(self, support):
        result = self.bounded(support, lambda: support.close(timeout=.2))
        self.assert_reaped(support)
        return result

    def test_save_publishes_complete_json_once(self):
        target = self.root / "measurements.json"
        old = b'{"status":"running","completeRows":[1]}\n'
        target.write_bytes(old)
        replacement = dict(status="error", completeRows=[1, 2], message="Unicode: \u6d4b\u8bd5")
        replace = os.replace
        observed = []
        def observe(source, destination, *args, **kwargs):
            self.assertEqual(Path(destination), target)
            self.assertEqual(Path(source).parent, target.parent)
            self.assertNotEqual(Path(source), target)
            self.assertEqual(target.read_bytes(), old)
            self.assertEqual(json.loads(Path(source).read_bytes()), replacement)
            observed.append(True)
            return replace(source, destination, *args, **kwargs)
        with patch.object(bench.os, "replace", side_effect=observe):
            bench.save(target, replacement)
        self.assertEqual(observed, [True])
        self.assertEqual(json.loads(target.read_bytes()), replacement)
        self.assertEqual(list(self.root.iterdir()), [target])

    def test_save_commit_failure_preserves_old_json_and_removes_temporary_file(self):
        target = self.root / "measurements.json"
        old = b'{"status":"running","completeRows":[1]}\n'
        target.write_bytes(old)
        failure = OSError("COMMIT_FAILURE")
        with patch.object(bench.os, "replace", side_effect=failure):
            with self.assertRaises(OSError) as raised:
                bench.save(target, dict(status="error", completeRows=[1, 2]))
        self.assertIs(raised.exception, failure)
        self.assertEqual(target.read_bytes(), old)
        self.assertEqual(list(self.root.iterdir()), [target])

    def test_save_invalid_value_preserves_old_json(self):
        target = self.root / "measurements.json"
        old = b'{"completeRows":[1]}\n'
        for invalid in (object(), float("nan"), float("inf")):
            with self.subTest(value=type(invalid).__name__, representation=str(invalid)):
                target.write_bytes(old)
                with self.assertRaises((TypeError, ValueError)):
                    bench.save(target, dict(value=invalid))
                self.assertEqual(target.read_bytes(), old)
                self.assertEqual(list(self.root.iterdir()), [target])

    def test_save_partial_write_failure_preserves_old_json(self):
        target = self.root / "measurements.json"
        old = b'{"completeRows":[1]}\n'
        target.write_bytes(old)
        create = tempfile.NamedTemporaryFile
        failure = OSError("PARTIAL_WRITE_FAILURE")
        class FailingWriter:
            def __init__(self, stream):
                self.stream, self.name = stream, stream.name
            def __enter__(self):
                self.stream.__enter__()
                return self
            def __exit__(self, *args):
                return self.stream.__exit__(*args)
            def write(self, text):
                self.stream.write(text[:7])
                self.stream.flush()
                raise failure
        with patch.object(bench.tempfile, "NamedTemporaryFile", side_effect=lambda *a, **kw: FailingWriter(create(*a, **kw))):
            with self.assertRaises(OSError) as raised:
                bench.save(target, dict(status="error", completeRows=[1, 2]))
        self.assertIs(raised.exception, failure)
        self.assertEqual(target.read_bytes(), old)
        self.assertEqual(list(self.root.iterdir()), [target])

    def test_support_round_trip_and_idempotent_close(self):
        support = self.helper("import json,sys\nfor line in sys.stdin:\n print(json.dumps({'echo':json.loads(line)}),flush=True)\n")
        result, error = self.bounded(support, lambda: support.call(op="echo", value="hello"))
        self.assertIsNone(error)
        self.assertEqual(result, dict(echo=dict(op="echo", value="hello")))
        self.assertIsNone(self.close_helper(support)[1])
        self.assertIsNone(self.close_helper(support)[1])

    def test_support_eof_and_partial_reply_are_bounded_failures(self):
        programs = {
            "eof": "import sys\nsys.stdin.readline()\n",
            "nonzero": "import sys\nsys.stdin.readline()\nraise SystemExit(7)\n",
            "partial": "import sys,time\nsys.stdin.readline()\nsys.stdout.write('{');sys.stdout.flush()\ntime.sleep(30)\n",
        }
        for name, source in programs.items():
            with self.subTest(fault=name):
                support = self.helper(source)
                _, error = self.bounded(support, lambda: support.call(op="probe"))
                self.assertIsInstance(error, Exception)
                self.close_helper(support)

    def test_support_nonreading_input_is_bounded(self):
        support = self.helper("import time\ntime.sleep(30)\n")
        _, error = self.bounded(support, lambda: support.call(op="probe", payload="x" * (4 * 1024 * 1024)))
        self.assertIsInstance(error, Exception)
        self.close_helper(support)

    def test_support_lock_wait_is_bounded(self):
        support = self.helper("import json,sys\nfor line in sys.stdin:\n print('{}',flush=True)\n")
        support.lock.acquire()
        try:
            _, error = self.bounded(support, lambda: support.call(op="probe"))
            self.assertIsInstance(error, Exception)
        finally:
            support.lock.release()
        self.close_helper(support)

    def test_support_forced_close_is_failure_and_reaps_term_ignoring_helper(self):
        support = self.helper("import signal,sys,time\nsignal.signal(signal.SIGTERM,signal.SIG_IGN)\nsys.stdin.readline()\nprint('{}',flush=True)\nsys.stdin.read()\ntime.sleep(30)\n")
        self.assertIsNone(self.bounded(support, lambda: support.call(op="ready"))[1])
        _, error = self.close_helper(support)
        self.assertIsInstance(error, Exception, "forced termination must not report clean shutdown")
        self.assertIsNone(self.close_helper(support)[1])

    def test_support_nonzero_close_cannot_pass_under_optimized_python(self):
        support = self.helper("import sys\nsys.stdin.readline()\nprint('{}',flush=True)\nsys.stdin.read()\nraise SystemExit(7)\n")
        self.assertIsNone(self.bounded(support, lambda: support.call(op="ready"))[1])
        _, error = self.close_helper(support)
        self.assertIsInstance(error, Exception)

    def test_memory_background_failure_stops_sampler_thread(self):
        marker = self.root / "second-memory-request"
        support = self.helper("import json,os,sys,time\nfrom pathlib import Path\nsys.stdin.readline()\nprint(json.dumps({'kind':'pss','valueKiB':123,'pids':[os.getpid()]}),flush=True)\nsys.stdin.readline()\nPath(" + repr(str(marker)) + ").touch()\ntime.sleep(30)\n")
        sampler = bench.Memory(support, [support.process.pid], .01)
        deadline = time.monotonic() + 2
        while not marker.exists() and time.monotonic() < deadline:
            time.sleep(.005)
        self.assertTrue(marker.exists(), "background sample never reached helper")
        _, error = self.bounded(support, sampler.close)
        self.assertIsInstance(error, Exception)
        self.assertFalse(sampler.thread.is_alive(), "memory sampler thread remained alive")
        self.close_helper(support)

    def test_support_pipe_setup_failure_reaps_started_process(self):
        path = self.root / "setup-helper.py"
        path.write_text("#!" + sys.executable + "\nimport time\ntime.sleep(30)\n")
        path.chmod(0o700)
        popen, processes = bench.subprocess.Popen, []
        failure = OSError("PIPE_SETUP_FAILURE")
        def started(*args, **kwargs):
            process = popen(*args, **kwargs)
            processes.append(process)
            self.addCleanup(self.rescue_helper, SimpleNamespace(process=process))
            return process
        with patch.object(bench.subprocess, "Popen", side_effect=started):
            with patch.object(bench.os, "set_blocking", side_effect=failure):
                with self.assertRaises(Exception) as raised:
                    bench.Support(path, timeout=.2)
        self.assertIn("PIPE_SETUP_FAILURE", diagnostic(raised.exception))
        self.assertEqual(len(processes), 1)
        self.assert_reaped(SimpleNamespace(process=processes[0]))

    def test_support_wait_and_terminate_errors_still_close_and_reap(self):
        support = self.helper("import sys,time\nsys.stdin.readline()\nprint('{}',flush=True)\nsys.stdin.read()\ntime.sleep(30)\n")
        self.assertIsNone(self.bounded(support, lambda: support.call(op="ready"))[1])
        original_wait = support.process.wait
        calls = []
        def fail_first_wait(*args, **kwargs):
            calls.append(True)
            if len(calls) == 1:
                raise OSError("FIRST_WAIT_FAILURE")
            return original_wait(*args, **kwargs)
        with patch.object(support.process, "wait", side_effect=fail_first_wait):
            with patch.object(support.process, "terminate", side_effect=OSError("TERMINATE_FAILURE")):
                _, error = self.close_helper(support)
        self.assertIsInstance(error, Exception)
        self.assertIn("FIRST_WAIT_FAILURE", diagnostic(error))

    def test_stop_app_observer_failure_still_closes_application(self):
        primary = OSError("OBSERVER_FAILURE")
        app = SimpleNamespace(roots=[], close=Mock(side_effect=OSError("APP_CLOSE_FAILURE")))
        support = SimpleNamespace(call=Mock(side_effect=primary), cleanup=[])
        with self.assertRaises(Exception) as raised:
            bench.stop_app(app, support)
        app.close.assert_called_once_with()
        self.assertTrue(any(error is primary for error in exception_chain(raised.exception)))
        self.assertIn("APP_CLOSE_FAILURE", diagnostic(raised.exception))
        self.assertEqual(support.cleanup, [], "failed cleanup must not produce a successful record")

    def test_cold_and_recovery_stop_failure_still_close_fixture(self):
        for operation in ("cold", "recovery"):
            with self.subTest(operation=operation):
                primary = ValueError("REQUEST_FAILURE")
                provider = SimpleNamespace(close=Mock(side_effect=OSError("FIXTURE_CLOSE_FAILURE")),
                                           release=SimpleNamespace(clear=Mock(), set=Mock()))
                app = SimpleNamespace(origin="unused")
                stop = Mock(side_effect=OSError("APP_STOP_FAILURE"))
                with ExitStack() as stack:
                    stack.enter_context(patch.object(bench, "fixture", return_value=provider))
                    stack.enter_context(patch.object(bench, "Micro", return_value=app))
                    stack.enter_context(patch.object(bench, "settings", return_value={}))
                    stack.enter_context(patch.object(bench.deploy, "initialize"))
                    stack.enter_context(patch.object(bench.deploy, "Deployment", return_value=app))
                    stack.enter_context(patch.object(bench.deploy, "http", side_effect=primary))
                    stack.enter_context(patch.object(bench, "request", side_effect=primary))
                    stack.enter_context(patch.object(bench, "stop_app", stop))
                    with self.assertRaises(Exception) as raised:
                        if operation == "cold":
                            bench.cold(self.root, dict(coldAdapterDelayMs=0, statusPollMs=5), self.root / "cold",
                                       "direct", dict(subjectChars=64), 0, object())
                        else:
                            bench.recovery(self.root, self.root / "recovery", 0, object())
                stop.assert_called_once()
                provider.close.assert_called_once_with()
                for message in ("REQUEST_FAILURE", "APP_STOP_FAILURE", "FIXTURE_CLOSE_FAILURE"):
                    self.assertIn(message, diagnostic(raised.exception))

    def test_cold_directory_creation_failure_still_closes_fixture(self):
        provider = SimpleNamespace(close=Mock())
        state = self.root / "existing-cold-root"; state.mkdir()
        with patch.object(bench, "fixture", return_value=provider):
            with self.assertRaises(FileExistsError):
                bench.cold(self.root, dict(coldAdapterDelayMs=0), state, "direct", dict(subjectChars=64), 0, object())
        provider.close.assert_called_once_with()

    def test_warm_request_and_sampler_failure_still_attempt_all_teardowns(self):
        primary = ValueError("REQUEST_FAILURE")
        provider = SimpleNamespace(close=Mock(side_effect=OSError("FIXTURE_CLOSE_FAILURE")))
        sampler = SimpleNamespace(close=Mock(side_effect=OSError("SAMPLER_CLOSE_FAILURE")))
        app = SimpleNamespace(roots=[])
        stop = Mock(side_effect=OSError("APP_STOP_FAILURE"))
        protocol = dict(warmupRequests=0, requestsPerRound=1, statusPollMs=5, memoryIntervalMs=50)
        with ExitStack() as stack:
            stack.enter_context(patch.object(bench, "fixture", return_value=provider))
            stack.enter_context(patch.object(bench, "Micro", return_value=app))
            stack.enter_context(patch.object(bench, "Memory", return_value=sampler))
            stack.enter_context(patch.object(bench, "request", side_effect=primary))
            stack.enter_context(patch.object(bench, "stop_app", stop))
            with self.assertRaises(Exception) as raised:
                bench.warm_round(self.root, object(), protocol, self.root / "warm", "direct",
                                 dict(subjectChars=64, name="small"), 1, 0, 0)
        sampler.close.assert_called_once_with()
        stop.assert_called_once()
        provider.close.assert_called_once_with()
        self.assertTrue(any(error is primary for error in exception_chain(raised.exception)))
        for message in ("REQUEST_FAILURE", "SAMPLER_CLOSE_FAILURE", "APP_STOP_FAILURE", "FIXTURE_CLOSE_FAILURE"):
            self.assertIn(message, diagnostic(raised.exception))

    def main_run(self, primary=None, close_error=None, final_save_error=None, metadata_error=None):
        repo = self.root / "repo"
        here, example = repo / "benchmarks/agent-triage", repo / "examples/agent-triage"
        here.mkdir(parents=True); example.mkdir(parents=True)
        for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml"):
            (repo / name).write_text("offline fixture\n")
        binary_dir = repo / "bin"; binary_dir.mkdir()
        for name in ("tysel", "tysel-service", "tysel-worker", "tysel-bench-agent-support"):
            (binary_dir / name).write_bytes(b"not an executable; main dependencies are mocked")
        protocol = json.loads((HERE / "protocol.json").read_bytes())
        protocol.update(variants=["direct"], sizes=[protocol["sizes"][0]], concurrency=[1],
                        requestsPerRound=1, warmupRequests=0, coldSamples=2, rounds=0, recoverySamples=0)
        protocol_path = here / "protocol.json"; protocol_path.write_text(json.dumps(protocol))
        output = repo / "output"
        class FakeSupport:
            def __init__(self):
                self.cleanup = []
                self.closed = 0
            def call(self, **request):
                if metadata_error:
                    raise metadata_error
                return dict(os="linux", arch="aarch64", os_version="offline", cpu_model="offline")
            def close(self, *args, **kwargs):
                self.closed += 1
                if close_error:
                    raise close_error
        support = FakeSupport()
        samples = [dict(observation="retained complete cold sample", sample=0),
                   primary if primary else dict(observation="second complete cold sample", sample=1)]
        save, final_saves = bench.save, []
        def saving(path, data):
            if data["status"] != "running":
                final_saves.append(json.loads(json.dumps(data)))
                if final_save_error:
                    raise final_save_error
            return save(path, data)
        def git_output(*args, **kwargs):
            return "a" * 40 + "\n" if kwargs.get("text") else b""
        error = result = None
        with ExitStack() as stack:
            # These synthetic main-loop tests cover persistence and teardown;
            # the real optimized interpreter entry point is tested separately.
            stack.enter_context(patch.object(bench, "require_assertions", create=True))
            for name, value in (("REPO", repo), ("HERE", here), ("EXAMPLE", example)):
                stack.enter_context(patch.object(bench, name, value))
            stack.enter_context(patch.object(sys, "argv", [str(DRIVER), "--bin-dir", str(binary_dir),
                                                           "--output", str(output), "--protocol", str(protocol_path)]))
            stack.enter_context(patch.object(bench.platform, "system", return_value="Linux"))
            stack.enter_context(patch.object(bench.subprocess, "check_output", side_effect=git_output))
            stack.enter_context(patch.object(bench, "Support", return_value=support))
            stack.enter_context(patch.object(bench, "prepare", return_value={"offline": True}))
            stack.enter_context(patch.object(bench, "cold", side_effect=samples))
            stack.enter_context(patch.object(bench, "warm_round", side_effect=AssertionError("unexpected warm sample")))
            stack.enter_context(patch.object(bench, "denials", return_value={"offline": True}))
            stack.enter_context(patch.object(bench, "recovery", side_effect=AssertionError("unexpected recovery sample")))
            stack.enter_context(patch.object(bench, "analyze", return_value=dict(passed=True, checks=[], distributions={})))
            stack.enter_context(patch.object(bench, "save", side_effect=saving))
            stack.enter_context(redirect_stdout(io.StringIO()))
            try:
                result = bench.main()
            except BaseException as caught:
                error = caught
        path = output / "measurements.json"
        retained = json.loads(path.read_bytes()) if path.exists() else None
        return dict(error=error, result=result, support=support, retained=retained, final_saves=final_saves)

    def test_main_preserves_primary_error_when_support_close_also_fails(self):
        primary, cleanup = ValueError("PRIMARY_SAMPLE_FAILURE"), OSError("CLEANUP_SUPPORT_FAILURE")
        observed = self.main_run(primary=primary, close_error=cleanup)
        self.assertIsNotNone(observed["error"])
        self.assertTrue(any(error is primary for error in exception_chain(observed["error"])))
        self.assertEqual(observed["support"].closed, 1)
        retained = observed["retained"]
        self.assertEqual(retained["status"], "error")
        self.assertIn("PRIMARY_SAMPLE_FAILURE", retained["error"])
        self.assertIn("CLEANUP_SUPPORT_FAILURE", "\n".join(retained["cleanupErrors"]))
        self.assertEqual(retained["cold"], [dict(observation="retained complete cold sample", sample=0)])

    def test_main_final_save_failure_retains_all_three_failure_causes(self):
        primary, cleanup, save_error = ValueError("PRIMARY_SAMPLE_FAILURE"), OSError("CLEANUP_SUPPORT_FAILURE"), OSError("FINAL_SAVE_FAILURE")
        observed = self.main_run(primary=primary, close_error=cleanup, final_save_error=save_error)
        self.assertIsNotNone(observed["error"])
        text = diagnostic(observed["error"])
        for message in ("PRIMARY_SAMPLE_FAILURE", "CLEANUP_SUPPORT_FAILURE", "FINAL_SAVE_FAILURE"):
            self.assertIn(message, text)
        self.assertEqual(observed["support"].closed, 1)
        self.assertEqual(len(observed["final_saves"]), 1)
        self.assertEqual(observed["retained"]["status"], "running")
        self.assertEqual(observed["retained"]["cold"], [dict(observation="retained complete cold sample", sample=0)])

    def test_main_close_failure_cannot_leave_a_passed_report(self):
        observed = self.main_run(close_error=OSError("CLEANUP_SUPPORT_FAILURE"))
        self.assertTrue(observed["error"] is not None or observed["result"] not in (None, 0))
        self.assertEqual(observed["support"].closed, 1)
        self.assertEqual(observed["retained"]["status"], "error")
        self.assertIn("CLEANUP_SUPPORT_FAILURE", "\n".join(observed["retained"]["cleanupErrors"]))

    def test_main_metadata_failure_still_closes_support_and_saves_diagnostics(self):
        primary = ValueError("PRIMARY_METADATA_FAILURE")
        observed = self.main_run(metadata_error=primary, close_error=OSError("CLEANUP_SUPPORT_FAILURE"))
        self.assertIsNotNone(observed["error"])
        self.assertTrue(any(error is primary for error in exception_chain(observed["error"])))
        self.assertEqual(observed["support"].closed, 1)
        self.assertEqual(observed["retained"]["status"], "error")
        self.assertIn("PRIMARY_METADATA_FAILURE", observed["retained"]["error"])
        self.assertIn("CLEANUP_SUPPORT_FAILURE", "\n".join(observed["retained"]["cleanupErrors"]))

    def test_main_success_remains_success(self):
        observed = self.main_run()
        self.assertIsNone(observed["error"], diagnostic(observed["error"]) if observed["error"] else "")
        self.assertEqual(observed["result"], 0)
        self.assertEqual(observed["support"].closed, 1)
        self.assertEqual(observed["retained"]["status"], "passed")
        self.assertFalse(observed["retained"].get("cleanupErrors"))
        self.assertEqual(len(observed["retained"]["cold"]), 2)

    def test_main_existing_output_is_untouched_and_never_starts_support(self):
        output = self.root / "retained-evidence"; output.mkdir()
        path = output / "measurements.json"
        original = b'{"status":"error","retained":"original evidence"}\n'
        path.write_bytes(original)
        with patch.object(bench, "require_assertions", create=True), patch.object(sys, "argv", [str(DRIVER), "--bin-dir", str(self.root / "unused-bin"),
                                         "--output", str(output), "--protocol", str(HERE / "protocol.json")]):
            with patch.object(bench.platform, "system", return_value="Linux"):
                with patch.object(bench, "Support") as support:
                    with self.assertRaises(FileExistsError):
                        bench.main()
        support.assert_not_called()
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(list(output.iterdir()), [path])

    def test_main_rejects_optimized_interpreters_before_side_effects(self):
        # Launch actual optimized interpreters. Keep the retained driver's
        # original __file__ so before/after runs use the same repository imports.
        script = """
import importlib.util, pathlib, sys
from unittest.mock import patch
driver, source, marker = map(pathlib.Path, sys.argv[1:4])
spec = importlib.util.spec_from_file_location('optimized_driver', driver)
bench = importlib.util.module_from_spec(spec)
exec(compile(source.read_bytes(), str(source), 'exec'), bench.__dict__)
def forbidden(*args, **kwargs):
    marker.touch()
    raise RuntimeError('benchmark side effect attempted')
sys.argv = [str(driver)] + sys.argv[4:]
with patch.object(bench.platform, 'system', return_value='Linux'), \\
     patch.object(bench, 'Support', side_effect=forbidden), \\
     patch.object(bench, 'prepare', side_effect=forbidden):
    bench.main()
"""
        for flags, optimization in ((["-O"], None), (["-OO"], None), ([], "1"), ([], "2")):
            with self.subTest(flags=flags, PYTHONOPTIMIZE=optimization):
                output, marker = self.root / "optimized-output", self.root / "side-effect"
                env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
                env.pop("PYTHONOPTIMIZE", None)
                if optimization is not None:
                    env["PYTHONOPTIMIZE"] = optimization
                result = subprocess.run([sys.executable, *flags, "-c", script, str(DRIVER), str(SOURCE),
                                         str(marker), "--bin-dir", str(self.root / "unused-bin"),
                                         "--output", str(output)], env=env, text=True,
                                        capture_output=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("requires assertions", result.stderr)
                self.assertFalse(output.exists(), "optimized driver created an evidence directory")
                self.assertFalse(marker.exists(), "optimized driver attempted build/process startup")

    def request_fixture(self):
        provider = SimpleNamespace(customers=[dict(id="bench-c0", token="token",
                                                   tickets=[dict(id="t0-00", priority=3)])],
                                   subject="s" * 64)
        return SimpleNamespace(origin="unused", errors=[]), provider, dict(tickets=1, subjectChars=64)

    def test_request_rejects_incorrect_micro_response_in_both_compilation_modes(self):
        for optimize in (0, 1):
            spec = importlib.util.spec_from_file_location("response_driver", DRIVER)
            driver = importlib.util.module_from_spec(spec)
            exec(compile(SOURCE.read_bytes(), str(SOURCE), "exec", optimize=optimize), driver.__dict__)
            for variant in ("direct", "snapshot"):
                for failure in ("status", "customer", "summary"):
                    with self.subTest(optimize=optimize, variant=variant, failure=failure):
                        app, provider, size = self.request_fixture()
                        def response(origin, route, method, body):
                            payload = dict(kind="done", customerId="bench-c0", summary="Prioritize t0-00: " + provider.subject)
                            if failure == "customer":
                                payload["customerId"] = "wrong-c0"  # Equal byte length cannot bypass correctness.
                            elif failure == "summary":
                                payload["summary"] = "Incorrect t0-00: " + provider.subject
                            return (500 if failure == "status" else 200), {**body, "payload": payload}
                        with patch.object(driver.deploy, "http", side_effect=response):
                            with self.assertRaises(AssertionError):
                                driver.request(app, provider, variant, size, 0, "key", .005)

    def test_related_live_entry_points_reject_optimized_interpreters_before_side_effects(self):
        # Refuse dangerous work with a sentinel as well as checking that neither
        # evidence nor state roots were created. No application binaries are used.
        script = """
import contextlib, importlib.util, pathlib, sys
from unittest.mock import patch
driver, marker = map(pathlib.Path, sys.argv[1:3])
spec = importlib.util.spec_from_file_location('related_optimized_driver', driver)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
def forbidden(*args, **kwargs):
    marker.touch()
    raise RuntimeError('live benchmark side effect attempted')
sys.argv = [str(driver)] + sys.argv[3:]
with contextlib.ExitStack() as stack:
    stack.enter_context(patch.object(pathlib.Path, 'mkdir', side_effect=forbidden))
    if hasattr(module, 'bench'):
        stack.enter_context(patch.object(module.bench, 'Support', side_effect=forbidden))
        stack.enter_context(patch.object(module.bench, 'warm_round', side_effect=forbidden))
        stack.enter_context(patch.object(module.bench.package, 'package', side_effect=forbidden))
    else:
        stack.enter_context(patch.object(module.platform, 'system', return_value='Linux'))
        stack.enter_context(patch.object(module, 'prepare', side_effect=forbidden))
        stack.enter_context(patch.object(module, 'resume', side_effect=forbidden))
        stack.enter_context(patch.object(module, 'save_new', side_effect=forbidden))
    module.main()
"""
        common = ["--output", str(self.root / "related-output")]
        paths = dict(bin_dir=self.root / "unused-bin", release=self.root / "unused-release",
                     source=self.root / "unused-source", state=self.root / "unused-state")
        arguments = {
            "storage-ab.py": ["--bin-dir", str(paths["bin_dir"]), "--baseline-release", str(paths["release"]),
                              "--candidate-release", str(paths["release"]), "--bind-root", str(paths["state"]),
                              "--volume-root", str(self.root / "unused-volume"), "--plan", str(self.root / "unused-plan")],
            "profile-caller.py": ["--bin-dir", str(paths["bin_dir"]), "--source", str(paths["source"]),
                                  "--state-root", str(paths["state"])],
            "volume-lifecycle.py": ["--phase", "prepare", "--release", str(paths["release"]),
                                    "--state-root", str(paths["state"])],
        }
        for name, args in arguments.items():
            for flags, optimization in ((["-O"], None), (["-OO"], None), ([], "1"), ([], "2")):
                with self.subTest(driver=name, flags=flags, PYTHONOPTIMIZE=optimization):
                    marker = self.root / (name + "-side-effect")
                    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1")
                    env.pop("PYTHONOPTIMIZE", None)
                    if optimization is not None:
                        env["PYTHONOPTIMIZE"] = optimization
                    result = subprocess.run([sys.executable, *flags, "-c", script, str(HERE / name), str(marker),
                                             *args, *common], env=env, text=True, capture_output=True, timeout=10)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("requires assertions", result.stderr)
                    self.assertFalse(marker.exists(), "optimized driver attempted live side effects")
                    self.assertFalse((self.root / "related-output").exists())
                    self.assertFalse(paths["state"].exists())

    def test_request_rejects_incorrect_lookup_response_in_both_compilation_modes(self):
        for optimize in (0, 1):
            spec = importlib.util.spec_from_file_location("response_driver", DRIVER)
            driver = importlib.util.module_from_spec(spec)
            exec(compile(SOURCE.read_bytes(), str(SOURCE), "exec", optimize=optimize), driver.__dict__)
            with self.subTest(optimize=optimize):
                app, provider, size = self.request_fixture()
                wrong = dict(state="succeeded", result=dict(customerId="wrong-c0", summary="Prioritize t0-00: " + provider.subject))
                provider.lock, provider.requests, provider.delays = threading.Lock(), [], []
                def response(origin, route, *args, **kwargs):
                    if route == "/jobs":
                        return 202, dict(jobId="job-1")
                    observed = time.monotonic()
                    provider.requests.append(dict(ticketId="t0-00", at=observed, authorized=True))
                    provider.delays.append(dict(ticketId="t0-00", start=observed, actualSleepMs=0))
                    return 200, wrong
                with patch.object(driver.deploy, "http", side_effect=response):
                    with self.assertRaises(AssertionError):
                        driver.request(app, provider, "lookup", size, 0, "key", .005)

    def test_request_accepts_correct_response_in_both_compilation_modes(self):
        for optimize in (0, 1):
            spec = importlib.util.spec_from_file_location("response_driver", DRIVER)
            driver = importlib.util.module_from_spec(spec)
            exec(compile(SOURCE.read_bytes(), str(SOURCE), "exec", optimize=optimize), driver.__dict__)
            for variant in ("direct", "snapshot", "lookup"):
                with self.subTest(optimize=optimize, variant=variant):
                    app, provider, size = self.request_fixture()
                    provider.lock, provider.requests, provider.delays = threading.Lock(), [], []
                    expected = dict(customerId="bench-c0", summary="Prioritize t0-00: " + provider.subject)
                    def response(origin, route, method=None, body=None, *args, **kwargs):
                        if route == "/jobs":
                            return 202, dict(jobId="job-1")
                        if variant != "lookup":
                            return 200, {**body, "payload": dict(kind="done", **expected)}
                        observed = time.monotonic()
                        provider.requests.append(dict(ticketId="t0-00", at=observed, authorized=True))
                        provider.delays.append(dict(ticketId="t0-00", start=observed, actualSleepMs=0))
                        return 200, dict(state="succeeded", result=expected)
                    with patch.object(driver.deploy, "http", side_effect=response):
                        sample = driver.request(app, provider, variant, size, 0, "key", .005)
                    self.assertEqual(sample["client"], 0)
                    self.assertGreaterEqual(sample["e2eMs"], 0)
                    self.assertEqual(len(sample["adapterAttempts"]), 1 if variant == "lookup" else 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
