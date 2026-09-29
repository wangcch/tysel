#!/usr/bin/env python3
"""P5 standalone-artifact measurements; see the frozen protocol before running."""
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import select
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
EXAMPLE = REPO / "examples/agent-triage"
sys.path.insert(0, str(EXAMPLE))
import deploy
import package
spec = importlib.util.spec_from_file_location("triage_fixture", EXAMPLE / "run.py")
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def save(path, value):
    # Encode before touching the last complete snapshot. A same-directory rename
    # publishes one complete JSON document; this is not a power-loss guarantee.
    payload = json.dumps(value, indent=2, allow_nan=False) + "\n"
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=path.parent,
                                         prefix="." + path.name + ".", suffix=".tmp",
                                         delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(payload)
            stream.flush()
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            cleanup_all(("temporary evidence file " + str(temporary),
                         lambda: temporary.unlink(missing_ok=True)))


def cleanup_all(*actions):
    """Attempt every teardown and keep the in-flight failure as the cause."""
    primary = sys.exc_info()[1]
    failures = []
    for label, action in actions:
        try:
            action()
        except BaseException as error:
            failures.append((label, error, traceback.format_exc()))
    if failures:
        detail = "\n".join(label + ": " + trace for label, _, trace in failures)
        raise RuntimeError("measurement cleanup failed:\n" + detail) from (primary or failures[0][1])


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def query(state, sql, runtime=False):
    with sqlite3.connect(state / "caller/data" / ("durable-events.db" if runtime else "jobs.db")) as db:
        db.row_factory = sqlite3.Row
        return [dict(row) for row in db.execute(sql)]


def wait_for(predicate, timeout=65, poll=.005):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        value = predicate()
        if value:
            return value
        time.sleep(poll)
    raise AssertionError("bounded wait expired")


