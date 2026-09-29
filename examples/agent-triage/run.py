#!/usr/bin/env python3
"""Local supervisor: two Tysel processes, a fake data source and bounded dispatch."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import AbstractContextManager
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import queue
import shutil
import signal
import sqlite3
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
import uuid

EXAMPLE = Path(__file__).resolve().parent
REPO = EXAMPLE.parents[1]
FAKE_SECRET = "triage-fake-provider-credential"
PRIVATE_FIELD = "triage-private-notes-must-not-leak"
DEFAULT_LIMITS = dict(callMs=5000, jobMs=120000, perCustomer=4, active=8, retained=1000)


def digest(paths):
    value = hashlib.sha256()
    for path in sorted(paths):
        value.update(path.name.encode() + b"\0" + path.read_bytes() + b"\0")
    return value.hexdigest()


def http(origin, path, method="GET", body=None, token=None, key=None, timeout=45):
    data = None if body is None else json.dumps(body).encode()
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    if key:
        headers["Idempotency-Key"] = key
    request = urllib.request.Request(origin + path, data=data, headers=headers, method=method)
    try:
        response = urllib.request.urlopen(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.loads(response.read())


class Fixture:
    """Independent observations; never trusts a plugin-supplied customer."""
    def __init__(self):
        self.lock = threading.Lock()
        self.requests = []
        self.plugin_requests = []
        self.plugin_headers = []
        self.plugin_responses = []
        self.plugin_reply = None
        self.adapter_reply = None
        self.chunked = False
        self.body_delay = 0
        self.plugin_origin = None
        self.gates = {}
        self.arrived = threading.Event()
        self.release = threading.Event()
        self.release.set()
        self.mode = "normal"
        self.subject = "Checkout unavailable"
        self.customers = [dict(id="customer-" + name, token="demo-" + name, tickets=[
            dict(id=f"{name}-{i:02}", priority=3 if i == 1 else 0) for i in range(16)
        ]) for name in ("a", "b")]
        self.records = {ticket["id"]: customer["id"] for customer in self.customers for ticket in customer["tickets"]}
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reply(self, status, body):
                self.protocol_version = "HTTP/1.1"
                self.send_response(status)
                self.send_header("Connection", "close")
                self.send_header("Content-Type", "application/json")
                self.send_header("Transfer-Encoding" if fixture.chunked else "Content-Length",
                                 "chunked" if fixture.chunked else str(len(body)))
                self.end_headers()
                try:
                    for at in range(0, len(body), 512):
                        if at and fixture.body_delay:
                            time.sleep(fixture.body_delay)
                        chunk = body[at:at + 512]
                        self.wfile.write((f"{len(chunk):x}\r\n".encode() + chunk + b"\r\n")
                                         if fixture.chunked else chunk)
                        self.wfile.flush()
                    if fixture.chunked:
                        self.wfile.write(b"0\r\n\r\n")
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def do_GET(self):
                if self.path.startswith("/gates/"):
                    name = self.path.removeprefix("/gates/")
                    gate = fixture.gates.get(name)
                    if gate:
                        gate[0].set()
                        gate[1].wait(15)
                    self.send_response(200)
                    self.send_header("Content-Length", "2")
                    self.end_headers()
                    try:
                        self.wfile.write(b"{}")
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                    return
                identifier = self.path.removeprefix("/tickets/")
                customer = self.headers.get("X-Customer-Id")
                valid = (self.path.startswith("/tickets/") and
                         self.headers.get("Authorization") == "Bearer " + FAKE_SECRET and
                         fixture.records.get(identifier) == customer)
                with fixture.lock:
                    fixture.requests.append(dict(ticketId=identifier, customerId=customer, authorized=valid, at=time.monotonic()))
                    attempt_number = len(fixture.requests)
                    mode = fixture.mode
                    subject = fixture.subject
                fixture.arrived.set()
                if not fixture.release.wait(10):
                    self.send_error(504)
                    return
                if mode == "redirect":
                    self.send_response(302)
                    self.send_header("Location", "/should-not-follow")
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                if mode in ("unavailable", "reject") or (mode in ("transient", "rate-limit") and attempt_number == 1):
                    self.send_response(403 if mode == "reject" else 429 if mode == "rate-limit" else 503)
                    self.send_header("Content-Length", "2")
                    self.end_headers()
                    try:
                        self.wfile.write(b"{}")
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                    return
                value = dict(ticketId=identifier, subject=subject, privateNotes=PRIVATE_FIELD)
                if mode == "oversized":
                    value["padding"] = "x" * 17000
                if mode == "invalid":
                    value["subject"] = 42
                data = json.dumps(value if valid else {"error": "forbidden"}).encode()
                if mode == "at-limit":
                    data += b" " * (16384 - len(data))
                status = 200 if valid else 403
                if fixture.adapter_reply:
                    status, data = fixture.adapter_reply(identifier, status, data)
                self.reply(status, data)

            def do_POST(self):
                data = self.rfile.read(int(self.headers.get("Content-Length", "0")))
                envelope = json.loads(data)
                with fixture.lock:
                    fixture.plugin_requests.append(envelope)
                    fixture.plugin_headers.append(dict(self.headers))
                request = urllib.request.Request(fixture.plugin_origin + self.path, data=data,
                                                 headers={"Content-Type": "application/json"})
                try:
                    response = urllib.request.urlopen(request, timeout=5)
                except urllib.error.HTTPError as error:
                    response = error
                except OSError:
                    self.send_error(503)
                    return
                with response:
                    body, status = response.read(), response.status
                if fixture.plugin_reply:
                    status, body = fixture.plugin_reply(envelope, status, body)
                with fixture.lock:
                    fixture.plugin_responses.append(body.decode("utf-8", errors="replace"))
                self.reply(status, body)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        # server_close joins all request handlers after bounded gates are released.
        self.server.daemon_threads = False
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.origin = f"http://127.0.0.1:{self.server.server_port}"

    def gate(self, name):
        gate = (threading.Event(), threading.Event())
        self.gates[name] = gate
        return gate

    def count(self):
        with self.lock:
            return len(self.requests)

    def close(self):
        self.release.set()
        for _, released in self.gates.values():
            released.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)
        if self.thread.is_alive():
            raise RuntimeError("fixture cleanup failed")


class Child:
    def __init__(self, binary, manifest, worker, token):
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("TYSEL_", "OTEL_", "OPENAI_", "TRIAGE_"))}
        env.update(TYSEL_WORKER=str(worker), TRIAGE_FIXTURE_TOKEN=token,
                   OTEL_SDK_DISABLED="true")
        self.logs = []
        self.process = subprocess.Popen([str(binary), "run", "--manifest", str(manifest)],
                                        cwd=manifest.parent, env=env, text=True,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        ready = queue.Queue()

        def drain(stream):
            for line in stream:
                self.logs.append(line)
                if line.startswith("tysel listen "):
                    ready.put("http://" + line.strip().removeprefix("tysel listen "))
            stream.close()

        self.readers = [threading.Thread(target=drain, args=(stream,), daemon=True)
                        for stream in (self.process.stdout, self.process.stderr)]
        for reader in self.readers:
            reader.start()
        try:
            self.origin = ready.get(timeout=15)
        except queue.Empty:
            self.close()
            raise RuntimeError("service readiness failed: " + "".join(self.logs)[-4000:])

    def close(self, crash=False):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGKILL if crash else signal.SIGTERM)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        for reader in self.readers:
            reader.join(timeout=5)
            if reader.is_alive():
                raise RuntimeError("child pipe cleanup failed")


class Demo(AbstractContextManager):
    def __init__(self, binary, worker, *, limits=None, plugin_source=None, dispatch=True, caller_transform=None, request_timeout_ms=None, plugin_proxy=False, plugin_transform=None):
        self.binary, self.worker = Path(binary).resolve(), Path(worker).resolve()
        self.temp = tempfile.TemporaryDirectory(prefix="tysel-triage-")
        self.root = Path(self.temp.name)
        self.fixture = None
        self.plugin = None
        self.caller = None
        self.history = []
        self.driver = None
        self.driver_stop = threading.Event()
        self.driver_errors = []
        self.dispatch_observations = []
        try:
            self.fixture = Fixture()
            plugin_dir = self.root / "plugin"
            shutil.copytree(REPO / "examples/isolated-plugin/src", plugin_dir / "src")
            shutil.copy(EXAMPLE / "plugin.toml", plugin_dir / "tysel.toml")
            if plugin_source is not None:
                (plugin_dir / "src/index.ts").write_text(plugin_source)
            if plugin_transform:
                plugin_transform(self, plugin_dir)
            self.plugin = Child(self.binary, plugin_dir / "tysel.toml", self.worker, "")
            self.caller_dir = self.root / "caller"
            shutil.copytree(EXAMPLE / "src", self.caller_dir / "src")
            shutil.copy(EXAMPLE / "tysel.toml", self.caller_dir / "tysel.toml")
            if request_timeout_ms is not None:
                manifest = self.caller_dir / "tysel.toml"
                manifest.write_text(manifest.read_text().replace("request_timeout_ms = 40000", f"request_timeout_ms = {request_timeout_ms}"))
            if caller_transform:
                caller_transform(self)
            # This disposable namespace pins its complete caller source before any admission.
            self.pinned_source = self.root / "pinned-caller"
            shutil.copytree(self.caller_dir / "src", self.pinned_source)
            self.fixture.plugin_origin = self.plugin.origin
            (self.caller_dir / "config").mkdir()
            self.config = dict(
                bootId=str(uuid.uuid4()), dispatchToken=str(uuid.uuid4()),
                pluginOrigin=self.fixture.origin if plugin_proxy else self.plugin.origin, adapterOrigin=self.fixture.origin,
                pluginDigest=digest(list((plugin_dir / "src").glob("*.ts")) + [plugin_dir / "tysel.toml"]),
                callerDigest=digest(list((self.caller_dir / "src").glob("*.ts"))),
                adapterId=hashlib.sha256((self.fixture.origin + ":fixture-v1").encode()).hexdigest(),
                limits={**DEFAULT_LIMITS, **(limits or {})}, customers=self.fixture.customers,
            )
            self.start_caller(dispatch)
        except BaseException:
            self.close()
            raise

    def start_caller(self, dispatch=True):
        # Recover original outbox submissions under their original full bundle.
        shutil.rmtree(self.caller_dir / "src")
        shutil.copytree(self.pinned_source, self.caller_dir / "src")
        self.config["bootId"] = str(uuid.uuid4())
        (self.caller_dir / "config/service.json").write_text(json.dumps(self.config))
        self.caller = Child(self.binary, self.caller_dir / "tysel.toml", self.worker, FAKE_SECRET)
        status, ready = http(self.caller.origin, "/health")
        if status != 200 or ready != {"ready": True}:
            raise RuntimeError(f"caller initialization failed: {status} {ready}")
        if dispatch:
            self.start_driver()

    def start_driver(self):
        self.driver_stop.clear()
        self.driver_errors.clear()
        origin, token = self.caller.origin, self.config["dispatchToken"]
        wake = threading.Event()

        def wake_completed(future):
            try:
                status, result = future.result()
            except Exception:
                return  # The driver observes and reports the original exception.
            if status == 200 and isinstance(result, dict) and result.get("status") == "completed":
                wake.set()

        def run():
            pending = {}
            retry_after = {}
            active_until = 0.0
            try:
                with ThreadPoolExecutor(max_workers=8) as pool:
                    while not self.driver_stop.is_set():
                        wake.clear()
                        for job_id, future in list(pending.items()):
                            if future.done():
                                status, result = future.result()
                                self.dispatch_observations.append(status)
                                del self.dispatch_observations[:-64]
                                if status not in (200, 503):
                                    raise RuntimeError(f"dispatch failed: {status} {result}")
                                if status == 200 and isinstance(result, dict) and result.get("status") == "completed":
                                    active_until = time.monotonic() + .25
                                else:
                                    retry_after[job_id] = time.monotonic() + .25
                                del pending[job_id]
                        now = time.monotonic()
                        retry_after = {job_id: deadline for job_id, deadline in retry_after.items() if deadline > now}
                        status, jobs = http(origin, "/internal/pending", token=token)
                        if status == 503:
                            self.driver_stop.wait(0.25)
                            continue
                        if status != 200:
                            raise RuntimeError(f"pending lookup failed: {status} {jobs}")
                        for job in jobs:
                            if job["id"] not in pending and len(pending) < 8 and time.monotonic() >= retry_after.get(job["id"], 0.0):
                                future = pool.submit(http, origin, "/internal/dispatch", "POST",
                                                     {"jobId": job["id"]}, token)
                                pending[job["id"]] = future
                                future.add_done_callback(wake_completed)
                        wake.wait(.05 if time.monotonic() < active_until else .25)
            except Exception as error:
                if not self.driver_stop.is_set():
                    self.driver_errors.append(str(error))

        self.driver = threading.Thread(target=run, daemon=True)
        self.driver.start()

    def stop_caller(self, crash=False):
        self.driver_stop.set()
        if self.caller:
            self.caller.close(crash)
            self.history.extend(self.caller.logs)
            self.caller = None
        if self.driver:
            self.driver.join(timeout=30)
            if self.driver.is_alive():
                raise RuntimeError("dispatcher cleanup failed")
            self.driver = None

    def submit(self, key, ids=None, token="demo-a"):
        return http(self.caller.origin, "/jobs", "POST",
                    {"protocolVersion": 1, "ticketIds": ["a-00", "a-01"] if ids is None else ids}, token, key)

    def wait(self, job_id, token="demo-a", timeout=60):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.driver_errors:
                raise RuntimeError("; ".join(self.driver_errors))
            status, job = http(self.caller.origin, "/jobs/" + job_id, token=token)
            if status != 200:
                raise RuntimeError(f"status lookup failed: {status} {job}")
            if job["state"] in ("succeeded", "failed", "expired"):
                return job
            time.sleep(0.025)
        raise TimeoutError("job did not reach a terminal state")

    def audit(self):
        with sqlite3.connect(self.caller_dir / "data/jobs.db") as db:
            db.row_factory = sqlite3.Row
            return [dict(row) for row in db.execute("SELECT * FROM triage_audit ORDER BY at_ms,job_id,event_key")]

    def close(self):
        failures = []
        if self.fixture:
            self.fixture.release.set()
            for _, released in self.fixture.gates.values():
                released.set()
        for cleanup in (self.stop_caller,
                        lambda: self.plugin.close() if self.plugin else None,
                        lambda: self.fixture.close() if self.fixture else None,
                        self.temp.cleanup):
            try:
                cleanup()
            except Exception as error:
                failures.append(str(error))
        if failures:
            raise RuntimeError("cleanup failed: " + "; ".join(failures))

    def __exit__(self, *_):
        self.close()


def main():
    def terminate(*_):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, terminate)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/debug")
    parser.add_argument("--serve", action="store_true", help="keep the local demo running")
    parser.add_argument("--audit", action="store_true", help="include bounded metadata audit records in the demo result")
    args = parser.parse_args()
    with Demo(args.bin_dir / "tysel", args.bin_dir / "tysel-worker") as demo:
        if args.serve:
            print(json.dumps({"url": demo.caller.origin, "developmentTokens": ["demo-a", "demo-b"]}), flush=True)
            try:
                while True:
                    if demo.driver_errors or demo.caller.process.poll() is not None or demo.plugin.process.poll() is not None:
                        raise RuntimeError("demo process or dispatcher failed")
                    time.sleep(0.25)
            except KeyboardInterrupt:
                pass
        else:
            status, admitted = demo.submit("demo-first")
            if status != 202:
                raise RuntimeError(f"admission failed: {status} {admitted}")
            job = demo.wait(admitted["jobId"])
            if job["state"] != "succeeded" or demo.fixture.count() != 1:
                raise RuntimeError(f"demo failed: {job}")
            result = {"job": job, "dataSourceReads": demo.fixture.count()}
            if args.audit:
                result["audit"] = demo.audit()
            print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
