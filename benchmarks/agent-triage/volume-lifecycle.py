#!/usr/bin/env python3
"""Two-container, same-volume lifecycle probe using the original packaged supervisor.

Run prepare then resume immediately in separate Linux containers with the same
named volume at --state-root and exactly the same release at --release. Each
--output is a new JSON file, normally on a separate evidence bind mount. State is
retained on success and failure. This is a process/container restart check, not
a host power-loss or physical-storage durability test.

For resume, --backup-root may name a new directory on a different mount. The
packaged backup helper copies there without overrides, and its restore helper
copies back into a new namespace under --state-root. The default backup root is
--state-root itself.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import sqlite3
import sys
import time
import traceback

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
HEALTHY_KEY = "p52-volume-healthy"
CRASH_KEY = "p52-volume-crash"
HEALTHY_TICKET = "a-01"
CRASH_TICKET = "a-02"
TOKEN = "demo-a"
APP_TABLES = ("triage_jobs", "triage_attempts", "triage_audit")
RUNTIME_TABLES = ("durable_events", "durable_executions", "durable_completions")


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_hash(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def no_symlinks(path, recursive=False):
    absolute = Path(os.path.abspath(path))
    for item in (absolute, *absolute.parents):
        if item.is_symlink():
            raise RuntimeError("symlink paths are not permitted: " + str(item))
    if recursive and absolute.exists():
        for item in absolute.rglob("*"):
            if item.is_symlink():
                raise RuntimeError("symlinks are not permitted in release/state: " + str(item))
    return absolute


def save_new(path, value):
    no_symlinks(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x") as stream:
        json.dump(value, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())


def read_json(path):
    return json.loads(path.read_text())


def sql_value(value):
    return {"blobHex": value.hex()} if isinstance(value, bytes) else value


def query(state, sql, runtime=False, params=()):
    path = state / "caller/data" / ("durable-events.db" if runtime else "jobs.db")
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5) as db:
        db.row_factory = sqlite3.Row
        return [{key: sql_value(row[key]) for key in row.keys()} for row in db.execute(sql, params)]


def snapshot(state):
    # Sort every column to make cross-copy comparisons independent of scan order.
    values = {table: query(state, "SELECT * FROM " + table) for table in APP_TABLES}
    values.update({table: query(state, "SELECT * FROM " + table, True) for table in RUNTIME_TABLES})
    for rows in values.values():
        rows.sort(key=lambda row: json.dumps(row, sort_keys=True))
    return values


def databases(state):
    result = {}
    for name in ("jobs.db", "durable-events.db"):
        path = state / "caller/data" / name
        with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5) as db:
            item = dict(sha256=sha(path), bytes=path.stat().st_size,
                        journalMode=db.execute("PRAGMA journal_mode").fetchone()[0],
                        synchronous=db.execute("PRAGMA synchronous").fetchone()[0],
                        integrity=[row[0] for row in db.execute("PRAGMA integrity_check")])
        assert item["integrity"] == ["ok"], item
        result[name] = item
    result["observationScope"] = (
        "Read-only Python SQLite connections after stopping all writers. journal_mode is "
        "persisted database metadata; synchronous is this observer connection's setting, "
        "not proof of the native writer's connection setting. No PRAGMA was changed.")
    return result


def mount_context(root):
    raw = Path("/proc/self/mountinfo").read_text()
    matching = []
    for line in raw.splitlines():
        fields = line.split()
        mount_point = fields[4].replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\")
        if root == Path(mount_point) or Path(mount_point) in root.parents:
            matching.append((len(mount_point), line))
    return dict(realPath=str(root.resolve()), device=root.stat().st_dev,
                hostname=platform.node(), platform=platform.platform(),
                bootId=Path("/proc/sys/kernel/random/boot_id").read_text().strip(),
                mountInfo=raw, containingMount=max(matching)[1] if matching else None)


def wait_for(predicate, timeout=70, poll=.025):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = predicate()
        if value:
            return value
        time.sleep(poll)
    raise AssertionError("bounded wait expired")


def process_table():
    result = {}
    for item in Path("/proc").iterdir():
        if not item.name.isdigit():
            continue
        try:
            fields = (item / "stat").read_text().rsplit(") ", 1)[1].split()
            result[int(item.name)] = dict(state=fields[0], parent=int(fields[1]), start=fields[19])
        except (FileNotFoundError, ProcessLookupError):
            pass
    return result


def descendants(roots):
    table = process_table()
    selected = set(roots)
    while True:
        more = {pid for pid, item in table.items() if item["parent"] in selected}
        if more <= selected:
            return {pid: table[pid] for pid in selected if pid in table}
        selected |= more


def close_app(app, evidence, crash=False):
    observed = descendants([app.caller.process.pid, app.plugin.process.pid])
    item = dict(crash=crash, observedProcesses=observed, liveProcessesRemaining=None)
    evidence["cleanup"].append(item)
    try:
        app.close(crash=crash)
        def stopped():
            now = process_table()
            return all(pid not in now or now[pid]["state"] == "Z" or
                       now[pid]["start"] != before["start"] for pid, before in observed.items())
        wait_for(stopped, timeout=5)
        item["liveProcessesRemaining"] = 0
        item["dispatcherStopped"] = app.driver is None or not app.driver.is_alive()
        assert item["dispatcherStopped"]
    except BaseException as error:
        item["error"] = str(error)
        raise


def fixture_contract(fixture):
    return dict(customers=fixture.customers, records=fixture.records, subject=fixture.subject,
                adapterId="p52-independent-volume-fixture-v1")


def reads(fixture, phase):
    with fixture.lock:
        return [dict(record, phase=phase, phaseOrdinal=index + 1)
                for index, record in enumerate(fixture.requests)]


def submit(deploy, app, key, ticket):
    return deploy.http(app.origin, "/jobs", "POST",
                       {"protocolVersion": 1, "ticketIds": [ticket]}, TOKEN, key)


def finished(deploy, app, job_id, timeout=70):
    def poll():
        assert not app.errors, app.errors
        status, value = deploy.http(app.origin, "/jobs/" + job_id, token=TOKEN, timeout=5)
        assert status == 200, (status, value)
        return value if value["state"] in ("succeeded", "failed", "expired") else None
    return wait_for(poll, timeout=timeout)


def drained(app):
    def poll():
        assert not app.errors, app.errors
        jobs = query(app.state, "SELECT state,delivery FROM triage_jobs")
        complete = query(app.state, "SELECT task_id FROM durable_completions", True)
        return (len(complete) == len(jobs) and all(
            job["state"] in ("succeeded", "failed", "expired") and job["delivery"] != "pending"
            for job in jobs))
    wait_for(poll, timeout=10)


def expect_failure(call, text):
    try:
        call()
    except RuntimeError as error:
        assert text in str(error), str(error)
        return str(error)
    raise AssertionError("expected failure: " + text)


def assert_canaries(value, fixture, base):
    encoded = json.dumps(value)
    assert base.FAKE_SECRET not in encoded and base.PRIVATE_FIELD not in encoded
    assert all(item["authorized"] for item in fixture.requests)


def prepare(deploy, base, release, root, evidence):
    assert not any(root.iterdir()), "prepare requires an empty state root"
    state = root / "namespace"
    fixture = base.Fixture()
    app = None
    try:
        contract = fixture_contract(fixture)
        settings = dict(adapterOrigin=fixture.origin, adapterId=contract["adapterId"], customers=fixture.customers)
        deploy.initialize(release, state, settings)
        original_config = read_json(state / "caller/config/service.json")
        assert original_config["limits"] == deploy.LIMITS
        binding = read_json(state / "binding.json")
        app = deploy.Deployment(release, state, base.FAKE_SECRET)
        evidence["lockDenied"] = expect_failure(lambda: deploy.Deployment(release, state, base.FAKE_SECRET), "namespace is in use")
        status, healthy = submit(deploy, app, HEALTHY_KEY, HEALTHY_TICKET)
        assert status == 202, (status, healthy)
        healthy = finished(deploy, app, healthy["jobId"])
        assert healthy["state"] == "succeeded", healthy
        assert healthy["result"] == dict(customerId="customer-a", summary="Prioritize " + HEALTHY_TICKET + ": " + fixture.subject)
        drained(app)
        assert fixture.count() == 1
        healthy_snapshot = snapshot(state)
        before_replay = fixture.count()
        assert submit(deploy, app, HEALTHY_KEY, HEALTHY_TICKET) == (202, healthy)
        assert fixture.count() == before_replay
        fixture.arrived.clear()
        fixture.release.clear()
        status, crashing = submit(deploy, app, CRASH_KEY, CRASH_TICKET)
        assert status == 202 and fixture.arrived.wait(5), (status, crashing)
        assert crashing["deadlineAt"] - crashing["createdAt"] == 120000
        active = query(state, "SELECT * FROM durable_executions WHERE state='running'", True)
        slots = query(state, "SELECT * FROM triage_attempts WHERE job_id=? AND step='read'", params=(crashing["jobId"],))
        assert len(active) == 1 and len(slots) == 1 and slots[0]["outcome_json"] is None
        assert fixture.count() == 2
        crash_wall_ms = time.time() * 1000
        remaining = active[0]["lease_until_ms"] - crash_wall_ms
        assert 0 < remaining <= 45000, remaining
        close_app(app, evidence, crash=True)
        app = None
        fixture.release.set()
        after_crash = snapshot(state)
        assert_canaries(after_crash, fixture, base)
        observed_reads = reads(fixture, "prepare")
        save_new(root / "fixture-prepare.json", observed_reads)
        checkpoint = dict(schemaVersion=1, phase="prepared", releaseMetadata=evidence["release"],
                          binding=binding, fixtureContract=contract, fixtureContractHash=canonical_hash(contract),
                          originalConfig=original_config, preparedConfig=read_json(state / "caller/config/service.json"),
                          healthy=healthy, crashing=crashing, healthySnapshot=healthy_snapshot,
                          afterCrash=after_crash, firstReadSlot=slots[0], execution=active[0],
                          crashWallMs=crash_wall_ms, leaseRemainingAtCrashMs=remaining,
                          defaultLeaseMs=45000, recoveryBudgetMs=50000,
                          readsFileSha256=sha(root / "fixture-prepare.json"),
                          context=evidence["context"])
        save_new(root / "checkpoint.json", checkpoint)
        evidence.update(checkpointSha256=sha(root / "checkpoint.json"), healthy=healthy,
                        crashing=crashing, crashWallMs=crash_wall_ms,
                        leaseRemainingAtCrashMs=remaining, independentReads=observed_reads,
                        afterCrash=after_crash, databases=databases(state),
                        originalArtifactBinding=binding,
                        nextStep="Run resume immediately in a different container with this same volume and release.")
    finally:
        fixture.release.set()
        try:
            if app is not None:
                close_app(app, evidence)
        finally:
            fixture.close()
            evidence["cleanup"].append(dict(fixtureStopped=not fixture.thread.is_alive(), requestHandlersJoined=True))
            evidence["independentReads"] = reads(fixture, "prepare")


def resume(deploy, base, release, root, backup_root, evidence):
    checkpoint = read_json(root / "checkpoint.json")
    evidence["checkpointSha256"] = sha(root / "checkpoint.json")
    assert checkpoint["schemaVersion"] == 1 and checkpoint["phase"] == "prepared"
    assert checkpoint["releaseMetadata"] == evidence["release"], "release bytes/metadata changed"
    assert checkpoint["context"]["hostname"] != evidence["context"]["hostname"], "use a new container hostname"
    assert checkpoint["context"]["realPath"] == evidence["context"]["realPath"]
    assert sha(root / "fixture-prepare.json") == checkpoint["readsFileSha256"]
    prior_reads = read_json(root / "fixture-prepare.json")
    state = root / "namespace"
    assert read_json(state / "binding.json") == checkpoint["binding"]
    assert snapshot(state) == checkpoint["afterCrash"], "namespace changed between containers"
    config_path = state / "caller/config/service.json"
    before_config = read_json(config_path)
    assert before_config == checkpoint["preparedConfig"]
    fixture = base.Fixture()
    app = None
    try:
        assert fixture_contract(fixture) == checkpoint["fixtureContract"], "provider records changed"
        # Only this transport address changes between containers. Deployment itself
        # rotates bootId and pluginOrigin as part of its documented restart path.
        with deploy.StateLock(state):
            replacement = dict(before_config, adapterOrigin=fixture.origin)
            deploy.write_json(config_path, replacement)
        evidence["adapterOriginUpdate"] = dict(before=before_config["adapterOrigin"], after=fixture.origin,
                                              changedFields=["adapterOrigin"], bindingChanged=False)
        evidence["resumeWallMs"] = time.time() * 1000
        evidence["containerGapAfterCrashMs"] = evidence["resumeWallMs"] - checkpoint["crashWallMs"]
        recovery_started = time.monotonic()
        app = deploy.Deployment(release, state, base.FAKE_SECRET)
        evidence["lockDenied"] = expect_failure(lambda: deploy.Deployment(release, state, base.FAKE_SECRET), "namespace is in use")
        crash_id = checkpoint["crashing"]["jobId"]
        status, retained = submit(deploy, app, CRASH_KEY, CRASH_TICKET)
        assert status in (200, 202) and retained["jobId"] == crash_id
        assert retained["deadlineAt"] == checkpoint["crashing"]["deadlineAt"]
        final = finished(deploy, app, crash_id, timeout=max(0, 70 - (time.monotonic() - recovery_started)))
        terminal_wall_ms = time.time() * 1000
        elapsed = terminal_wall_ms - checkpoint["crashWallMs"]
        evidence["recoveryTiming"] = dict(crashWallMs=checkpoint["crashWallMs"], terminalWallMs=terminal_wall_ms,
                                        elapsedWallMs=elapsed, budgetMs=50000, withinBudget=0 <= elapsed <= 50000,
                                        defaultLeaseMs=45000, resumeWaitLimitSeconds=70,
                                        includesContainerGap=True,
                                        scope="One cross-container observation; gap includes orchestration. Not the frozen P5 performance matrix.")
        assert final["state"] == "succeeded", final
        assert final["deadlineAt"] == checkpoint["crashing"]["deadlineAt"]
        assert final["result"] == dict(customerId="customer-a", summary="Prioritize " + CRASH_TICKET + ": " + fixture.subject)
        drained(app)
        slots = query(state, "SELECT * FROM triage_attempts WHERE job_id=? AND step='read' ORDER BY ordinal", params=(crash_id,))
        assert len(slots) == 2 and [slot["ordinal"] for slot in slots] == [1, 2]
        for field in ("job_id", "step", "ordinal", "owner", "attempt_id"):
            assert slots[0][field] == checkpoint["firstReadSlot"][field], field
        assert json.loads(slots[0]["outcome_json"])["error"] == "UNKNOWN_OUTCOME"
        assert json.loads(slots[1]["outcome_json"])["ok"] is True
        before_replay = fixture.count()
        assert submit(deploy, app, CRASH_KEY, CRASH_TICKET) == (202, final)
        assert submit(deploy, app, HEALTHY_KEY, HEALTHY_TICKET) == (202, checkpoint["healthy"])
        assert fixture.count() == before_replay == 1
        evidence["liveBackupDenied"] = expect_failure(lambda: deploy.backup(state, backup_root / "live-backup-forbidden"), "namespace is in use")
        close_app(app, evidence)
        app = None
        terminal = snapshot(state)
        evidence["terminalSnapshot"] = terminal
        assert_canaries(terminal, fixture, base)
        old_job = next(row for row in checkpoint["afterCrash"]["triage_jobs"] if row["id"] == crash_id)
        new_job = next(row for row in terminal["triage_jobs"] if row["id"] == crash_id)
        for field in ("id", "customer", "key_hash", "input_json", "request_json", "created_at", "deadline_at",
                      "definition_json", "execution_version", "delivery_until"):
            assert new_job[field] == old_job[field], "job identity changed: " + field
        # Durable admission precedes the synchronous start's return. A crash in
        # the external read can leave the application outbox acknowledgement
        # NULL even though its deterministic runtime task already exists.
        admission_key = "triage.v2:" + crash_id
        task_id = hashlib.sha256(("tysel:admission:v1:" + admission_key).encode()).hexdigest()[:32]
        evidence["taskIdentity"] = dict(admissionKey=admission_key, expectedTaskId=task_id,
                                        before=old_job["task_id"], after=new_job["task_id"],
                                        deliveryBefore=old_job["delivery"], deliveryAfter=new_job["delivery"])
        assert checkpoint["execution"]["admission_key"] == admission_key
        assert checkpoint["execution"]["task_id"] == {"blobHex": task_id}
        assert old_job["task_id"] in (None, task_id)
        assert old_job["task_id"] is not None or old_job["delivery"] == "pending"
        assert new_job["task_id"] == task_id and new_job["delivery"] == "completed"
        assert len(terminal["durable_executions"]) == 2
        assert sum(row["admission_key"] == admission_key for row in terminal["durable_executions"]) == 1
        recovered_execution = next(row for row in terminal["durable_executions"]
                                   if row["task_id"] == checkpoint["execution"]["task_id"])
        for field in ("task_id", "admission_key", "request_hash"):
            assert recovered_execution[field] == checkpoint["execution"][field], field
        assert recovered_execution["state"] == "completed"
        assert recovered_execution["generation"] > checkpoint["execution"]["generation"]
        assert len(terminal["triage_jobs"]) == len(terminal["durable_completions"]) == 2
        assert len([row for row in terminal["triage_audit"] if row["decision"] == "terminal"]) == 2
        # The already completed healthy job/attempts/audit/completion must survive
        # the other job's crash and recovery byte-for-byte at the row level.
        for table in APP_TABLES:
            field = "id" if table == "triage_jobs" else "job_id"
            healthy_rows = [row for row in terminal[table] if row[field] == checkpoint["healthy"]["jobId"]]
            assert healthy_rows == checkpoint["healthySnapshot"][table], table
        original_runtime_ids = {json.dumps(row["task_id"], sort_keys=True) for row in checkpoint["healthySnapshot"]["durable_completions"]}
        assert [row for row in terminal["durable_completions"] if json.dumps(row["task_id"], sort_keys=True) in original_runtime_ids] == checkpoint["healthySnapshot"]["durable_completions"]
        evidence["originalDatabases"] = databases(state)
        snapshot_dir = backup_root / "stopped-snapshot"
        restored = root / "restored-namespace"
        evidence["backupPaths"] = dict(source=str(state), snapshot=str(snapshot_dir), restored=str(restored),
                                      helper="original packaged deploy.py backup/restore without overrides")
        deploy.backup(state, snapshot_dir)
        deploy.restore(release, snapshot_dir, restored)
        assert snapshot(restored) == terminal
        assert read_json(restored / "binding.json") == checkpoint["binding"]
        evidence["backup"] = read_json(snapshot_dir / "snapshot.json")
        evidence["restoreOverwriteDenied"] = expect_failure(lambda: deploy.restore(release, snapshot_dir, restored), "new destination")
        evidence["restoredDatabasesBeforeStart"] = databases(restored)
        app = deploy.Deployment(release, restored, base.FAKE_SECRET)
        assert submit(deploy, app, CRASH_KEY, CRASH_TICKET) == (202, final)
        assert submit(deploy, app, HEALTHY_KEY, HEALTHY_TICKET) == (202, checkpoint["healthy"])
        drained(app)
        assert fixture.count() == before_replay
        close_app(app, evidence)
        app = None
        assert snapshot(restored) == terminal
        assert read_json(restored / "binding.json") == checkpoint["binding"]
        all_reads = prior_reads + reads(fixture, "resume")
        assert all(row["authorized"] for row in all_reads)
        counts = {ticket: sum(row["ticketId"] == ticket for row in all_reads) for ticket in (HEALTHY_TICKET, CRASH_TICKET)}
        assert counts == {HEALTHY_TICKET: 1, CRASH_TICKET: 2} and len(all_reads) == 3, counts
        evidence.update(final=final, healthy=checkpoint["healthy"], firstSlotBurned=True,
                        originalDeadlineAndIdentityPreserved=True, artifactBinding=checkpoint["binding"],
                        terminalSnapshot=terminal, terminalSnapshotSha256=canonical_hash(terminal),
                        restoredTablesEqual=True, restoredArtifactBindingEqual=True,
                        replayAdditionalReads=0, independentReads=all_reads, physicalReadCounts=counts,
                        restoredDatabasesAfterStop=databases(restored))
        save_new(root / "fixture-resume.json", reads(fixture, "resume"))
    finally:
        fixture.release.set()
        try:
            if app is not None:
                close_app(app, evidence)
        finally:
            fixture.close()
            evidence["cleanup"].append(dict(fixtureStopped=not fixture.thread.is_alive(), requestHandlersJoined=True))
            evidence["independentReads"] = prior_reads + reads(fixture, "resume")


def main():
    if not __debug__:
        raise RuntimeError("volume lifecycle probe requires assertions; do not use -O, -OO or PYTHONOPTIMIZE")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--phase", choices=("prepare", "resume"), required=True)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--state-root", type=Path, required=True)
    parser.add_argument("--backup-root", type=Path,
                        help="new backup directory root for resume, possibly on a separate mount; defaults to --state-root")
    parser.add_argument("--output", type=Path, required=True, help="new evidence JSON file")
    args = parser.parse_args()
    output = no_symlinks(args.output)
    if output.exists():
        raise SystemExit("output exists; evidence is never overwritten")
    evidence = dict(schemaVersion=1, phase=args.phase, status="running", cleanup=[],
                    startedWallMs=time.time() * 1000, scriptSha256=sha(Path(__file__)),
                    scope="Podman named-volume local Linux process/container restart, not host power loss",
                    stateRetained=True, overrides=[], externalRequests=0)
    try:
        assert platform.system() == "Linux", "requires Linux /proc and real release binaries"
        root = no_symlinks(args.state_root, recursive=True)
        release = no_symlinks(args.release, recursive=True)
        root.mkdir(parents=True, exist_ok=True)
        assert output != root and root not in output.parents, "write evidence outside the state volume"
        evidence["context"] = mount_context(root)
        backup_root = root if args.backup_root is None else no_symlinks(args.backup_root, recursive=True)
        evidence["backupRoot"] = str(backup_root)
        if args.phase == "resume":
            if args.backup_root is not None:
                backup_root.mkdir(parents=True, exist_ok=False)
            evidence["backupContext"] = mount_context(backup_root)
        deploy = load("p52_packaged_deployment", release / "deploy.py")
        evidence["release"] = deploy.verify(release)
        evidence["releaseMetadataSha256"] = sha(release / "release.json")
        base = load("p52_independent_fixture", REPO / "examples/agent-triage/run.py")
        evidence["fixtureSourceSha256"] = sha(REPO / "examples/agent-triage/run.py")
        if args.phase == "prepare":
            prepare(deploy, base, release, root, evidence)
        else:
            resume(deploy, base, release, root, backup_root, evidence)
        no_symlinks(root, recursive=True)
        no_symlinks(backup_root, recursive=True)
        evidence["status"] = "passed"
    except BaseException as error:
        evidence.update(status="failed", error=str(error), traceback=traceback.format_exc())
    finally:
        evidence["finishedWallMs"] = time.time() * 1000
        save_new(output, evidence)
    print(json.dumps({"phase": args.phase, "status": evidence["status"], "output": str(output)}), flush=True)
    return 0 if evidence["status"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