class Support:
    def __init__(self, executable, timeout=5.0):
        if not 0 < timeout < float("inf"):
            raise ValueError("support timeout must be finite and positive")
        self.timeout = timeout
        self.lock = threading.Lock()
        self.close_lock = threading.Lock()
        self.cleanup = []
        self.closed = False
        self.closing = False
        self.failure = None
        self.process = subprocess.Popen([str(executable)], stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, bufsize=0)
        try:
            os.set_blocking(self.process.stdin.fileno(), False)
            os.set_blocking(self.process.stdout.fileno(), False)
        except BaseException as original:
            cleanup_errors = self._force_stop() + self._close_pipes()
            if cleanup_errors:
                raise RuntimeError("benchmark support initialization cleanup failed: " +
                                   "; ".join(str(error) for error in cleanup_errors)) from original
            raise

    def _force_stop(self):
        errors = []
        if self.process.poll() is None:
            try:
                self.process.terminate()
            except BaseException as error:
                errors.append(error)
            try:
                self.process.wait(timeout=1)
            except subprocess.TimeoutExpired:
                pass
            except BaseException as error:
                errors.append(error)
        if self.process.poll() is None:
            try:
                self.process.kill()
            except BaseException as error:
                errors.append(error)
            try:
                self.process.wait(timeout=1)
            except BaseException as error:
                errors.append(error)
        return errors

    def _close_pipes(self):
        errors = []
        for stream in (self.process.stdin, self.process.stdout):
            try:
                stream.close()
            except BaseException as error:
                errors.append(error)
        return errors

    @staticmethod
    def remaining(deadline):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("benchmark support request timed out")
        return remaining

    def call(self, **request):
        deadline = time.monotonic() + self.timeout
        if not self.lock.acquire(timeout=self.remaining(deadline)):
            raise TimeoutError("benchmark support request timed out waiting for the lock")
        try:
            if self.closed or self.closing:
                raise RuntimeError("benchmark support is closed")
            if self.failure is not None:
                raise RuntimeError("benchmark support is unavailable after a failed request") from self.failure
            outgoing = (json.dumps(request) + "\n").encode()
            sent = 0
            descriptor = self.process.stdin.fileno()
            while sent < len(outgoing):
                _, ready, _ = select.select([], [descriptor], [], self.remaining(deadline))
                if not ready:
                    raise TimeoutError("benchmark support request timed out writing input")
                try:
                    written = os.write(descriptor, outgoing[sent:])
                except (BlockingIOError, InterruptedError):
                    continue
                if not written:
                    raise RuntimeError("benchmark support input pipe closed")
                sent += written
            incoming = bytearray()
            descriptor = self.process.stdout.fileno()
            while True:
                ready, _, _ = select.select([descriptor], [], [], self.remaining(deadline))
                if not ready:
                    raise TimeoutError("benchmark support request timed out waiting for a complete reply")
                try:
                    chunk = os.read(descriptor, 65536)
                except (BlockingIOError, InterruptedError):
                    continue
                if not chunk:
                    raise RuntimeError("benchmark support process ended before a complete reply")
                incoming.extend(chunk)
                if b"\n" in chunk:
                    line, extra = incoming.split(b"\n", 1)
                    if extra:
                        raise RuntimeError("benchmark support emitted extra reply data")
                    return json.loads(line)
        except BaseException as error:
            self.failure = error
            raise
        finally:
            self.lock.release()

    def close(self, timeout=5.0):
        if not 0 < timeout < float("inf"):
            raise ValueError("support close timeout must be finite and positive")
        with self.close_lock:
            if self.closed:
                return
            self.closing = True
            deadline = time.monotonic() + timeout
            acquired = self.lock.acquire(timeout=max(0, deadline - time.monotonic()))
            failure = None
            cleanup_errors = []
            try:
                if not acquired:
                    raise TimeoutError("benchmark support shutdown timed out waiting for an active request")
                self.process.stdin.close()
                try:
                    self.process.wait(timeout=max(0, deadline - time.monotonic()))
                except subprocess.TimeoutExpired as error:
                    raise TimeoutError("benchmark support shutdown timed out after EOF") from error
                if self.process.returncode != 0:
                    raise RuntimeError(f"benchmark support process exited with status {self.process.returncode}")
            except BaseException as error:
                failure = error
            finally:
                try:
                    # Even a close/wait exception must attempt to stop and reap
                    # the helper. A forced stop cannot turn the run into a pass.
                    cleanup_errors.extend(self._force_stop())
                finally:
                    cleanup_errors.extend(self._close_pipes())
                    self.closed = (self.process.poll() is not None and
                                   self.process.stdin.closed and self.process.stdout.closed)
                    if acquired:
                        self.lock.release()
            if cleanup_errors:
                raise RuntimeError("benchmark support cleanup failed: " +
                                   "; ".join(str(error) for error in cleanup_errors)) from (failure or cleanup_errors[0])
            if failure is not None:
                raise failure


class Memory:
    def __init__(self, support, roots, interval):
        self.support, self.roots, self.interval = support, roots, interval
        self.samples, self.errors = [], []
        self.error = None
        self.stop = threading.Event()
        self.started = time.monotonic()
        self.sample()
        self.thread = threading.Thread(target=self.run)
        self.thread.start()

    def sample(self):
        value = self.support.call(op="memory", roots=self.roots)
        if value["kind"] != "pss":
            raise RuntimeError("memory sampler requires Linux PSS: " + str(value))
        value["atMs"] = (time.monotonic() - self.started) * 1000
        self.samples.append(value)

    def run(self):
        try:
            while not self.stop.wait(self.interval):
                self.sample()
        except Exception as error:
            self.errors.append(str(error))
            self.error = error

    def close(self):
        self.stop.set()
        # A sample already in progress retains its complete request deadline.
        self.thread.join(timeout=self.support.timeout + 1)
        if self.thread.is_alive():
            try:
                self.support.close(timeout=1)
            finally:
                self.thread.join(timeout=2)
            raise RuntimeError("memory sampling thread did not finish within its request deadline")
        if self.errors:
            raise RuntimeError("memory sampling failed: " + "; ".join(self.errors)) from self.error
        self.sample()


