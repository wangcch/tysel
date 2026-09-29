#!/usr/bin/env python3
"""Initialization must preserve failed-start retry, migration and live maintenance."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import itertools
import json
import os
from pathlib import Path
import signal
import sqlite3
import sys
import threading
import time
import traceback
import uuid

spec = importlib.util.spec_from_file_location("triage_recovery", Path(__file__).with_name("agent_triage_recovery.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
Demo, http, query = base.Demo, base.http, base.query
demo_module = base.base.demo_module


def raw_start(demo):
    """Start without the demo's eager health check, to exercise first requests."""
    demo.config["bootId"] = str(uuid.uuid4())
    (demo.caller_dir / "config/service.json").write_text(json.dumps(demo.config))
    demo.caller = demo_module.Child(demo.binary, demo.caller_dir / "tysel.toml", demo.worker, demo_module.FAKE_SECRET)


def one_worker(demo):
    manifest = demo.caller_dir / "tysel.toml"
    manifest.write_text(manifest.read_text().replace("workers = 9", "workers = 1"))


def failed_first_request_retries(binary, worker):
    with Demo(binary, worker, dispatch=False, caller_transform=one_worker) as demo:
        demo.stop_caller()
        db = sqlite3.connect(demo.caller_dir / "data/jobs.db")
        db.execute("BEGIN EXCLUSIVE")
        try:
            raw_start(demo)
            pid = demo.caller.process.pid
            status, error = http(demo.caller.origin, "/health")
            assert status == 503 and error == {"error": "STORAGE_UNAVAILABLE"}, (status, error)
        finally:
            db.rollback()
            db.close()
        assert http(demo.caller.origin, "/health") == (200, {"ready": True})
        assert demo.caller.process.pid == pid and demo.caller.process.poll() is None
        status, job = demo.submit("after-storage-unlocked", [])
        assert status == 202 and job["state"] == "accepted"
        assert len(query(demo, "SELECT * FROM triage_audit WHERE decision='admit'")) == 1
        assert demo.fixture.count() == 0
        return dict(cases=["C06", "C08"], firstRequestStatus=503, retryStatus=200,
                    sameProcess=True, sameHttpIsolate=True, physicalReads=0)


def interrupted_first_request_retries(binary, worker):
    with Demo(binary, worker, dispatch=False, caller_transform=one_worker,
              request_timeout_ms=1000) as demo:
        demo.stop_caller()
        db = sqlite3.connect(demo.caller_dir / "data/jobs.db", check_same_thread=False)
        db.execute("BEGIN EXCLUSIVE")
        released, errors = threading.Event(), []
        def unlock():
            try:
                db.rollback()
            except Exception as error:
                errors.append(str(error))
            finally:
                released.set()
        timer = None
        try:
            raw_start(demo)
            pid = demo.caller.process.pid
            # The request expires while schema I/O is pending. Unlock within
            # the isolate's subsequent one-second native quiescence window.
            timer = threading.Timer(1.5, unlock)
            timer.start()
            status, _ = http(demo.caller.origin, "/health", timeout=5)
            assert status == 504, status
        finally:
            if timer:
                timer.join(timeout=5)
                assert released.is_set() and not timer.is_alive(), "unlock did not finish"
            else:
                db.rollback()
            db.close()
        assert not errors, errors
        # A cached pending initialization promise must not poison this worker.
        assert http(demo.caller.origin, "/health", timeout=5) == (200, {"ready": True})
        assert http(demo.caller.origin, "/health", timeout=5) == (200, {"ready": True})
        assert demo.caller.process.pid == pid and demo.caller.process.poll() is None
        assert demo.fixture.count() == 0
        return dict(cases=["C06", "C08"], firstRequestStatus=504, retryStatus=200,
                    sameProcess=True, sameHttpIsolate=True, physicalReads=0)


