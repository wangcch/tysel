#!/usr/bin/env python3
"""P2 crash and ownership contracts against real local Tysel processes."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import signal
import sqlite3
import sys
import time
import traceback

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("triage_contracts", Path(__file__).with_name("agent_triage.py"))
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
Demo, http, rows = base.Demo, base.http, base.rows


def wait_for(predicate, timeout=18):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = predicate()
        if value:
            return value
        time.sleep(.025)
    raise AssertionError("condition timed out")


def query(demo, sql, args=(), *, runtime=False):
    name = "durable-events.db" if runtime else "jobs.db"
    with sqlite3.connect(demo.caller_dir / "data" / name) as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute(sql, args)]


def mutate(demo, sql, *, runtime=False):
    name = "durable-events.db" if runtime else "jobs.db"
    with sqlite3.connect(demo.caller_dir / "data" / name) as db:
        db.execute(sql)


def edit(demo, file, needle, replacement):
    path = demo.caller_dir / "src" / file
    source = path.read_text()
    assert source.count(needle) == 1, (file, needle, source.count(needle))
    path.write_text(source.replace(needle, replacement))


def gate_call(demo, name):
    demo.fixture.gate(name)
    return f"await (await fetch({json.dumps(demo.fixture.origin + '/gates/' + name)})).text();"


def barrier(mode):
    def transform(demo):
        gate = gate_call(demo, mode)
        if mode == "admission":
            edit(demo, "index.ts", "        return Response.json(publicJob(job),", gate + "\n        return Response.json(publicJob(job),")
        elif mode == "runtime_admission":
            edit(demo, "workflow.ts", "  const job = await claim(input.jobId, cfg.bootId);", gate + "\n  const job = await claim(input.jobId, cfg.bootId);")
        elif mode == "selection":
            needle = '    if (snapshot.tickets.length > 0) {'
            edit(demo, "workflow.ts", needle, gate + "\n" + needle)
        elif mode == "reservation":
            needle = "    const slot = await reserve(job, step, ordinal);"
            edit(demo, "workflow.ts", needle, needle + '\n if (step === "read") { ' + gate + ' }')
        elif mode in ("adapter_return", "plugin_return"):
            needle = "    const outcome: Outcome = { ok: true, value };"
            step = "read" if mode == "adapter_return" else "select"
            edit(demo, "workflow.ts", needle, f'if (step === "{step}") {{ {gate} }}\n' + needle)
        elif mode in ("adapter_accepted", "plugin_accepted"):
            needle = "    await accept(job, slot, outcome);\n    return outcome;"
            step = "read" if mode == "adapter_accepted" else "select"
            edit(demo, "workflow.ts", needle, f'    await accept(job, slot, outcome);\n if (step === "{step}") {{ {gate} }}\n    return outcome;')
        elif mode == "adapter_recorded":
            needle = '      await stored(ctx, "project-detail", async () => {'
            edit(demo, "workflow.ts", needle, gate + "\n" + needle)
        elif mode == "terminal":
            needle = "  return publicJob(saved);"
            edit(demo, "store.ts", needle, gate + "\n" + needle)
        else:
            raise AssertionError(mode)
    return transform


def completed(demo, job_id):
    final = demo.wait(job_id, timeout=18)
    def acknowledged():
        job = next(row for row in rows(demo) if row["id"] == job_id)
        return job if job["delivery"] == "completed" else None
    job = wait_for(acknowledged)
    results = query(demo, "SELECT result_json FROM durable_completions", runtime=True)
    assert any(json.loads(result["result_json"]) == final for result in results), (final, results)
    assert query(demo, "SELECT count(*) AS n FROM durable_programs", runtime=True)[0]["n"] == 1
    assert job["task_id"] is not None
    base.no_secrets(demo)
    assert not query(demo, "SELECT step FROM triage_attempts GROUP BY job_id,step HAVING count(*) > 2")
    return final


def assert_live_runtime(demo):
    state = query(demo, "SELECT state,lease_until_ms FROM durable_executions", runtime=True)
    assert len(state) == 1 and state[0]["state"] == "running", state
    assert state[0]["lease_until_ms"] > int(time.time() * 1000), state
    return state[0]["lease_until_ms"]


def crash_boundaries(binary, worker):
    evidence = {}
    for mode in ("runtime_admission", "selection", "reservation", "adapter_return",
                 "adapter_accepted", "adapter_recorded", "plugin_return", "plugin_accepted", "terminal"):
        with Demo(binary, worker, caller_transform=barrier(mode), request_timeout_ms=1500,
                  limits={"callMs": 500}, plugin_proxy=True) as demo:
            status, job = demo.submit(mode)
            assert status == 202
            entered, release = demo.fixture.gates[mode]
            assert entered.wait(5), (mode, demo.caller.logs)
            lease = assert_live_runtime(demo)
            before = rows(demo)[0]
            count_before = demo.fixture.count()
            attempts_before = query(demo, "SELECT * FROM triage_attempts")
            events_before = query(demo, "SELECT event_key FROM durable_events", runtime=True)
            if mode == "runtime_admission":
                assert before["task_id"] is None and not attempts_before
            if mode == "selection":
                assert any(event["event_key"] == "select:1:storage:1" for event in events_before)
            if mode == "reservation":
                assert count_before == 0 and any(a["step"] == "read" and a["outcome_json"] is None for a in attempts_before)
            if mode in ("adapter_return", "adapter_accepted", "adapter_recorded"):
                assert count_before == 1
                slot = next(a for a in attempts_before if a["step"] == "read")
                assert (slot["outcome_json"] is None) == (mode == "adapter_return")
                assert any(event["event_key"] == "read:1:storage:1" for event in events_before) == (mode == "adapter_recorded")
            if mode == "terminal":
                assert before["state"] == "succeeded"
                assert not any(event["event_key"] == "terminal:storage:1" for event in events_before)
            demo.stop_caller(crash=True)
            demo.fixture.subject = "Changed after crash"
            release.set()
            demo.start_caller()
            final = completed(demo, job["jobId"])
            assert final["state"] == "succeeded", (mode, final)
            assert demo.submit(mode)[1]["jobId"] == job["jobId"]
            expected_reads = 2 if mode == "adapter_return" else 1
            assert demo.fixture.count() == expected_reads, (mode, demo.fixture.requests)
            selected = [p for p in demo.fixture.plugin_requests if p["stepId"] == "1"]
            assert len(selected) == (2 if mode == "plugin_return" else 1), (mode, selected)
            old = mode in ("adapter_accepted", "adapter_recorded", "terminal")
            assert final["result"]["summary"].endswith("Checkout unavailable" if old else "Changed after crash"), (mode, final)
            evidence[mode] = dict(physicalReads=demo.fixture.count(), selectAttempts=len(selected),
                                  reservationsBefore=len(attempts_before), leaseUntil=lease,
                                  businessState=final["state"], durableCompletion="matched")
    return dict(cases=["C07", "C08"], boundaries=evidence)


def lost_admission_and_deployment(binary, worker):
    with Demo(binary, worker, dispatch=False, caller_transform=barrier("admission"), request_timeout_ms=1500) as demo:
        entered, release = demo.fixture.gates["admission"]
        with ThreadPoolExecutor(1) as pool:
            future = pool.submit(demo.submit, "lost")
            assert entered.wait(5)
            saved = rows(demo)[0]
            assert demo.fixture.count() == 0
            demo.stop_caller(crash=True)
            try:
                future.result(timeout=5)
            except (OSError, json.JSONDecodeError):
                pass
        release.set()
        # Simulate an incompatible replacement of the live checkout. The namespace
        # must restore its pinned full bundle before repairing the outbox.
        source = demo.caller_dir / "src/index.ts"
        original = source.read_bytes()
        source.write_text("export default {fetch(){return new Response('new deployment')}};")
        demo.start_caller()
        assert source.read_bytes() == original
        final = completed(demo, saved["id"])
        assert final["state"] == "succeeded"
        assert demo.submit("lost")[1]["jobId"] == saved["id"]
        assert demo.fixture.count() == 1
        return dict(cases=["C08"], lostAcknowledgement=True, originalBundleRestored=True,
                    sameAdmission=True, physicalReads=1)


def received_request_and_revocation(binary, worker):
    results = {}
    for mode in ("response_lost", "revoked", "moved", "expired", "plugin_version"):
        limits = {"callMs": 1000, "jobMs": 900 if mode == "expired" else 120000}
        with Demo(binary, worker, limits=limits, request_timeout_ms=1500) as demo:
            demo.fixture.release.clear()
            _, job = demo.submit(mode)
            assert demo.fixture.arrived.wait(5)
            assert_live_runtime(demo)
            demo.stop_caller(crash=True)
            demo.fixture.subject = "New subject"
            if mode in ("revoked", "moved"):
                demo.config["customers"][0]["tickets"] = []
            if mode == "moved":
                demo.fixture.records["a-01"] = "customer-b"
            if mode == "plugin_version":
                demo.config["pluginDigest"] = "incompatible-plugin"
            demo.fixture.release.set()
            demo.start_caller()
            final = demo.wait(job["jobId"], timeout=18)
            if mode == "response_lost":
                assert final["state"] == "succeeded" and final["result"]["summary"].endswith("New subject"), final
                assert demo.fixture.count() == 2
                completed(demo, job["jobId"])
            elif mode == "expired":
                assert final["state"] == "expired", final
                completed(demo, job["jobId"])
            else:
                expected = "VERSION_MISMATCH" if mode == "plugin_version" else "FORBIDDEN_TICKET"
                assert final["error"] == expected and final["state"] == "failed", final
            assert final["deadlineAt"] == job["deadlineAt"]
            assert demo.fixture.count() == (2 if mode == "response_lost" else 1)
            assert all(r["authorized"] for r in demo.fixture.requests)
            results[mode] = dict(state=final["state"], error=final["error"], reads=demo.fixture.count(), deadlinePreserved=True)
    return dict(cases=["C07", "C08", "C09"], variants=results)


def repeated_crashes(binary, worker):
    def transform(demo):
        gates = [gate_call(demo, f"reserve-{i}") for i in (1, 2)]
        needle = "    const slot = await reserve(job, step, ordinal);"
        edit(demo, "workflow.ts", needle, needle + '\n if (step === "read") { if (ordinal === 1) { ' + gates[0] + ' } else { ' + gates[1] + ' } }')
    with Demo(binary, worker, caller_transform=transform, request_timeout_ms=1500) as demo:
        _, job = demo.submit("repeated")
        for i in (1, 2):
            entered, release = demo.fixture.gates[f"reserve-{i}"]
            assert entered.wait(15), (i, demo.caller.logs)
            assert_live_runtime(demo)
            assert len(query(demo, "SELECT * FROM triage_attempts WHERE step='read'")) == i
            assert demo.fixture.count() == 0
            demo.stop_caller(crash=True)
            release.set()
            demo.start_caller()
        final = completed(demo, job["jobId"])
        assert final["state"] == "failed" and final["error"] == "ATTEMPTS_EXHAUSTED", final
        assert demo.fixture.count() == 0
        assert len(query(demo, "SELECT * FROM triage_attempts WHERE step='read'")) == 2
        return dict(cases=["C08"], callerCrashes=2, reservedReads=2, physicalReads=0, result=final["error"])


def storage_and_completion(binary, worker):
    outcomes = {}
    for boundary in ("effect", "completion", "admission", "business_result", "business_exhausted"):
        with Demo(binary, worker, dispatch=False, request_timeout_ms=1500) as demo:
            if boundary == "admission":
                mutate(demo, "CREATE TRIGGER storage_fault BEFORE INSERT ON triage_jobs BEGIN SELECT RAISE(FAIL, 'test storage unavailable'); END")
                assert demo.submit("storage")[0] == 503
                assert not rows(demo) and demo.fixture.count() == 0
                mutate(demo, "DROP TRIGGER storage_fault")
            elif boundary in ("business_result", "business_exhausted"):
                mutate(demo, "CREATE TRIGGER storage_fault BEFORE UPDATE ON triage_attempts WHEN NEW.step='read' AND NEW.outcome_json IS NOT NULL BEGIN SELECT RAISE(FAIL, 'application storage fault'); END")
            else:
                target = "durable_events" if boundary == "effect" else "durable_completions"
                condition = "WHEN NEW.event_key = 'read:1:storage:1'" if boundary == "effect" else ""
                mutate(demo, f"CREATE TRIGGER storage_fault BEFORE INSERT ON {target} {condition} BEGIN SELECT RAISE(FAIL, 'test storage unavailable'); END", runtime=True)
            status, job = demo.submit("storage")
            assert status == 202, (boundary, status, job)
            demo.start_driver()
            if boundary in ("business_result", "business_exhausted"):
                wait_for(lambda: any("APPLICATION_STORAGE_UNAVAILABLE" in item["payload"] for item in query(demo, "SELECT payload FROM durable_events WHERE kind='retry'", runtime=True)))
                assert rows(demo)[0]["result_json"] is None
                if boundary == "business_result":
                    mutate(demo, "DROP TRIGGER storage_fault")
            elif boundary != "admission":
                wait_for(lambda: demo.fixture.count() == 1)
                if boundary == "completion":
                    wait_for(lambda: rows(demo)[0]["state"] == "succeeded")
                else:
                    wait_for(lambda: any(a["outcome_json"] for a in query(demo, "SELECT * FROM triage_attempts WHERE step='read'")))
                wait_for(lambda: 503 in demo.dispatch_observations)
                assert query(demo, "SELECT count(*) AS n FROM durable_completions", runtime=True)[0]["n"] == 0
                assert query(demo, "SELECT state FROM durable_executions", runtime=True)[0]["state"] == "running"
                if boundary == "effect":
                    assert rows(demo)[0]["result_json"] is None
                mutate(demo, "DROP TRIGGER storage_fault", runtime=True)
            final = completed(demo, job["jobId"])
            expected = 2 if boundary == "business_result" else 1
            assert final["state"] == ("failed" if boundary == "business_exhausted" else "succeeded"), final
            if boundary == "business_exhausted":
                assert final["error"] == "STORAGE_UNAVAILABLE" and final["result"] is None
            assert demo.fixture.count() == expected, final
            outcomes[boundary] = dict(state=final["state"], physicalReads=expected, completionMatches=True)
    return dict(cases=["C08"], boundaries=outcomes)


def stale_owner(binary, worker):
    def transform(demo):
        gate = gate_call(demo, "old-response")
        edit(demo, "workflow.ts", "async function physical(", "export async function physical(")
        needle = "    const outcome: Outcome = { ok: true, value };"
        edit(demo, "workflow.ts", needle, 'if (step === "read" && ordinal === 1) { ' + gate + ' }\n' + needle)
        edit(demo, "index.ts", 'import { config, triage }', 'import { config, triage, physical }')
        edit(demo, "index.ts", 'import { admit, byKey, find, initialize, publicJob }', 'import { admit, byKey, claim, find, finish, initialize, publicJob }')
        needle = '        if (path === "/internal/pending"'
        route = '''        if (path === "/internal/test-replace" && request.method === "POST") {
          const input = await request.json();
          const old = (await find(input.jobId))!;
          const replacement = await claim(input.jobId, cfg.bootId);
          const burned = await physical(replacement, "read", 1, {ticketId: "a-01"});
          const accepted = await physical(replacement, "read", 2, {ticketId: "a-01"});
          if (!accepted.ok) throw new Error("replacement failed");
          const saved = await finish(replacement, {customerId: "customer-a", summary: "replacement result"}, null);
          await finish(old, {summary: "STALE TERMINAL"}, null);
          return Response.json({burned, accepted, saved});
        }
'''
        edit(demo, "index.ts", needle, route + needle)
    with Demo(binary, worker, caller_transform=transform, request_timeout_ms=10000) as demo:
        _, job = demo.submit("stale")
        entered, release = demo.fixture.gates["old-response"]
        assert entered.wait(5)
        first = query(demo, "SELECT * FROM triage_attempts WHERE step='read'")[0]
        demo.fixture.subject = "Replacement detail"
        status, replacement = http(demo.caller.origin, "/internal/test-replace", "POST",
                                   {"jobId": job["jobId"]}, demo.config["dispatchToken"])
        assert status == 200, replacement
        assert replacement["burned"]["error"] == "UNKNOWN_OUTCOME"
        release.set()
        runtime_state = wait_for(lambda: next((row["state"] for row in query(demo, "SELECT state FROM durable_executions", runtime=True)
                                              if row["state"] in ("failed", "completed")), None))
        final = demo.wait(job["jobId"])
        # A replay may reconcile the replacement's committed business result;
        # the required invariant is unchanged accepted data, not task failure.
        if runtime_state == "completed":
            assert json.loads(query(demo, "SELECT result_json FROM durable_completions", runtime=True)[0]["result_json"]) == final
        assert final == replacement["saved"] and final["result"]["summary"] == "replacement result"
        slots = query(demo, "SELECT * FROM triage_attempts WHERE step='read' ORDER BY ordinal")
        assert slots[0]["attempt_id"] == first["attempt_id"]
        assert json.loads(slots[0]["outcome_json"])["error"] == "UNKNOWN_OUTCOME"
        assert json.loads(slots[1]["outcome_json"])["value"]["subject"] == "Replacement detail"
        assert demo.fixture.count() == 2
        return dict(cases=["C09"], applicationOwners=2, schedulers=1, lateResponseAccepted=False,
                    staleTerminalAccepted=False, physicalReads=2, runtimeState=runtime_state, terminalResult=final["result"])


def retry_policy_and_legacy(binary, worker):
    outcomes = {}
    for mode in ("transient", "rate-limit", "unavailable", "reject", "backoff-expiry"):
        with Demo(binary, worker, limits={"jobMs": 1000 if mode == "backoff-expiry" else 120000}, request_timeout_ms=1500) as demo:
            demo.fixture.mode = "unavailable" if mode == "backoff-expiry" else mode
            if mode == "backoff-expiry":
                demo.fixture.release.clear()
            _, job = demo.submit(mode)
            if mode == "backoff-expiry":
                assert demo.fixture.arrived.wait(5)
                time.sleep(max(0, (job["deadlineAt"] - time.time() * 1000 - 100) / 1000))
                demo.fixture.release.set()
            final = completed(demo, job["jobId"])
            if mode in ("transient", "rate-limit"):
                assert final["state"] == "succeeded", (mode, final)
            else:
                expected = {"unavailable": "ATTEMPTS_EXHAUSTED", "reject": "UPSTREAM_REJECTED", "backoff-expiry": "DEADLINE_EXCEEDED"}[mode]
                assert final["error"] == expected, (mode, final)
            count = 1 if mode in ("reject", "backoff-expiry") else 2
            assert demo.fixture.count() == count, (mode, demo.fixture.requests)
            if count == 2:
                assert demo.fixture.requests[1]["at"] - demo.fixture.requests[0]["at"] >= .24
            outcomes[mode] = dict(state=final["state"], error=final["error"], physicalReads=count)
    with Demo(binary, worker, dispatch=False) as demo:
        _, kept = demo.submit("legacy-complete")
        _, interrupted = demo.submit("legacy-active")
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
                    row["state"] = "succeeded"
                    row["result_json"] = json.dumps({"summary": "retained P1 result"})
                db.execute("INSERT INTO triage_jobs (" + columns + ") VALUES (" + ",".join("?" for _ in row) + ")", list(row.values()))
        demo.start_caller()
        assert demo.wait(kept["jobId"])["result"]["summary"] == "retained P1 result"
        assert demo.wait(interrupted["jobId"])["error"] == "INTERRUPTED"
        assert demo.submit("legacy-complete")[1]["jobId"] == kept["jobId"]
        assert demo.fixture.count() == 0
        assert all(row["execution_version"] == 1 and row["delivery"] == "legacy" for row in rows(demo))
        assert query(demo, "SELECT count(*) AS n FROM durable_programs", runtime=True)[0]["n"] == 0
    return dict(cases=["C08", "C09"], retries=outcomes, legacyResultsPreserved=True, legacyJobsRedispatched=False)


def outbox_fairness(binary, worker):
    with Demo(binary, worker, dispatch=False) as demo:
        for i in range(8):
            token, ticket = ("demo-a", "a-01") if i < 4 else ("demo-b", "b-01")
            assert demo.submit("unavailable-" + str(i), [ticket], token)[0] == 202
        with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
            for job in rows(demo):
                definition = json.loads(job["definition_json"])
                definition["callerDigest"] = "unavailable-original-bundle"
                db.execute("UPDATE triage_jobs SET state='failed',error='FIXTURE_UNAVAILABLE',definition_json=? WHERE id=?", (json.dumps(definition), job["id"]))
        status, fresh = demo.submit("fresh")
        assert status == 202
        demo.start_driver()
        final = completed(demo, fresh["jobId"])
        assert final["state"] == "succeeded" and demo.fixture.count() == 1
        pending = query(demo, "SELECT * FROM triage_jobs WHERE delivery='pending'")
        assert len(pending) == 8 and all(job["delivery_checked_at"] > 0 for job in pending)
        mutate(demo, "UPDATE triage_jobs SET delivery_until=0 WHERE delivery='pending'")
        wait_for(lambda: len(query(demo, "SELECT id FROM triage_jobs WHERE delivery='unresolved'")) == 8)
        assert query(demo, "SELECT count(*) AS n FROM durable_programs", runtime=True)[0]["n"] == 1
        return dict(cases=["C08"], unavailableOutboxes=8, healthyJobCompleted=True,
                    replacementTasks=0, deliveryHorizonEnforced=True)


CASES = (crash_boundaries, lost_admission_and_deployment, received_request_and_revocation,
         repeated_crashes, storage_and_completion, stale_owner, retry_policy_and_legacy, outbox_fairness)


def main():
    if not __debug__:
        raise RuntimeError("acceptance assertions must be enabled")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path(os.environ.get("TYSEL_GATE_BIN_DIR", REPO / "target/debug")))
    parser.add_argument("--output", type=Path, default=Path("agent-triage-recovery-report.json"))
    parser.add_argument("--case", choices=[case.__name__ for case in CASES])
    args = parser.parse_args()
    report = dict(schemaVersion=1, stage="P2", status="running", cases=[])
    try:
        for case in CASES:
            if args.case and case.__name__ != args.case:
                continue
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
