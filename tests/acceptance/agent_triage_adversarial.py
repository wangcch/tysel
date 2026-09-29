#!/usr/bin/env python3
"""P3 adversarial contracts against isolated workers and independent wire counters."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from collections import Counter
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import threading
import time
import traceback
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("triage_recovery", Path(__file__).with_name("agent_triage_recovery.py"))
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)
base = recovery.base
Demo, http, rows = base.Demo, base.http, base.rows
query, mutate, wait_for = recovery.query, recovery.mutate, recovery.wait_for
HOSTILE = Path(__file__).with_name("fixtures") / "triage-hostile.ts"
AUDIT_FIELDS = {"job_id", "event_key", "step", "operation", "ordinal", "attempt_id", "decision", "outcome", "at_ms"}


def hostile(mode):
    return HOSTILE.read_text().replace("__MODE__", mode)


def check_audit(demo):
    records = demo.audit()
    jobs = rows(demo)
    for job in jobs:
        events = [event for event in records if event["job_id"] == job["id"]]
        assert 2 <= len(events) <= 20, events
        assert sum(event["decision"] == "admit" for event in events) == 1
        assert sum(event["decision"] == "terminal" for event in events) == 1
    for event in records:
        assert set(event) == AUDIT_FIELDS
        assert event["operation"] in {"job.create", "job.complete", "plugin.select", "plugin.summarize", "ticket.read"}
        assert event["outcome"].replace("_", "").isupper(), event
    for attempt in query(demo, "SELECT * FROM triage_attempts"):
        event = next(event for event in records if event["job_id"] == attempt["job_id"] and event["event_key"] == f'reserve:{attempt["step"]}:{attempt["ordinal"]}')
        assert event["attempt_id"] == attempt["attempt_id"]
        if attempt["outcome_json"]:
            event = next(event for event in records if event["job_id"] == attempt["job_id"] and event["event_key"] == f'outcome:{attempt["step"]}:{attempt["ordinal"]}')
            assert event["attempt_id"] == attempt["attempt_id"]
    base.no_secrets(demo)
    return len(records)


def raw_submit(demo, body, key):
    request = urllib.request.Request(demo.caller.origin + "/jobs", data=body, method="POST", headers={
        "Authorization": "Bearer demo-a", "Idempotency-Key": key, "Content-Type": "application/json"})
    try:
        response = urllib.request.urlopen(request, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.loads(response.read())


def malformed_admission(binary, worker):
    attacks = {
        "duplicate": b'{"protocolVersion":2,"protocolVersion":1,"ticketIds":[]}',
        "escaped-duplicate": b'{"protocolVersion":2,"protocolVer\\u0073ion":1,"ticketIds":[]}',
        "nested-duplicate": b'{"protocolVersion":1,"ticketIds":[{"id":1,"id":2}]}',
        "utf8": b'{"protocolVersion":1,"ticketIds":["\xff"]}',
        "trailing": b'{"protocolVersion":1,"ticketIds":[]}{}',
        "depth": b'[' * 33 + b'0' + b']' * 33,
    }
    with Demo(binary, worker, dispatch=False) as demo:
        for name, body in attacks.items():
            status, reply = raw_submit(demo, body, name)
            assert status == 400 and reply["error"] == "INVALID_JSON", (name, status, reply)
        assert not rows(demo) and not demo.audit() and demo.fixture.count() == 0
    return dict(cases=["C03", "C10"], rejected=list(attacks), admitted=0, physicalReads=0)


def hostile_plugins(binary, worker):
    variants = ("delete", "url", "customer", "other-ticket", "lower-priority", "wrong-version", "wrong-job",
                "wrong-step", "wrong-attempt", "array", "extra-envelope", "duplicate-envelope", "duplicate-payload", "malformed", "utf8", "oversized", "rounds", "final-customer", "final-extra", "final-action")
    outcomes = {}
    for name in variants:
        with Demo(binary, worker, plugin_source=hostile(name), plugin_proxy=True) as demo:
            _, job = demo.submit("hostile-" + name)
            final = demo.wait(job["jobId"])
            assert final["state"] == ("succeeded" if name == "final-action" else "failed"), (name, final)
            count = 1 if name in ("rounds", "final-customer", "final-extra", "final-action") else 0
            assert demo.fixture.count() == count, (name, demo.fixture.requests)
            assert len(demo.fixture.plugin_requests) == (2 if count else 1), (name, final, demo.fixture.plugin_responses, demo.plugin.logs[-8:])
            if name == "final-action":
                assert final["result"]["summary"].startswith("Please delete")
            check_audit(demo)
            outcomes[name] = dict(outcome=final["error"] or "TEXT_ONLY", physicalReads=count)
    return dict(cases=["C04", "C10"], variants=outcomes, forbiddenPhysicalReads=0)


def response_correlation(binary, worker):
    observations = {}
    for schedule in ("serial", "concurrent", "earlier-attempt"):
        with Demo(binary, worker, plugin_proxy=True) as demo:
            captured = []
            ready = threading.Event()
            lock = threading.Lock()
            def reply(envelope, status, body):
                with lock:
                    captured.append((envelope, body))
                    position = len(captured)
                    if position == 2:
                        ready.set()
                if schedule == "earlier-attempt":
                    return (503, b'{}') if position == 1 else (200, captured[0][1])
                if schedule == "serial":
                    return (status, body) if position == 1 else (200, captured[0][1])
                assert ready.wait(4), "cross-job barrier was not reached"
                return 200, captured[1 if position == 1 else 0][1]
            demo.fixture.plugin_reply = reply
            _, first = demo.submit("wire-shared", [] if schedule == "serial" else None)
            if schedule == "serial":
                assert demo.wait(first["jobId"])["state"] == "succeeded"
            jobs = [(first, "demo-a")]
            if schedule != "earlier-attempt":
                _, second = demo.submit("wire-shared", ["b-01"], "demo-b")
                jobs.append((second, "demo-b"))
            finals = [demo.wait(job["jobId"], token) for job, token in jobs]
            for final in finals[1 if schedule == "serial" else 0:]:
                assert final["error"] == "PROTOCOL_ERROR", (schedule, finals)
            assert demo.fixture.count() == 0 and len(captured) == 2
            if schedule == "earlier-attempt":
                assert captured[0][0]["jobId"] == captured[1][0]["jobId"]
                assert captured[0][0]["stepId"] == captured[1][0]["stepId"]
                assert captured[0][0]["attemptId"] != captured[1][0]["attemptId"]
            check_audit(demo)
            observations[schedule] = dict(pluginRequests=2, physicalReads=0, errors=[final["error"] for final in finals])
    return dict(cases=["C09", "C10"], schedules=observations)


def concurrent_capacity_recovery(binary, worker):
    with Demo(binary, worker, dispatch=False, plugin_proxy=True, request_timeout_ms=1500) as demo:
        demo.fixture.release.clear()
        entries = [(customer, key) for customer in ("a", "b") for key in range(4)]
        def submit(entry):
            customer, key = entry
            return demo.submit(f"concurrent-{key}", [f"{customer}-{key:02}"], "demo-" + customer)
        with ThreadPoolExecutor(max_workers=8) as pool:
            admitted = list(pool.map(submit, entries))
            duplicates = list(pool.map(submit, entries * 2))
        assert all(status == 202 for status, _ in admitted + duplicates)
        ids = {job["jobId"] for _, job in admitted}
        assert len(ids) == 8 and {job["jobId"] for _, job in duplicates} == ids
        for customer in ("a", "b"):
            assert demo.submit("one-over", [f"{customer}-01"], "demo-" + customer)[0] == 429
        assert demo.submit("forged", ["b-01"], "demo-a")[0] == 403
        assert demo.submit("unauthorized", [], "wrong")[0] == 401
        for (_, job), (customer, _) in zip(admitted, entries):
            assert http(demo.caller.origin, "/jobs/" + job["jobId"], token="demo-" + ("b" if customer == "a" else "a"))[0] == 404
        demo.start_driver()
        wait_for(lambda: demo.fixture.count() == 8)
        demo.stop_caller(crash=True)
        demo.fixture.release.set()
        demo.start_caller(dispatch=False)
        with ThreadPoolExecutor(max_workers=8) as pool:
            restarted = list(pool.map(submit, entries * 2))
        assert {job["jobId"] for _, job in restarted} == ids
        demo.start_driver()
        for (_, job), (customer, key) in zip(admitted, entries):
            final = demo.wait(job["jobId"], "demo-" + customer, timeout=20)
            assert final["state"] == "succeeded", final
            assert final["result"]["summary"].startswith(f"Prioritize {customer}-{key:02}:")
            assert http(demo.caller.origin, "/jobs/" + job["jobId"], token="demo-" + ("b" if customer == "a" else "a"))[0] == 404
        wait_for(lambda: all(job["delivery"] == "completed" for job in rows(demo)))
        assert demo.fixture.count() == 16 and all(read["authorized"] for read in demo.fixture.requests)
        assert Counter(read["ticketId"] for read in demo.fixture.requests) == {f"{customer}-{key:02}": 2 for customer, key in entries}
        assert query(demo, "SELECT count(*) AS n FROM durable_programs", runtime=True)[0]["n"] == 8
        assert not query(demo, "SELECT job_id,step FROM triage_attempts GROUP BY job_id,step HAVING count(*) > 2")
        for envelope, headers in zip(demo.fixture.plugin_requests, demo.fixture.plugin_headers):
            snapshot = envelope["payload"]
            prefix = snapshot["customerId"][-1] + "-"
            assert all(ticket["id"].startswith(prefix) for ticket in snapshot["tickets"])
            assert set(snapshot) == ({"customerId", "tickets"} if envelope["stepId"] == "1" else {"customerId", "tickets", "detail"})
            assert not ({name.lower() for name in headers} & {"authorization", "idempotency-key", "x-customer-id"})
            if envelope["stepId"] == "2":
                assert set(snapshot["detail"]) == {"ticketId", "subject"}
        audits = check_audit(demo)
        return dict(cases=["C01", "C03", "C07", "C09", "C10"], customers=2, activeJobs=8, duplicateSubmissions=32,
                    physicalReads=16, maximumReadsPerJob=2, auditRows=audits, runtimePrograms=8)


def suspended_revocation(binary, worker):
    def transform(demo):
        gate = recovery.gate_call(demo, "retry")
        needle = '    const outcome = await stored(ctx, `${step}:${ordinal}`, () => physical(job, step, ordinal, payload));'
        recovery.edit(demo, "workflow.ts", needle, '    if (step === "read" && ordinal === 2) { ' + gate + ' }\n' + needle)
    observations = {}
    for mode in ("backoff", "inflight", "recorded"):
        with Demo(binary, worker, plugin_proxy=True, caller_transform=transform if mode == "backoff" else recovery.barrier("adapter_recorded") if mode == "recorded" else None) as demo:
            if mode == "backoff":
                demo.fixture.mode = "transient"
            if mode == "inflight":
                demo.fixture.release.clear()
            _, job = demo.submit(mode)
            if mode == "inflight":
                assert demo.fixture.arrived.wait(5)
                release = demo.fixture.release
            else:
                arrived, release = demo.fixture.gates["retry" if mode == "backoff" else "adapter_recorded"]
                assert arrived.wait(5)
            if mode == "backoff":
                assert query(demo, "SELECT * FROM durable_events WHERE kind='sleep'", runtime=True), "real Durable backoff must be recorded"
            demo.config["customers"][0]["tickets"] = []
            (demo.caller_dir / "config/service.json").write_text(json.dumps(demo.config))
            for _ in range(4):
                assert demo.submit(mode)[1]["jobId"] == job["jobId"]
            release.set()
            final = recovery.completed(demo, job["jobId"])
            assert final["error"] == ("FORBIDDEN_TICKET" if mode == "backoff" else None), final
            assert demo.fixture.count() == 1
            if mode == "backoff":
                assert any(event["outcome"] == "FORBIDDEN_TICKET" and event["ordinal"] == 2 for event in demo.audit())
            check_audit(demo)
            observations[mode] = dict(outcome=final["error"] or "SUCCEEDED", physicalReads=1, readsAfterRevocation=0)
    return dict(cases=["C09", "C10"], scenarios=observations)


def streaming_limits(binary, worker):
    observations = {}
    for kind, size in (("plugin", 8192), ("plugin", 8193), ("adapter", 16384), ("adapter", 16385)):
        with Demo(binary, worker, plugin_proxy=True) as demo:
            demo.fixture.chunked = True
            def padded(_, status, body):
                return status, body + b' ' * (size - len(body))
            if kind == "plugin":
                demo.fixture.plugin_reply = padded
            else:
                demo.fixture.adapter_reply = padded
            _, job = demo.submit(f"{kind}-{size}")
            final = demo.wait(job["jobId"])
            assert final["error"] == ("MESSAGE_TOO_LARGE" if size % 2 else None), (kind, size, final, query(demo, "SELECT * FROM triage_attempts"))
            expected = 0 if kind == "plugin" and size == 8193 else 1
            assert demo.fixture.count() == expected
            check_audit(demo)
            observations[f"{kind}-{size}"] = dict(outcome=final["error"] or "SUCCEEDED", physicalReads=expected, chunked=True)
    for mode in ("summary-limit", "summary-over", "subject-limit", "subject-over", "slow", "duplicate-adapter"):
        with Demo(binary, worker, plugin_proxy=True, plugin_source=hostile(mode) if mode.startswith("summary") else None,
                  limits={"callMs": 150} if mode == "slow" else None) as demo:
            if mode.startswith("subject"):
                demo.fixture.subject = "😀" * 250 + ("x" if mode.endswith("over") else "")
            if mode == "slow":
                demo.fixture.chunked = True
                demo.fixture.body_delay = .3
                demo.fixture.adapter_reply = lambda _, status, body: (status, body + b' ' * 1024)
            if mode == "duplicate-adapter":
                demo.fixture.adapter_reply = lambda _, status, body: (status, b'{"subject":"forged",' + body[1:])
            _, job = demo.submit(mode, [] if mode.startswith("summary") else None)
            final = demo.wait(job["jobId"])
            expected = {"summary-over": "PROTOCOL_ERROR", "subject-over": "INVALID_ADAPTER_RESULT", "slow": "ATTEMPTS_EXHAUSTED", "duplicate-adapter": "INVALID_JSON"}.get(mode)
            assert final["error"] == expected, (mode, final)
            reads = 0 if mode.startswith("summary") else 2 if mode == "slow" else 1
            assert demo.fixture.count() == reads
            check_audit(demo)
            observations[mode] = dict(outcome=final["error"] or "SUCCEEDED", physicalReads=reads)
    return dict(cases=["C03", "C04", "C05", "C10"], boundaries=observations)


def least_authority(binary, worker):
    canary = "unrelated-plugin-local-file-canary"
    def transform(demo, plugin_dir):
        (plugin_dir / "data").mkdir()
        (plugin_dir / "data/secret.txt").write_text(canary)
        path = plugin_dir / "src/index.ts"
        path.write_text(path.read_text().replace("__ORIGIN__", demo.fixture.origin))
    with Demo(binary, worker, plugin_source=hostile("permission-probes"), plugin_transform=transform, plugin_proxy=True) as demo:
        final = base.successful(demo, "probe-raw-idempotency-key-canary", [])
        assert final["result"]["summary"] == "network:denied,file:denied,secret:denied", final
        assert demo.fixture.count() == 0
        check_audit(demo)
        captures = json.dumps(rows(demo)) + json.dumps(demo.audit()) + json.dumps(demo.fixture.plugin_requests) + json.dumps(demo.fixture.plugin_headers) + json.dumps(demo.fixture.plugin_responses)
        captures += "".join(demo.caller.logs + demo.plugin.logs)
        captures += json.dumps(query(demo, "SELECT * FROM triage_attempts"))
        captures += json.dumps(query(demo, "SELECT payload FROM durable_events", runtime=True))
        captures += json.dumps(query(demo, "SELECT result_json FROM durable_completions", runtime=True))
        assert canary not in captures and "probe-raw-idempotency-key-canary" not in captures
    return dict(cases=["C10"], denied=["network", "file", "secret"], physicalReads=0, leakedCanaries=0)


def audit_atomicity(binary, worker):
    with Demo(binary, worker, dispatch=False, plugin_proxy=True) as demo:
        _, job = demo.submit("audit-fault")
        mutate(demo, """CREATE TRIGGER reject_audit BEFORE INSERT ON triage_audit
          WHEN NEW.decision='reserve' BEGIN SELECT RAISE(ABORT,'audit unavailable'); END""")
        demo.start_driver()
        final = recovery.completed(demo, job["jobId"])
        assert final["error"] == "STORAGE_UNAVAILABLE", final
        assert demo.fixture.count() == 0 and not demo.fixture.plugin_requests
        assert not query(demo, "SELECT * FROM triage_attempts"), "reservation must roll back with audit failure"
        assert check_audit(demo) == 2
        before = demo.audit()
        for _ in range(12):
            assert demo.submit("audit-fault")[1] == final
        assert demo.audit() == before
        mutate(demo, "DROP TRIGGER reject_audit")
        good = base.successful(demo, "audit-healthy")
        wait_for(lambda: all(job["delivery"] == "completed" for job in rows(demo)))
        assert check_audit(demo) == 10
        audit_text = json.dumps(demo.audit())
        assert demo.fixture.subject not in audit_text and good["result"]["summary"] not in audit_text
        return dict(cases=["C10"], failedAuditReservations=0, failedJobPhysicalReads=0, healthyAuditRows=8, duplicateAddedRows=0,
                    maximumRowsPerJob=20, businessBodiesInAudit=0)


CASES = [malformed_admission, hostile_plugins, response_correlation, concurrent_capacity_recovery,
         suspended_revocation, streaming_limits, least_authority, audit_atomicity]


def main():
    if not __debug__:
        raise RuntimeError("acceptance assertions must be enabled")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path(os.environ.get("TYSEL_GATE_BIN_DIR", REPO / "target/debug")))
    parser.add_argument("--output", type=Path, default=Path("agent-triage-adversarial-report.json"))
    parser.add_argument("--case", choices=[case.__name__ for case in CASES])
    args = parser.parse_args()
    report = dict(schemaVersion=1, stage="P3", status="running", cases=[])
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
