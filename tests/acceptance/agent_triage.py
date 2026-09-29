#!/usr/bin/env python3
"""P1 end-to-end contracts; fake providers only, existing acceptance runner owns evidence."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import sys
import time
import traceback
import urllib.error
import urllib.request

REPO = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("triage_demo", REPO / "examples/agent-triage/run.py")
demo_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(demo_module)
Demo, http = demo_module.Demo, demo_module.http


def rows(demo):
    with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute("SELECT * FROM triage_jobs ORDER BY created_at,id")]


def no_secrets(demo):
    payload = json.dumps(demo.audit()) + json.dumps(demo.fixture.plugin_requests) + json.dumps(demo.fixture.plugin_headers) + json.dumps(demo.fixture.plugin_responses) + json.dumps(rows(demo)) + "".join(demo.history + demo.caller.logs + demo.plugin.logs)
    with sqlite3.connect(demo.caller_dir / "data/jobs.db") as db:
        payload += json.dumps(db.execute("SELECT * FROM triage_attempts").fetchall())
    with sqlite3.connect(demo.caller_dir / "data/durable-events.db") as db:
        payload += json.dumps(db.execute("SELECT payload FROM durable_events").fetchall())
        payload += json.dumps(db.execute("SELECT result_json FROM durable_completions").fetchall())
    for canary in (demo_module.FAKE_SECRET, demo_module.PRIVATE_FIELD):
        assert canary not in payload, "private data entered persisted state or logs"


def successful(demo, key, ids=None, token="demo-a"):
    status, job = demo.submit(key, ids, token)
    assert status == 202, (status, job)
    final = demo.wait(job["jobId"], token)
    assert final["state"] == "succeeded", (final, demo.plugin.logs[-15:], demo.caller.logs[-15:])
    return final


def healthy_identity(binary, worker):
    with Demo(binary, worker) as demo:
        empty = successful(demo, "empty", [])
        assert empty["result"]["summary"] == "No open tickets."
        assert demo.fixture.count() == 0
        with ThreadPoolExecutor(max_workers=12) as pool:
            attempts = list(pool.map(lambda _: demo.submit("same-key"), range(12)))
        assert all(status == 202 for status, _ in attempts), attempts
        assert len({job["jobId"] for _, job in attempts}) == 1
        job = demo.wait(attempts[0][1]["jobId"])
        assert job["state"] == "succeeded", job
        assert job["result"]["summary"] == "Prioritize a-01: Checkout unavailable"
        assert demo.fixture.count() == 1
        assert demo.submit("same-key", ["a-00"])[0] == 409
        other = successful(demo, "same-key", ["b-00", "b-01"], "demo-b")
        assert other["jobId"] != job["jobId"] and demo.fixture.count() == 2
        assert http(demo.caller.origin, "/jobs/" + other["jobId"], token="demo-a")[0] == 404
        assert demo.submit("cross-scope", ["b-01"])[0] == 403
        assert http(demo.caller.origin, "/jobs", "POST", {}, "wrong", "key")[0] == 401
        assert http(demo.caller.origin, "/internal/pending", token="demo-a")[0] == 401
        assert demo.fixture.count() == 2
        assert all(item["authorized"] for item in demo.fixture.requests)
        no_secrets(demo)
        return dict(cases=["C01", "C02", "C03"], physicalReads=2, concurrentSubmissions=12,
                    storedJobs=len(rows(demo)), duplicateJobs=1)


def admission_boundaries(binary, worker):
    with Demo(binary, worker, dispatch=False) as demo:
        for body in (None, [], {}, {"protocolVersion": 2, "ticketIds": []},
                     {"protocolVersion": 1, "ticketIds": ["a-00", "a-00"]},
                     {"protocolVersion": 1, "ticketIds": [], "customerId": "customer-b"},
                     {"protocolVersion": 1, "ticketIds": [f"a-{i:02}" for i in range(17)]}):
            assert http(demo.caller.origin, "/jobs", "POST", body, "demo-a", "bad")[0] == 400
        assert demo.submit("x" * 129)[0] == 400
        assert http(demo.caller.origin, "/jobs", "POST", {"protocolVersion": 1, "ticketIds": []}, "demo-a")[0] == 400
        payload = json.dumps({"protocolVersion": 1, "ticketIds": [f"a-{i:02}" for i in range(16)]}).encode()

        def raw(body):
            request = urllib.request.Request(demo.caller.origin + "/jobs", data=body, method="POST",
                headers={"Authorization": "Bearer demo-a", "Idempotency-Key": "boundary", "Content-Type": "application/json"})
            try:
                result = urllib.request.urlopen(request, timeout=5)
            except urllib.error.HTTPError as error:
                result = error
            with result:
                return result.status, json.loads(result.read())

        assert raw(payload + b" " * (16385 - len(payload)))[0] == 413
        status, job = raw(payload + b" " * (16384 - len(payload)))
        assert status == 202
        assert demo.fixture.count() == 0
        demo.start_driver()
        assert demo.wait(job["jobId"])["state"] == "succeeded"
        assert demo.fixture.count() == 1
        return dict(cases=["C04", "C05"], acceptedBodyBytes=16384, rejectedBodyBytes=16385, maxTickets=16)


def capacity_retention(binary, worker):
    with Demo(binary, worker, dispatch=False) as demo:
        accepted = []
        with ThreadPoolExecutor(max_workers=12) as pool:
            attempts = list(pool.map(lambda i: demo.submit("capacity-" + str(i)), range(12)))
        accepted = [job for status, job in attempts if status == 202]
        assert len(accepted) == 4 and sum(status == 429 for status, _ in attempts) == 8
        for i in range(4):
            assert demo.submit("b-" + str(i), ["b-01"], "demo-b")[0] == 202
        assert len(rows(demo)) == 8 and demo.fixture.count() == 0
        assert demo.submit("overflow-b", ["b-01"], "demo-b")[0] == 429
        # At capacity, retained identities are still returned before new-job rejection.
        accepted_key = next("capacity-" + str(i) for i, (status, _) in enumerate(attempts) if status == 202)
        assert demo.submit(accepted_key)[0] == 202
    with Demo(binary, worker, limits={"retained": 1}) as demo:
        job = successful(demo, "retained")
        assert demo.submit("retained")[1]["jobId"] == job["jobId"]
        assert demo.submit("new")[0] == 503 and demo.fixture.count() == 1
    return dict(cases=["C02", "C05"], perCustomer=4, totalActive=8, overflowRejected=8, retentionCap=1)


def restart_disconnect(binary, worker):
    with Demo(binary, worker, request_timeout_ms=1500) as demo:
        completed = successful(demo, "completed")
        demo.fixture.arrived.clear()
        demo.fixture.release.clear()
        # The independent driver continues even when this client never reads its response.
        from urllib.parse import urlsplit
        url = urlsplit(demo.caller.origin)
        body = json.dumps({"protocolVersion": 1, "ticketIds": ["a-01"]}).encode()
        with socket.create_connection((url.hostname, url.port), timeout=5) as connection:
            connection.sendall(("POST /jobs HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer demo-a\r\n"
                                "Idempotency-Key: disconnected\r\nContent-Type: application/json\r\n"
                                f"Content-Length: {len(body)}\r\nConnection: close\r\n\r\n").encode() + body)
            # Prove admission happened before disconnecting; a socket closed before
            # the request reaches the service carries no durability promise.
            assert demo.fixture.arrived.wait(5), "persisted admission was not dispatched"
        status, admitted = demo.submit("disconnected", ["a-01"])
        assert status == 202
        demo.fixture.release.set()
        resumed = demo.wait(admitted["jobId"])
        assert resumed["state"] == "succeeded", resumed
        assert demo.fixture.count() == 2
        demo.fixture.arrived.clear()
        demo.fixture.release.clear()
        status, running = demo.submit("crash")
        assert status == 202 and demo.fixture.arrived.wait(5)
        before = demo.fixture.count()
        demo.stop_caller(crash=True)
        demo.fixture.release.set()
        demo.start_caller()
        interrupted = demo.wait(running["jobId"])
        assert interrupted["state"] == "succeeded", interrupted
        assert demo.submit("crash")[1]["jobId"] == running["jobId"]
        assert demo.wait(completed["jobId"]) == completed
        assert demo.fixture.count() == before + 1
        # A later configuration change cannot reinterpret a retained submission.
        demo.stop_caller()
        demo.config["customers"][0]["tickets"] = []
        demo.start_caller()
        assert demo.submit("completed")[1]["jobId"] == completed["jobId"]
        assert demo.fixture.count() == before + 1
        no_secrets(demo)
        return dict(cases=["C06", "C06a"], readsBeforeRestart=before,
                    readsAfterRestart=demo.fixture.count(), interruptedState=interrupted["state"])


def expiry_timeout(binary, worker):
    with Demo(binary, worker, limits={"jobMs": 700}) as demo:
        demo.fixture.release.clear()
        status, job = demo.submit("expires")
        assert status == 202 and demo.fixture.arrived.wait(5)
        expired = demo.wait(job["jobId"])
        assert expired["state"] == "expired", expired
        demo.fixture.release.set()
        assert demo.wait(job["jobId"]) == expired
        assert rows(demo)[0]["plugin_calls"] == 1 and rows(demo)[0]["read_attempts"] == 1
        demo.stop_caller()
        demo.start_caller()
        assert demo.wait(job["jobId"]) == expired
        assert demo.submit("expires")[1]["deadlineAt"] == job["deadlineAt"]
    with Demo(binary, worker, limits={"callMs": 300}) as demo:
        demo.fixture.release.clear()
        _, job = demo.submit("timeout")
        assert demo.fixture.arrived.wait(5)
        result = demo.wait(job["jobId"])
        assert result["state"] == "failed" and result["error"] == "ATTEMPTS_EXHAUSTED", result
        assert demo.fixture.count() == 2
        demo.fixture.release.set()
    return dict(cases=["C05", "C06"], lateResultAccepted=False, originalDeadlinePreserved=True)


def hostile_plugin(binary, worker):
    variants = {
        "forged-operation": "payload = {kind:'lookup', operation:'ticket.delete', ticketId:'a-01'};",
        "cross-customer": "payload = {kind:'lookup', operation:'ticket.read', ticketId:'b-01'};",
        "extra-url": "payload = {kind:'lookup', operation:'ticket.read', ticketId:'a-01', url:'http://127.0.0.1'};",
        "stale-job": "input.jobId = 'stale';",
        "stale-step": "input.stepId = '9';",
        "stale-attempt": "input.attemptId = 'stale';",
        "oversized": "payload = {kind:'done', customerId:'customer-a', summary:'x'.repeat(9000)};",
        "second-lookup": "/* Never produces done, even after detail is received. */",
    }
    outcomes = {}
    for name, change in variants.items():
        source = """export default { async fetch(request) {
          const input = await request.json();
          let payload = {kind:'lookup', operation:'ticket.read', ticketId:'a-01'};
          CHANGE
          return Response.json({...input, payload});
        }};""".replace("CHANGE", change)
        with Demo(binary, worker, plugin_source=source) as demo:
            _, job = demo.submit(name)
            result = demo.wait(job["jobId"])
            assert result["state"] == "failed", (name, result)
            assert demo.fixture.count() == (1 if name == "second-lookup" else 0), name
            outcomes[name] = result["error"]
    return dict(cases=["C04", "C05"], forbiddenReads=0, repeatedLookupReads=1, variants=outcomes)


def projection_and_message_limit(binary, worker):
    source = """export default { async fetch(request) {
      const input = await request.json();
      if (request.headers.has('authorization')) throw new Error('credential forwarded');
      const expected = input.stepId === '1' ? ['customerId','tickets'] : ['customerId','tickets','detail'];
      if (Object.keys(input.payload).length !== expected.length ||
          expected.some(key => !(key in input.payload))) throw new Error('unprojected input');
      if (input.stepId === '2' && Object.keys(input.payload.detail).sort().join(',') !== 'subject,ticketId') {
        throw new Error('unprojected detail');
      }
      const payload = input.stepId === '1'
        ? {kind:'lookup', operation:'ticket.read', ticketId:'a-01'}
        : {kind:'done', customerId:'customer-a', summary:JSON.stringify(input.payload)};
      return new Response(JSON.stringify({...input, payload}).padEnd(8192, ' '));
    }};"""
    with Demo(binary, worker, plugin_source=source) as demo:
        demo.fixture.mode = "at-limit"
        result = successful(demo, "projection")
        capture = json.loads(result["result"]["summary"])
        assert capture["detail"]["subject"] == demo.fixture.subject
        assert set(capture) == {"customerId", "tickets", "detail"}
        assert set(capture["detail"]) == {"ticketId", "subject"}
        assert rows(demo)[0]["plugin_calls"] == 2
        no_secrets(demo)
        assert demo.fixture.count() == 1
    return dict(cases=["C04", "C05"], pluginResponseBytes=8192, adapterResponseBytes=16384,
                verifiedPluginRequests=2, leakedCanaries=0)


def adapter_boundaries(binary, worker):
    outcomes = {}
    for mode in ("redirect", "oversized", "invalid"):
        with Demo(binary, worker) as demo:
            demo.fixture.mode = mode
            _, job = demo.submit(mode)
            result = demo.wait(job["jobId"])
            assert result["state"] == "failed", (mode, result)
            assert demo.fixture.count() == 1, (mode, result, demo.fixture.requests)
            no_secrets(demo)
            outcomes[mode] = result["error"]
    with Demo(binary, worker) as demo:
        demo.fixture.subject = "工" * 500
        assert successful(demo, "subject-limit")["result"]["summary"].endswith("工" * 500)
        demo.fixture.subject += "工"
        _, job = demo.submit("subject-over-limit")
        result = demo.wait(job["jobId"])
        assert result["state"] == "failed" and result["error"] == "INVALID_ADAPTER_RESULT"
    return dict(cases=["C04", "C05"], variants=outcomes, subjectCharacters=500)


CASES = (healthy_identity, admission_boundaries, capacity_retention, restart_disconnect,
         expiry_timeout, hostile_plugin, adapter_boundaries, projection_and_message_limit)


def main():
    if not __debug__:
        raise RuntimeError("acceptance assertions must be enabled")
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(130))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=Path(os.environ.get("TYSEL_GATE_BIN_DIR", REPO / "target/debug")))
    parser.add_argument("--output", type=Path, default=Path("agent-triage-report.json"))
    parser.add_argument("--case", choices=[case.__name__ for case in CASES])
    args = parser.parse_args()
    report = dict(schemaVersion=1, stage="P1", status="running", cases=[])
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