def concurrent_legacy_migration(binary, worker):
    with Demo(binary, worker, dispatch=False) as demo:
        _, kept = demo.submit("legacy-done")
        _, active = demo.submit("legacy-active")
        demo.stop_caller()
        columns = "id,customer,key_hash,input_json,request_json,state,error,result_json,created_at,deadline_at,boot_id,definition_json,plugin_calls,read_attempts,detail_json"
        old = query(demo, "SELECT " + columns + " FROM triage_jobs")
        with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
            db.execute("DROP TABLE triage_jobs")
            db.execute("""CREATE TABLE triage_jobs (
                id TEXT PRIMARY KEY, customer TEXT NOT NULL, key_hash TEXT NOT NULL,
                input_json TEXT NOT NULL, request_json TEXT NOT NULL, state TEXT NOT NULL,
                error TEXT, result_json TEXT, created_at INTEGER NOT NULL, deadline_at INTEGER NOT NULL,
                boot_id TEXT NOT NULL, definition_json TEXT NOT NULL, plugin_calls INTEGER NOT NULL DEFAULT 0,
                read_attempts INTEGER NOT NULL DEFAULT 0, detail_json TEXT, UNIQUE(customer,key_hash))""")
            for row in old:
                if row["id"] == kept["jobId"]:
                    row["state"], row["result_json"] = "succeeded", json.dumps({"summary": "original P1 result"})
                db.execute("INSERT INTO triage_jobs (" + columns + ") VALUES (" + ",".join("?" for _ in row) + ")", list(row.values()))
        raw_start(demo)
        barrier = threading.Barrier(12)
        def first_request(i):
            barrier.wait(timeout=5)
            return demo.submit("concurrent-first", []) if i % 2 else http(demo.caller.origin, "/health")
        with ThreadPoolExecutor(max_workers=12) as pool:
            replies = list(pool.map(first_request, range(12)))
        assert all(status == (202 if i % 2 else 200) for i, (status, _) in enumerate(replies)), replies
        job_ids = {value["jobId"] for i, (_, value) in enumerate(replies) if i % 2}
        assert len(job_ids) == 1
        assert demo.wait(kept["jobId"])["result"] == {"summary": "original P1 result"}
        assert demo.wait(active["jobId"])["error"] == "INTERRUPTED"
        assert len(query(demo, "SELECT * FROM triage_jobs")) == 3
        assert len(query(demo, "SELECT * FROM triage_jobs WHERE execution_version=1 AND delivery='legacy'")) == 2
        assert len(query(demo, "SELECT * FROM triage_audit WHERE job_id=? AND decision='admit'", (next(iter(job_ids)),))) == 1
        assert demo.fixture.count() == 0
        return dict(cases=["C02", "C06"], simultaneousFirstRequests=12, httpIsolates=9,
                    retainedLegacyRows=2, newJobs=1, physicalReads=0)


def maintenance_and_authority_stay_live(binary, worker):
    with Demo(binary, worker, dispatch=False) as demo:
        with ThreadPoolExecutor(max_workers=9) as pool:
            assert all(status == 200 for status, _ in pool.map(lambda _: http(demo.caller.origin, "/health"), range(36)))
        _, expired = demo.submit("expire-after-warmup")
        _, recovering = demo.submit("recover-after-warmup")
        with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
            db.execute("UPDATE triage_jobs SET deadline_at=0,delivery_until=0 WHERE id=?", (expired["jobId"],))
            db.execute("UPDATE triage_jobs SET boot_id='old-owner',owner='stale-owner' WHERE id=?", (recovering["jobId"],))
        assert demo.wait(expired["jobId"])["error"] == "DEADLINE_EXCEEDED"
        current = query(demo, "SELECT * FROM triage_jobs WHERE id=?", (recovering["jobId"],))[0]
        assert current["state"] == "recovering" and current["owner"] is None and current["boot_id"] == demo.config["bootId"]
        assert query(demo, "SELECT delivery FROM triage_jobs WHERE id=?", (expired["jobId"],))[0]["delivery"] == "unresolved"
        demo.config["customers"][0]["token"] = "replacement-development-token"
        (demo.caller_dir / "config/service.json").write_text(json.dumps(demo.config))
        assert http(demo.caller.origin, "/jobs/" + expired["jobId"], token="demo-a")[0] == 401
        status, result = http(demo.caller.origin, "/jobs/" + expired["jobId"], token="replacement-development-token")
        assert status == 200 and result["state"] == "expired"
        assert len(query(demo, "SELECT * FROM triage_audit WHERE job_id=? AND decision='terminal'", (expired["jobId"],))) == 1
        assert demo.fixture.count() == 0
        return dict(cases=["C05", "C06", "C08", "C09"], warmedRequests=36, deadlineEnforced=True,
                    deliveryHorizonEnforced=True, ownershipRecovered=True, changedAuthorityEnforced=True, physicalReads=0)