def prepare(binary_dir, output):
    release = output / "lookup"
    metadata = {"lookup": package.package(binary_dir, release)}
    for variant in ("direct", "snapshot"):
        with tempfile.TemporaryDirectory(prefix="triage-bench-build-") as temp:
            root = Path(temp)
            (root / "src").mkdir()
            (root / "src/index.ts").write_text((HERE / "index.ts").read_text().replace(
                "../../examples/isolated-plugin/src/triage.js", "./triage.js"))
            shutil.copy(REPO / "examples/isolated-plugin/src/triage.ts", root / "src/triage.ts")
            manifest = (EXAMPLE / "plugin.toml").read_text()
            manifest = manifest.replace('name = "agent-triage-plugin"', 'name = "agent-cost-' + variant + '"')
            if variant == "direct":
                manifest = manifest.replace('profile = "isolated"', 'profile = "service"')
            (root / "tysel.toml").write_text(manifest)
            destination = output / variant
            command = [str(binary_dir / "tysel"), "-C", str(root), "build", "--stub",
                       str(binary_dir / "tysel-service"), "--output", str(destination)]
            result = subprocess.run(command, capture_output=True, text=True, timeout=90)
            assert result.returncode == 0, result.stdout + result.stderr
            metadata[variant] = dict(sha256=sha(destination), bytes=destination.stat().st_size,
                                     manifest=manifest, buildOutput=result.stdout,
                                     sources={p.name: sha(p) for p in (root / "src").iterdir()})
    return metadata


def fixture(subject_chars, delay_ms):
    value = base.Fixture()
    value.customers = [dict(id=f"bench-c{c}", token=f"bench-token-{c}", tickets=[
        dict(id=f"t{c}-{i:02}", priority=3 if i == 0 else 0) for i in range(16)
    ]) for c in range(4)]
    value.records = {ticket["id"]: customer["id"] for customer in value.customers for ticket in customer["tickets"]}
    value.subject = "s" * subject_chars
    value.delays = []
    def response(identifier, status, data):
        started = time.monotonic()
        if delay_ms:
            time.sleep(delay_ms / 1000)
        ended = time.monotonic()
        with value.lock:
            value.delays.append(dict(ticketId=identifier, start=started, end=ended,
                                     actualSleepMs=(ended - started) * 1000 if delay_ms else 0,
                                     responseBytes=len(data)))
        return status, data
    value.adapter_reply = response
    return value


def settings(value):
    return dict(adapterOrigin=value.origin, adapterId="p5-independent-counted-fixture-v1", customers=value.customers)


class Micro:
    def __init__(self, releases, variant, root):
        self.child = deploy.Process(releases / variant, root, releases / "lookup/tysel-worker")
        self.origin = self.child.origin

    @property
    def roots(self):
        return [self.child.process.pid]

    def close(self):
        self.child.close()


def roots(app):
    return [app.caller.process.pid, app.plugin.process.pid] if isinstance(app, deploy.Deployment) else app.roots


def stop_app(app, support, crash=False):
    try:
        pids = support.call(op="memory", roots=roots(app))["pids"]
    finally:
        # An unavailable observer must not prevent stopping the actual workload.
        cleanup_all(("application", lambda: app.close(crash=crash)
                     if isinstance(app, deploy.Deployment) else app.close()))
    def stopped():
        for pid in pids:
            try:
                state = Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()[0]
                if state != "Z":
                    return False
            except FileNotFoundError:
                pass
        return True
    wait_for(stopped, timeout=5)
    support.cleanup.append(dict(pids=pids, crash=crash, liveProcessesRemaining=0))


