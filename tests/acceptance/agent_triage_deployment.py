#!/usr/bin/env python3
"""P4 exact-artifact, persistent namespace, restore and Linux sandbox contracts."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import queue
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback

REPO = Path(__file__).resolve().parents[2]
EXAMPLE = REPO / "examples/agent-triage"
sys.path.insert(0, str(EXAMPLE))
import package as packager
import deploy as packing_deploy
spec = importlib.util.spec_from_file_location("triage_demo_p4", EXAMPLE / "run.py")
base = importlib.util.module_from_spec(spec); spec.loader.exec_module(base)


def load_deployment(release):
    spec = importlib.util.spec_from_file_location("packaged_deployment", release / "deploy.py")
    module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
    return module


def query(state, sql, runtime=False):
    with sqlite3.connect(state / "caller/data" / ("durable-events.db" if runtime else "jobs.db")) as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute(sql)]


def wait_for(predicate, timeout=65):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = predicate()
        if value: return value
        time.sleep(.025)
    raise AssertionError("condition timed out")


def submit(app, key, ids=None):
    return app_module.http(app.origin, "/jobs", "POST", {"protocolVersion": 1, "ticketIds": ["a-01"] if ids is None else ids}, "demo-a", key)


def completed(app, job_id):
    def finished():
        if app.errors: raise AssertionError(app.errors)
        status, job = app_module.http(app.origin, "/jobs/" + job_id, token="demo-a")
        assert status == 200, (status, job)
        return job if job["state"] in ("succeeded", "failed", "expired") else None
    final = wait_for(finished)
    wait_for(lambda: all(job["delivery"] != "pending" for job in query(app.state, "SELECT delivery FROM triage_jobs")))
    return final


def expect_error(call, message):
    try: call()
    except RuntimeError as error:
        assert message in str(error), (message, str(error))
        return str(error)
    raise AssertionError("expected failure: " + message)


def settings(fixture):
    return dict(adapterOrigin=fixture.origin, adapterId="p4-local-counted-fixture-v1", customers=fixture.customers)


def assert_clean(release):
    assert {path.name for path in release.iterdir()} == {*packing_deploy.ARTIFACTS, "release.json"}
    assert not list(release.rglob("*.ts")) and not list(release.rglob("node_modules"))


def canaries(app, fixture):
    payload = json.dumps(query(app.state, "SELECT * FROM triage_jobs"))
    for table in ("triage_attempts", "triage_audit"):
        payload += json.dumps(query(app.state, "SELECT * FROM " + table))
    payload += json.dumps(query(app.state, "SELECT payload FROM durable_events", True))
    payload += json.dumps(query(app.state, "SELECT result_json FROM durable_completions", True))
    payload += "".join(app.caller.logs + app.plugin.logs)
    for secret in (base.FAKE_SECRET, base.PRIVATE_FIELD): assert secret not in payload
    assert all(read["authorized"] for read in fixture.requests)


def clean_cli(release, binary_dir, root):
    fixture = base.Fixture()
    state = root / "cli-state"
    app_module.initialize(release, state, settings(fixture))
    env = {key: value for key, value in os.environ.items() if not key.startswith(("TYSEL_", "OTEL_", "TRIAGE_", "OPENAI_"))}
    env.update(TRIAGE_FIXTURE_TOKEN=base.FAKE_SECRET, PYTHONDONTWRITEBYTECODE="1")
    output = queue.Queue()
    process = subprocess.Popen([sys.executable, str(release / "deploy.py"), "serve", "--state", str(state)], cwd=state,
                               env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    errors = []
    def drain(stream, ready=False):
        for line in stream:
            if ready: output.put(line)
            else: errors.append(line)
        stream.close()
    readers = [threading.Thread(target=drain, args=(process.stdout, True)), threading.Thread(target=drain, args=(process.stderr,))]
    for reader in readers: reader.start()
    try:
        try: startup = json.loads(output.get(timeout=15))
        except queue.Empty: raise AssertionError("packaged supervisor did not start: " + "".join(errors))
        status, job = app_module.http(startup["url"], "/jobs", "POST", {"protocolVersion": 1, "ticketIds": ["a-01"]}, "demo-a", "portable-cli")
        assert status == 202
        def done():
            status, final = app_module.http(startup["url"], "/jobs/" + job["jobId"], token="demo-a")
            assert status == 200
            return final if final["state"] == "succeeded" else None
        wait_for(done)
        assert fixture.count() == 1
        assert_clean(release)
    finally:
        process.terminate()
        try: process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill(); process.wait(timeout=5)
        for reader in readers:
            reader.join(timeout=5); assert not reader.is_alive()
        fixture.close()
    assert process.returncode == 0 and not errors, errors
    return dict(cases=["C11"], executableArtifacts=3, supervisor="packaged deploy.py", sourceFiles=0, physicalReads=1)


def restart_and_binding(release, binary_dir, root):
    fixture = base.Fixture()
    state = root / "recovery-state"
    app_module.initialize(release, state, settings(fixture))
    app = app_module.Deployment(release, state, base.FAKE_SECRET)
    try:
        for route in ("/probe/fetch", "/probe/filesystem"):
            status, reply = app_module.http(app.plugin.origin, route)
            assert status == 403 and reply["denied"]
        fixture.release.clear()
        status, job = submit(app, "crash-retained")
        assert status == 202 and fixture.arrived.wait(5)
        execution = query(state, "SELECT state,lease_until_ms FROM durable_executions", True)
        assert execution[0]["state"] == "running" and execution[0]["lease_until_ms"] > time.time() * 1000
        slots = query(state, "SELECT * FROM triage_attempts WHERE step='read'")
        assert len(slots) == 1 and slots[0]["outcome_json"] is None
        deadline = job["deadlineAt"]
        app.close(crash=True); app = None
        fixture.release.set()
        app = app_module.Deployment(release, state, base.FAKE_SECRET)
        assert submit(app, "crash-retained")[1]["jobId"] == job["jobId"]
        final = completed(app, job["jobId"])
        assert final["state"] == "succeeded" and final["deadlineAt"] == deadline, final
        assert fixture.count() == 2
        slots = query(state, "SELECT * FROM triage_attempts WHERE step='read'")
        assert len(slots) == 2 and json.loads(slots[0]["outcome_json"])["error"] == "UNKNOWN_OUTCOME"
        before = query(state, "SELECT * FROM triage_jobs")
        source = root / "plugin-v2-src"
        shutil.copytree(REPO / "examples/isolated-plugin/src", source)
        path = source / "triage.ts"; path.write_text(path.read_text().replace("Prioritize", "New version prioritizes"))
        newer = root / "release-v2"
        packager.package(binary_dir, newer, plugin_source=source)
        expect_error(lambda: app_module.Deployment(newer, state, base.FAKE_SECRET), "namespace is in use")
        app.close(); app = None
        expect_error(lambda: app_module.Deployment(newer, state, base.FAKE_SECRET), "bound to another release")
        assert query(state, "SELECT * FROM triage_jobs") == before
        with app_module.Deployment(release, state, base.FAKE_SECRET) as original:
            assert submit(original, "crash-retained")[1] == final
            assert fixture.count() == 2
            canaries(original, fixture)
        return dict(cases=["C11", "C12"], leaseMs=45000, physicalReads=2, retainedJobs=1, originalDeadlinePreserved=True,
                    reservedReadAttempts=2, incompatibleReleaseDenied=True, originalResultPreserved=True)
    finally:
        fixture.release.set()
        if app: app.close()
        fixture.close()


def backup_restore(release, binary_dir, root):
    fixture = base.Fixture()
    state = root / "backup-state"
    app_module.initialize(release, state, settings(fixture))
    try:
        with app_module.Deployment(release, state, base.FAKE_SECRET) as app:
            _, job = submit(app, "backup-retained")
            final = completed(app, job["jobId"])
            assert final["state"] == "succeeded"
            expect_error(lambda: app_module.backup(state, root / "live-backup"), "namespace is in use")
            saved = {table: query(state, "SELECT * FROM " + table) for table in ("triage_jobs", "triage_attempts", "triage_audit")}
            runtime = query(state, "SELECT result_json FROM durable_completions", True)
        snapshot = root / "nested-backup/snapshot"
        restored = root / "nested-restore/restored-state"
        assert not snapshot.parent.exists() and not restored.parent.exists()
        mode_paths = [state] + [path for path in sorted(state.rglob("*")) if path.name != "namespace.lock"]
        original_modes = {path: path.stat().st_mode & 0o7777 for path in mode_paths}
        try:
            # Different file/directory modes make accidental default-mode copies
            # visible. The namespace root remains private throughout this probe.
            for relative, mode in (("caller", 0o710), ("caller/config", 0o700),
                                   ("caller/data", 0o750), ("caller/data/jobs.db", 0o640),
                                   ("caller/data/durable-events.db", 0o600)):
                (state / relative).chmod(mode)
            saved_modes = {str(path.relative_to(state)): path.stat().st_mode & 0o7777 for path in mode_paths}
            saved_files = app_module.snapshot_files(state)
            for kind in ("symlink", "fifo"):
                unsupported = state / ("unsupported-" + kind)
                rejected = root / ("rejected-" + kind + "-backup")
                if kind == "symlink":
                    unsupported.symlink_to(state / "caller/config/service.json")
                else:
                    os.mkfifo(unsupported, 0o600)
                try:
                    expect_error(lambda: app_module.backup(state, rejected), "ordinary files and directories")
                    assert not rejected.exists(), "failed backup must remove its entire destination"
                finally:
                    unsupported.unlink()
                assert app_module.snapshot_files(state) == saved_files, "failed backup changed its source"
                app_module.backup(state, rejected)
                assert app_module.snapshot_files(rejected) == saved_files, "same-path backup retry changed contents"
                assert json.loads((rejected / "snapshot.json").read_text())["files"] == saved_files
            app_module.backup(state, snapshot)
            assert app_module.snapshot_files(snapshot) == saved_files
            for relative, mode in saved_modes.items():
                assert (snapshot / relative).stat().st_mode & 0o7777 == mode, "backup mode changed: " + relative
            inside_snapshot = snapshot / "nested-restore-forbidden"
            expect_error(lambda: app_module.restore(release, snapshot, inside_snapshot), "outside")
            assert not inside_snapshot.exists(), "rejected restore must not mutate its source snapshot"
            unsupported = snapshot / "unsupported-fifo"
            os.mkfifo(unsupported, 0o600)
            try:
                # FIFO entries are absent from the file hash map, so this reaches
                # the real copy failure after integrity and binding validation.
                assert app_module.snapshot_files(snapshot) == saved_files
                expect_error(lambda: app_module.restore(release, snapshot, restored), "ordinary files and directories")
                assert not restored.exists(), "failed restore must remove its entire destination"
            finally:
                unsupported.unlink()
            assert app_module.snapshot_files(snapshot) == saved_files, "failed restore changed its source"
            app_module.restore(release, snapshot, restored)
            assert app_module.snapshot_files(restored) == saved_files, "same-path restore retry changed contents"
            # Check before starting Deployment, which legitimately rewrites config.
            for relative, mode in saved_modes.items():
                assert (restored / relative).stat().st_mode & 0o7777 == mode, "restore mode changed: " + relative
            assert (restored / "caller/config/service.json").stat().st_mode & 0o7777 == 0o600
        finally:
            for path, mode in original_modes.items():
                path.chmod(mode)
        for table, expected in saved.items(): assert query(restored, "SELECT * FROM " + table) == expected
        assert query(restored, "SELECT result_json FROM durable_completions", True) == runtime
        expect_error(lambda: app_module.restore(release, snapshot, restored), "new destination")
        expect_error(lambda: app_module.initialize(release, restored, settings(fixture)), "never reset")
        expect_error(lambda: app_module.restore(root / "release-v2", snapshot, root / "wrong-restore"), "original release")
        with app_module.Deployment(release, restored, base.FAKE_SECRET) as app:
            assert submit(app, "backup-retained")[1] == final
            assert fixture.count() == 1
            canaries(app, fixture)
        (snapshot / "caller/config/service.json").write_text("{}")
        expect_error(lambda: app_module.restore(release, snapshot, root / "corrupt-restore"), "integrity check failed")
        active = root / "undrained-state"
        app_module.initialize(release, active, settings(fixture))
        with app_module.Deployment(release, active, base.FAKE_SECRET, dispatch=False) as app:
            assert submit(app, "not-delivered")[0] == 202
        expect_error(lambda: app_module.backup(active, root / "undrained-backup"), "requires terminal jobs")
        return dict(cases=["C12"], stateStores=2, restoredJobs=1, physicalReads=1, restoredBudgetAndBindings=True,
                    liveBackupDenied=True, undrainedBackupDenied=True, overwrittenState=0, corruptedBackupDenied=True,
                    backupAndRestorePosixModesPreserved=True, modePathsChecked=len(saved_modes),
                    symlinkBackupDenied=True, specialFileBackupDenied=True,
                    failedBackupTargetsRemoved=True, backupSamePathRetries=2,
                    specialFileRestoreDenied=True, failedRestoreTargetRemoved=True, restoreSamePathRetry=True,
                    missingDestinationParentsCreated=True, restoreInsideBackupDenied=True)
    finally:
        fixture.close()


def deployment_failures(release, binary_dir, root):
    altered = root / "broken-release"
    shutil.copytree(release, altered)
    (altered / "tysel-worker").unlink()
    missing = expect_error(lambda: app_module.verify(altered), "missing tysel-worker")
    env = {key: value for key, value in os.environ.items() if not key.startswith(("TYSEL_", "OTEL_"))}
    direct = subprocess.run([str(altered / "plugin")], cwd=altered, env=env, capture_output=True, text=True, timeout=12)
    assert direct.returncode and "worker binary not found" in direct.stderr
    shutil.copy(binary_dir / "tysel", altered / "tysel-worker")
    expect_error(lambda: app_module.verify(altered), "artifact mismatch: tysel-worker")
    metadata = json.loads((altered / "release.json").read_text())
    metadata["artifacts"]["tysel-worker"]["sha256"] = app_module.sha256(altered / "tysel-worker")
    (altered / "release.json").write_text(json.dumps(metadata))
    expect_error(lambda: app_module.verify(altered), "incompatible toolchain: tysel-worker")
    fixture = base.Fixture()
    try:
        expect_error(lambda: app_module.initialize(release, root / "denied-origin", dict(settings(fixture), adapterOrigin="https://unconfigured.invalid")), "loopback")
        manifest = root / "denied.toml"
        manifest.write_text((EXAMPLE / "tysel.toml").read_text().replace('fs_read = ["./config"]', 'fs_read = []'))
        denied = root / "denied-release"
        packager.package(binary_dir, denied, caller_manifest=manifest)
        state = root / "denied-state"
        app_module.initialize(denied, state, settings(fixture))
        expect_error(lambda: app_module.Deployment(denied, state, base.FAKE_SECRET), "embedded fs_read grant")
        assert fixture.count() == 0
    finally:
        fixture.close()
    return dict(cases=["C11", "C12"], missingWorker=missing, wrongWorkerDenied=True, incompatibleWorkerIdentityDenied=True,
                deniedConfigurationFailsHealth=True, forbiddenPhysicalReads=0)


def packaged_limits(release, binary_dir, root):
    source = root / "limits-src"; source.mkdir()
    (source / "index.ts").write_text('''export default {fetch(request) {
      const path = new URL(request.url).pathname;
      if (path === '/cpu') { for (;;) {} }
      if (path === '/memory') { const values = []; for (;;) values.push(new Uint8Array(1024 * 1024)); }
      return Response.json({ok:true});
    }};''')
    limits = root / "limits-release"
    packager.package(binary_dir, limits, plugin_source=source)
    cwd = root / "limits-state"; cwd.mkdir()
    process = app_module.Process(limits / "plugin", cwd, limits / "tysel-worker")
    observations = {}
    try:
        for name in ("cpu", "memory"):
            status, result = app_module.http(process.origin, "/" + name, timeout=10)
            assert status in (500, 504), (name, status, result)
            assert app_module.http(process.origin, "/")[1] == {"ok": True}
            observations[name] = dict(status=status, recovered=True)
    finally:
        process.close()
    return dict(cases=["C11"], limits=observations, artifactHashes=app_module.verify(limits)["artifacts"])


def linux_sandbox(release, binary_dir, root):
    if platform.system() != "Linux":
        return dict(cases=["C11"], linuxSandbox="not-run", reason="macOS development evidence only")
    cwd = root / "sandbox-state"; cwd.mkdir()
    process = app_module.Process(release / "plugin", cwd, release / "tysel-worker")
    try:
        pid = process.process.pid
        children = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
        assert len(children) == 1, children
        status = Path(f"/proc/{children[0]}/status").read_text()
        assert "NoNewPrivs:\t1" in status and "Seccomp:\t2" in status, status
        limits = Path(f"/proc/{children[0]}/limits").read_text()
        assert any(line.startswith("Max open files") and line.split()[3:5] == ["64", "64"] for line in limits.splitlines()), limits
    finally:
        process.close()
    failures = {}
    for name in ("landlock", "seccomp", "rlimit"):
        result = subprocess.run([sys.executable, str(Path(__file__).with_name("fixtures") / "linux_sandbox_failure.py"), name,
                                 str(release / "plugin"), str(release / "tysel-worker"), str(cwd)], capture_output=True, text=True, timeout=15)
        assert result.returncode != 0 and "tysel listen" not in result.stdout, (name, result.stdout, result.stderr)
        assert ("landlock is required" if name == "landlock" else "seccomp" if name == "seccomp" else "resource limit") in result.stderr, result.stderr
        failures[name] = dict(exitCode=result.returncode, diagnostic=result.stderr.strip())
    return dict(cases=["C11"], linuxSandbox="passed", noNewPrivileges=True, seccompFilter=True,
                openFileLimit=64, requiredSetupFailures=failures, cgroup="best-effort; not a required guarantee")


CASES = [clean_cli, restart_and_binding, backup_restore, deployment_failures, packaged_limits, linux_sandbox]


def main():
    global app_module
    if not __debug__: raise RuntimeError("acceptance assertions must be enabled")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path(os.environ.get("TYSEL_GATE_BIN_DIR", REPO / "target/debug")))
    parser.add_argument("--output", type=Path, default=Path("agent-triage-deployment-report.json"))
    parser.add_argument("--retain-release", type=Path, help="copy the exact successfully tested release to a new directory")
    args = parser.parse_args()
    report = dict(schemaVersion=1, stage="P4", status="running", platform=platform.platform(), cases=[])
    try:
        with tempfile.TemporaryDirectory(prefix="triage-deployment-") as temporary:
            root = Path(temporary)
            release = root / "release"
            report["release"] = packager.package(args.bin_dir, release)
            app_module = load_deployment(release)
            for case in CASES:
                started = time.monotonic()
                row = dict(name=case.__name__, status="failed")
                report["cases"].append(row)
                row.update(case(release, args.bin_dir.resolve(), root))
                row.update(status="passed", elapsedSeconds=round(time.monotonic() - started, 3))
                print(json.dumps(row), flush=True)
            assert_clean(release)
            if args.retain_release:
                shutil.copytree(release, args.retain_release)
                assert app_module.verify(args.retain_release) == report["release"]
                report["retainedRelease"] = str(args.retain_release.resolve())
        report.update(status="passed", cleanup="passed")
    except BaseException:
        report.update(status="failed", failure=traceback.format_exc())
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