def maintenance_precedence_matrix(binary, worker):
    frozen = 1800000000000
    def freeze_maintenance_clock(demo):
        path = demo.caller_dir / "src/store.ts"
        source = path.read_text()
        needle = "const now = Date.now();"
        assert source.index(needle) < source.index("export async function find")
        path.write_text(source.replace(needle, f"const now = {frozen};", 1))
    with Demo(binary, worker, dispatch=False, caller_transform=freeze_maintenance_clock) as demo:
        states = ("accepted", "running", "recovering", "succeeded", "failed", "expired")
        active = set(states[:3])
        expected, terminal_ids = {}, set()
        with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
            combinations = itertools.product(states, (1, 2), (frozen-1, frozen, frozen+1),
                                             (demo.config["bootId"], "older-boot"), ("pending", "completed", "legacy"),
                                             (frozen-1, frozen, frozen+1))
            for i, (state, version, deadline, boot, delivery, horizon) in enumerate(combinations):
                identifier = str(uuid.UUID(int=i+1))
                row = dict(id=identifier, customer="customer-a", key_hash=f"matrix-{i}", input_json="{}", request_json="{}",
                           state=state, error="ORIGINAL_ERROR", result_json='{"summary":"retained"}', created_at=frozen-1000,
                           deadline_at=deadline, boot_id=boot, definition_json="{}", plugin_calls=0, read_attempts=0,
                           detail_json=None, execution_version=version, owner="original-owner" if i % 2 else None,
                           task_id=None, delivery=delivery, delivery_until=horizon, delivery_checked_at=0)
                db.execute("INSERT INTO triage_jobs (" + ",".join(row) + ") VALUES (" + ",".join("?" for _ in row) + ")", list(row.values()))
                wanted = dict(row)
                if state in active and deadline <= frozen:
                    wanted.update(state="expired", error="DEADLINE_EXCEEDED")
                elif state in active and boot != demo.config["bootId"]:
                    wanted.update(state="failed" if version == 1 else "recovering", error="INTERRUPTED" if version == 1 else None,
                                  owner=None, boot_id=demo.config["bootId"])
                if delivery == "pending" and horizon <= frozen:
                    wanted["delivery"] = "unresolved"
                if state in active and wanted["state"] in ("failed", "expired"):
                    terminal_ids.add(identifier)
                expected[identifier] = wanted
        for _ in range(18):
            assert http(demo.caller.origin, "/health") == (200, {"ready": True})
        actual = {row["id"]: row for row in query(demo, "SELECT * FROM triage_jobs")}
        assert actual == expected, [(key, actual[key], value) for key, value in expected.items() if actual[key] != value][:3]
        terminal = query(demo, "SELECT job_id,outcome FROM triage_audit WHERE decision='terminal'")
        assert len(terminal) == len(terminal_ids) and {row["job_id"] for row in terminal} == terminal_ids
        assert all(row["outcome"] == expected[row["job_id"]]["error"] for row in terminal)
        assert demo.fixture.count() == 0
        return dict(cases=["C05", "C06", "C08", "C10"], stateCombinations=len(expected),
                    deadlineEqualityIncluded=True, deliveryEqualityIncluded=True, terminalAuditRows=len(terminal),
                    originalTerminalResultsPreserved=True, expiryWinsBeforeOwnershipRecovery=True, physicalReads=0)


CASES = (failed_first_request_retries, interrupted_first_request_retries,
         concurrent_legacy_migration, maintenance_and_authority_stay_live,
         maintenance_precedence_matrix)


def main():
    if not __debug__: raise RuntimeError("acceptance assertions must be enabled")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path(os.environ.get("TYSEL_GATE_BIN_DIR", base.REPO / "target/debug")))
    parser.add_argument("--output", type=Path, default=Path("agent-triage-initialization-report.json"))
    args = parser.parse_args()
    report = dict(schemaVersion=1, stage="P5.1", status="running", cases=[])
    try:
        for case in CASES:
            started = time.monotonic()
            row = dict(name=case.__name__, status="failed")
            report["cases"].append(row)
            row.update(case(args.bin_dir / "tysel", args.bin_dir / "tysel-worker"))
            row.update(status="passed", cleanup="passed", elapsedSeconds=round(time.monotonic() - started, 3))
            print(json.dumps(row), flush=True)
        report["status"] = "passed"
    except BaseException:
        report["status"] = "failed"
        report["failure"] = traceback.format_exc()
        raise
    finally:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