@contextmanager
def application(releases, variant, root, provider, support):
    root.mkdir()
    if variant == "lookup":
        deploy.initialize(releases / "lookup", root, settings(provider))
        app = deploy.Deployment(releases / "lookup", root, base.FAKE_SECRET)
    else:
        app = Micro(releases, variant, root)
    try:
        yield app
    finally:
        cleanup_all(("application", lambda: stop_app(app, support)))


def payload(provider, size, client):
    customer = provider.customers[client]
    tickets = customer["tickets"][:size["tickets"]]
    detail = dict(ticketId=tickets[0]["id"], subject=provider.subject)
    snapshot = dict(customerId=customer["id"], tickets=tickets, detail=detail)
    expected = dict(customerId=customer["id"], summary=f"Prioritize {tickets[0]['id']}: {provider.subject}")
    return customer, tickets, snapshot, expected


def request(app, provider, variant, size, client, key, poll):
    customer, tickets, snapshot, expected = payload(provider, size, client)
    started = time.monotonic()
    if variant == "lookup":
        body = dict(protocolVersion=1, ticketIds=[t["id"] for t in tickets])
        status, admitted = deploy.http(app.origin, "/jobs", "POST", body, customer["token"], key)
        if status != 202:
            raise AssertionError((status, admitted))
        admission_ms = (time.monotonic() - started) * 1000
        def terminal():
            if app.errors:
                raise AssertionError(app.errors)
            status, job = deploy.http(app.origin, "/jobs/" + admitted["jobId"], token=customer["token"])
            if status != 200:
                raise AssertionError((status, job))
            return job if job["state"] in ("succeeded", "failed", "expired") else None
        final = wait_for(terminal, poll=poll)
        ended = time.monotonic()
        if final["state"] != "succeeded" or final["result"] != expected:
            raise AssertionError(final)
        with provider.lock:
            attempts = [r.copy() for r in provider.requests if r["ticketId"] == tickets[0]["id"] and started <= r["at"] <= ended]
            delays = [r.copy() for r in provider.delays if r["ticketId"] == tickets[0]["id"] and started <= r["start"] <= ended]
        if len(attempts) != 1 or len(delays) != 1 or not attempts[0]["authorized"]:
            raise AssertionError((attempts, delays))
        result = dict(jobId=admitted["jobId"], admissionMs=admission_ms,
                      adapterAttempts=attempts, adapter=delays[0], actualSleepMs=delays[0]["actualSleepMs"])
    else:
        body = dict(protocolVersion=1, jobId="bench-job-000000000000000000000000", stepId="2",
                    attemptId="bench-attempt-000000000000000000000", payload=snapshot)
        status, final = deploy.http(app.origin, "/triage/v1", "POST", body)
        ended = time.monotonic()
        if status != 200 or final != {**body, "payload": {"kind": "done", **expected}}:
            raise AssertionError((status, final))
        result = dict(actualSleepMs=0, adapterAttempts=[])
    elapsed = (ended - started) * 1000
    result.update(e2eMs=elapsed, excludingAdapterSleepMs=elapsed - result["actualSleepMs"],
                  requestBytes=len(json.dumps(body).encode()), responseJsonBytes=len(json.dumps(final, separators=(",", ":")).encode()),
                  resultBytes=len(json.dumps(expected, separators=(",", ":")).encode()), client=client)
    if variant != "lookup":
        result["stagesMs"] = dict(httpTransform=elapsed)
    return result


def attach_audit(app, samples):
    wait_for(lambda: not query(app.state, "SELECT id FROM triage_jobs WHERE delivery='pending'"))
    rows = query(app.state, "SELECT * FROM triage_audit ORDER BY at_ms,event_key")
    for sample in samples:
        events = [r for r in rows if r["job_id"] == sample["jobId"]]
        by_key = {r["event_key"]: r for r in events}
        sample["audit"] = events
        assert len(events) == 8 and not any(r["decision"] == "deny" for r in events), events
        stages = {}
        gaps = 0
        previous = by_key["admission"]["at_ms"]
        for step in ("select", "read", "summarize"):
            reserve = by_key[f"reserve:{step}:1"]["at_ms"]
            accept = by_key[f"outcome:{step}:1"]["at_ms"]
            assert reserve >= previous and accept >= reserve, "audit clock moved backwards"
            if step == "select":
                stages["queueToSelect"] = reserve - previous
            else:
                gaps += reserve - previous
            stages[step] = accept - reserve
            previous = accept
        stages["betweenStages"] = gaps
        stages["terminalCommit"] = by_key["terminal"]["at_ms"] - previous
        assert stages["terminalCommit"] >= 0
        stages["persistedTotal"] = by_key["terminal"]["at_ms"] - by_key["admission"]["at_ms"]
        sample["stagesMs"] = stages


def warm_round(releases, support, protocol, root, variant, size, concurrency, delay, round_index):
    provider = fixture(size["subjectChars"], delay)
    try:
        with application(releases, variant, root, provider, support) as app:
            for i in range(protocol["warmupRequests"]):
                request(app, provider, variant, size, 0, f"warm-{i}", protocol["statusPollMs"] / 1000)
            sampler = Memory(support, roots(app), protocol["memoryIntervalMs"] / 1000)
            def client(c):
                return [request(app, provider, variant, size, c, f"job-{c}-{i}", protocol["statusPollMs"] / 1000)
                        for i in range(protocol["requestsPerRound"] // concurrency)]
            try:
                with ThreadPoolExecutor(max_workers=concurrency) as pool:
                    started = time.monotonic()
                    samples = [sample for group in pool.map(client, range(concurrency)) for sample in group]
                    elapsed = time.monotonic() - started
            finally:
                cleanup_all(("memory sampler", sampler.close))
            expected_processes = {"direct": 1, "snapshot": 2, "lookup": 3}[variant]
            assert all(len(s["pids"]) >= expected_processes for s in sampler.samples), sampler.samples
            if variant == "lookup":
                attach_audit(app, samples)
            expected_reads = protocol["requestsPerRound"] + protocol["warmupRequests"] if variant == "lookup" else 0
            assert provider.count() == expected_reads
            assert all(r["authorized"] for r in provider.requests)
            assert len(samples) == protocol["requestsPerRound"]
            return dict(variant=variant, size=size["name"], concurrency=concurrency, adapterDelayMs=delay,
                        round=round_index, samples=samples, durationMs=elapsed * 1000,
                        jobsPerSec=len(samples) / elapsed, memory=sampler.samples,
                        idlePssKiB=sampler.samples[0]["valueKiB"],
                        peakPssKiB=max(s["valueKiB"] for s in sampler.samples),
                        errors=0, unexpectedDenials=0, physicalReadsIncludingWarmup=provider.count())
    finally:
        cleanup_all(("fixture", provider.close))


def cold(releases, protocol, root, variant, size, index, support):
    provider = fixture(size["subjectChars"], protocol["coldAdapterDelayMs"])
    app = None
    try:
        root.mkdir()
        if variant == "lookup":
            deploy.initialize(releases / "lookup", root, settings(provider))
        started = time.monotonic()
        app = (deploy.Deployment(releases / "lookup", root, base.FAKE_SECRET) if variant == "lookup"
               else Micro(releases, variant, root))
        startup_ms = (time.monotonic() - started) * 1000
        sample = request(app, provider, variant, size, 0, "cold", protocol["statusPollMs"] / 1000)
        sample.update(variant=variant, sample=index, startupMs=startup_ms, coldTotalMs=(time.monotonic() - started) * 1000)
        if variant == "lookup":
            attach_audit(app, [sample])
        return sample
    finally:
        cleanup_all(("application", lambda: stop_app(app, support) if app else None),
                    ("fixture", provider.close))


def recovery(releases, root, index, support):
    provider = fixture(64, 0)
    app = None
    try:
        deploy.initialize(releases / "lookup", root, settings(provider))
        app = deploy.Deployment(releases / "lookup", root, base.FAKE_SECRET)
        provider.release.clear()
        status, job = deploy.http(app.origin, "/jobs", "POST", dict(protocolVersion=1, ticketIds=["t0-00"]), "bench-token-0", "recover")
        assert status == 202 and provider.arrived.wait(5)
        slots = query(root, "SELECT * FROM triage_attempts WHERE step='read'")
        execution = query(root, "SELECT state,lease_until_ms FROM durable_executions", True)
        assert len(slots) == 1 and slots[0]["outcome_json"] is None and provider.count() == 1
        assert execution[0]["state"] == "running"
        crash_at = time.monotonic()
        remaining_lease = execution[0]["lease_until_ms"] - time.time() * 1000
        stop_app(app, support, crash=True)
        app = None
        provider.release.set()
        app = deploy.Deployment(releases / "lookup", root, base.FAKE_SECRET)
        def finished():
            assert not app.errors, app.errors
            status, final = deploy.http(app.origin, "/jobs/" + job["jobId"], token="bench-token-0")
            assert status == 200
            return final if final["state"] in ("succeeded", "failed", "expired") else None
        final = wait_for(finished)
        elapsed = (time.monotonic() - crash_at) * 1000
        assert final["state"] == "succeeded" and final["deadlineAt"] == job["deadlineAt"]
        assert final["result"] == dict(customerId="bench-c0", summary="Prioritize t0-00: " + "s" * 64)
        status, repeated = deploy.http(app.origin, "/jobs", "POST", dict(protocolVersion=1, ticketIds=["t0-00"]), "bench-token-0", "recover")
        assert status == 202 and repeated["jobId"] == job["jobId"]
        slots = query(root, "SELECT * FROM triage_attempts WHERE step='read'")
        assert len(slots) == provider.count() == 2
        assert all(r["authorized"] for r in provider.requests)
        assert json.loads(slots[0]["outcome_json"])["error"] == "UNKNOWN_OUTCOME"
        wait_for(lambda: not query(root, "SELECT id FROM triage_jobs WHERE delivery='pending'"))
        return dict(sample=index, recoveryMs=elapsed, remainingLeaseAtCrashMs=remaining_lease,
                    defaultLeaseMs=45000, originalDeadlinePreserved=True, physicalReads=provider.requests,
                    slots=slots, audit=query(root, "SELECT * FROM triage_audit"))
    finally:
        cleanup_all(("application", lambda: stop_app(app, support) if app else None),
                    ("fixture", provider.close))


def denials(releases, root, support):
    provider = fixture(64, 0)
    try:
        with application(releases, "lookup", root, provider, support) as app:
            results = []
            for path in ("/probe/fetch", "/probe/filesystem"):
                started = time.monotonic()
                status, reply = deploy.http(app.plugin.origin, path)
                assert status == 403 and reply["denied"]
                results.append(dict(path=path, status=status, denied=reply["denied"], elapsedMs=(time.monotonic()-started)*1000))
            assert provider.count() == 0
            return dict(expectedDenials=2, physicalReads=0, probes=results)
    finally:
        cleanup_all(("fixture", provider.close))


def analyze(support, protocol, data):
    groups = {}
    series = {}
    for row in data["warm"]:
        name = f"{row['variant']}/{row['size']}/c{row['concurrency']}/delay{row['adapterDelayMs']}"
        groups.setdefault(name, []).append(row)
    for name, rows in groups.items():
        samples = [s for row in rows for s in row["samples"]]
        for metric in ("e2eMs", "excludingAdapterSleepMs", "actualSleepMs"):
            series[name + "/" + metric] = [s[metric] for s in samples]
        for stage in samples[0]["stagesMs"]:
            series[name + "/stage/" + stage] = [s["stagesMs"][stage] for s in samples]
        for row in rows:
            series[name + f"/round{row['round']}/e2eMs"] = [s["e2eMs"] for s in row["samples"]]
    for variant in protocol["variants"]:
        series[variant + "/coldTotalMs"] = [s["coldTotalMs"] for s in data["cold"] if s["variant"] == variant]
        series[variant + "/coldStartupMs"] = [s["startupMs"] for s in data["cold"] if s["variant"] == variant]
    distributions = support.call(op="stats", series=series)
    checks = []
    def check(name, metric, actual, limit, minimum=False):
        checks.append(dict(cell=name, metric=metric, actual=actual, limit=limit,
                           relation=">=" if minimum else "<=", passed=actual >= limit if minimum else actual <= limit))
    for name, rows in groups.items():
        budget = protocol["budgets"][rows[0]["variant"]]
        check(name, "warmP95Ms", distributions[name + "/e2eMs"]["p95"], budget["warmP95Ms"])
        # Every round must meet the budget; an average cannot hide a failed round.
        for row in rows:
            check(name, f"round{row['round']}.p95Ms", distributions[name + f"/round{row['round']}/e2eMs"]["p95"], budget["warmP95Ms"])
            check(name, f"round{row['round']}.jobsPerSec", row["jobsPerSec"], budget["minJobsPerSec"][str(row["concurrency"])], True)
        check(name, "peakPssKiB", max(r["peakPssKiB"] for r in rows), budget["peakPssKiB"])
    for variant in protocol["variants"]:
        check(variant, "coldP95Ms", distributions[variant + "/coldTotalMs"]["p95"], protocol["budgets"][variant]["coldP95Ms"])
    check("lookup", "recoveryMaxMs", max(r["recoveryMs"] for r in data["recovery"]), protocol["budgets"]["recoveryMaxMs"])
    return dict(distributions=distributions, checks=checks, passed=all(c["passed"] for c in checks))


def require_assertions():
    # The complete workload and imported fixtures still use assertions. Reject
    # optimization before parsing inputs, starting processes or writing evidence.
    if not __debug__:
        raise RuntimeError("agent-triage benchmark requires assertions; do not use -O, -OO or PYTHONOPTIMIZE")


def main():
    require_assertions()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--protocol", type=Path, default=HERE / "protocol.json")
    args = parser.parse_args()
    if platform.system() != "Linux":
        raise RuntimeError("P5 protocol requires Linux PSS and isolation")
    binary_dir, output = args.bin_dir.resolve(), args.output.resolve()
    if output.exists():
        raise FileExistsError("choose a fresh output directory; never overwrite evidence")
    output.mkdir(parents=True)
    protocol = json.loads(args.protocol.read_text())
    if protocol["requestsPerRound"] % max(protocol["concurrency"]) != 0:
        raise ValueError("requests per round must divide evenly among clients")
    shutil.copy(args.protocol, output / "protocol.json")
    started = time.monotonic()
    data = dict(protocol=protocol, protocolSha256=sha(args.protocol), command=sys.argv,
                startedAtUnixMs=time.time_ns() // 1000000, startLoadAverage=list(os.getloadavg()),
                cold=[], warm=[], recovery=[], status="running")
    support, primary = None, None
    cleanup_errors = []
    try:
        support = Support(binary_dir / "tysel-bench-agent-support")
        data.update(
            sourceCommit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
            workspaceDirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=REPO)),
            system=support.call(op="system"), kernel=platform.release(),
            binaries={name: dict(sha256=sha(binary_dir/name), bytes=(binary_dir/name).stat().st_size)
                      for name in ("tysel", "tysel-service", "tysel-worker", "tysel-bench-agent-support")})
        source_paths = [p for folder in (HERE, EXAMPLE, REPO / "examples/isolated-plugin/src", REPO / "crates/tysel-bench-compare/src")
                        for p in folder.rglob("*") if p.is_file() and p.suffix in (".py", ".ts", ".toml", ".rs", ".json")]
        data["sources"] = {str(p.relative_to(REPO)): sha(p) for p in sorted(set(source_paths))}
        data["nativeSourceTreeSha256"] = hashlib.sha256(b"".join(
            str(p.relative_to(REPO)).encode() + b"\0" + p.read_bytes() + b"\0"
            for p in sorted(REPO.glob("crates/*/src/**/*.rs")))).hexdigest()
        data["cargoFiles"] = {name: sha(REPO / name) for name in ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml")}
        releases = output / "releases"
        releases.mkdir()
        data["artifacts"] = prepare(binary_dir, releases)
        save(output / "measurements.json", data)
        with tempfile.TemporaryDirectory(prefix="agent-p5-states-", dir=output) as temp:
            root = Path(temp)
            size = next(s for s in protocol["sizes"] if s["name"] == protocol["coldSize"])
            for i in range(protocol["coldSamples"]):
                variants = protocol["variants"][i % 3:] + protocol["variants"][:i % 3]
                for variant in variants:
                    data["cold"].append(cold(releases, protocol, root / f"cold-{i}-{variant}", variant, size, i, support))
                save(output / "measurements.json", data)
                print(f"cold sample {i+1}/{protocol['coldSamples']} complete", flush=True)
            for round_index in range(protocol["rounds"]):
                variants = protocol["variants"][round_index:] + protocol["variants"][:round_index]
                for variant in variants:
                    for size in protocol["sizes"]:
                        for concurrency in protocol["concurrency"]:
                            for delay in protocol["adapterDelayMs"] if variant == "lookup" else [0]:
                                name = f"r{round_index}-{variant}-{size['name']}-c{concurrency}-d{delay}"
                                data["warm"].append(warm_round(releases, support, protocol, root / name, variant, size, concurrency, delay, round_index))
                                save(output / "measurements.json", data)
                                print(name + " complete", flush=True)
            data["denials"] = denials(releases, root / "denials", support)
            for i in range(protocol["recoverySamples"]):
                data["recovery"].append(recovery(releases, root / f"recovery-{i}", i, support))
                save(output / "measurements.json", data)
                print(f"recovery {i+1}/{protocol['recoverySamples']} complete", flush=True)
        data["analysis"] = analyze(support, protocol, data)
        data["status"] = "passed" if data["analysis"]["passed"] else "budget_miss"
        data["cleanup"] = "all application processes stopped, request threads joined, temporary namespaces removed"
    except BaseException as error:
        primary = error
        data["status"] = "error"
        data["error"] = traceback.format_exc()
    finally:
        data["processCleanup"] = support.cleanup if support is not None else []
        # Saving the final state must be attempted even when teardown fails.
        # A failed final save leaves the last complete (usually running) snapshot
        # intact and reports every failure on stderr through the exception chain.
        for label, action in (
            ("benchmark support close", lambda: support.close() if support is not None else None),
            ("final timing metadata", lambda: data.update(endLoadAverage=list(os.getloadavg()),
                                                         durationSec=time.monotonic() - started)),
        ):
            try:
                action()
            except BaseException as error:
                cleanup_errors.append((error, label + ":\n" + traceback.format_exc()))
        if cleanup_errors:
            data["status"] = "error"
            data["cleanupErrors"] = [detail for _, detail in cleanup_errors]
            if primary is None:
                data["error"] = data["cleanupErrors"][0]
        try:
            save(output / "measurements.json", data)
        except BaseException as error:
            detail = "final evidence save failed at " + str(output / "measurements.json") + ":\n" + traceback.format_exc()
            cleanup_errors.append((error, detail))
    if cleanup_errors:
        raise RuntimeError("measurement failed; last complete snapshot may be from an earlier stage:\n" +
                           "\n".join(detail for _, detail in cleanup_errors)) from (primary or cleanup_errors[0][0])
    if primary is not None:
        raise primary.with_traceback(primary.__traceback__)
    return 0 if data["status"] == "passed" else 2


if __name__ == "__main__":
    sys.exit(main())
