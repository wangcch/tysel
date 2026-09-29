#!/usr/bin/env python3
"""O4.1 capacity and cost diagnostic using complete, bounded application jobs."""
import argparse
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import platform
import shutil
import statistics
import sys
import time
import traceback

HERE = Path(__file__).resolve().parent


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


paired = load_module("capacity_paired", HERE / "dispatch-ab.py")
bench, require = paired.bench, paired.require


def validate_plan(plan):
    require(plan["schemaVersion"] == 1, "unsupported capacity protocol")
    require(plan["limits"] == bench.deploy.LIMITS, "application limits must remain unchanged")
    require(plan["concurrency"] == [1, 2, 4, 8], "capacity coverage must be 1/2/4/8")
    require(plan["retainedJobs"] == [0, 900] and plan["adapterDelayMs"] == [0, 20], "state/delay coverage changed")
    require(plan["rounds"] == 3 and plan["requestsPerRound"] == 64 and plan["warmupRequests"] == 8, "sample coverage changed")
    require(plan["size"] == {"name": "bounded", "tickets": 16, "subjectChars": 500}, "payload changed")
    require(plan["statusPollMs"] == 5 and plan["memoryIntervalMs"] == 50, "observation intervals changed")
    require(plan["seedConcurrency"] == 8 and plan["phaseTimeoutSec"] == 240, "seed or timeout changed")
    require(max(plan["retainedJobs"]) + plan["warmupRequests"] + plan["requestsPerRound"] < plan["limits"]["retained"], "measurement would hit retention limit")


def schedule(plan):
    validate_plan(plan)
    cells = [(retained, delay, concurrency) for retained in plan["retainedJobs"]
             for delay in plan["adapterDelayMs"] for concurrency in plan["concurrency"]]
    rows = []
    for block in range(plan["rounds"]):
        rotated = cells[block:] + cells[:block]
        for retained, delay, concurrency in rotated:
            rows.append(dict(sequence=len(rows), round=block, retainedJobs=retained,
                             adapterDelayMs=delay, concurrency=concurrency))
    return rows


def parse_cpu_stat(text):
    # comm can contain spaces and parentheses. Fields after its final ')' start at 3.
    fields = text.rsplit(") ", 1)[1].split()
    user, system, started = (int(fields[index]) for index in (11, 12, 19))
    require(min(user, system, started) >= 0, "invalid process CPU counters")
    return dict(userTicks=user, systemTicks=system, startTicks=started)


def cpu_snapshot(pids):
    return {str(pid): parse_cpu_stat(Path(f"/proc/{pid}/stat").read_text()) for pid in pids}


def cpu_delta(before, after, ticks_per_second):
    require(ticks_per_second > 0 and math.isfinite(ticks_per_second), "invalid clock tick frequency")
    require(before and before.keys() == after.keys(), "native process set changed during measurement")
    total = 0
    for pid, previous in before.items():
        current = after[pid]
        require(previous["startTicks"] == current["startTicks"], "native PID was reused")
        for field in ("userTicks", "systemTicks"):
            require(current[field] >= previous[field], "native CPU counter decreased")
            total += current[field] - previous[field]
    return total * 1000 / ticks_per_second


def fixture(plan):
    provider = bench.fixture(plan["size"]["subjectChars"], 0)
    provider.customers = [dict(id=f"bench-c{c}", token=f"bench-token-{c}", tickets=[
        dict(id=f"t{c}-{i:02}", priority=3 if i == 0 else 0) for i in range(16)
    ]) for c in range(8)]
    provider.records = {ticket["id"]: customer["id"] for customer in provider.customers for ticket in customer["tickets"]}
    provider.capacity_delay_ms = 0

    def reply(identifier, status, data):
        started = time.monotonic()
        if provider.capacity_delay_ms:
            time.sleep(provider.capacity_delay_ms / 1000)
        ended = time.monotonic()
        with provider.lock:
            provider.delays.append(dict(ticketId=identifier, start=started, end=ended,
                                       actualSleepMs=(ended - started) * 1000 if provider.capacity_delay_ms else 0,
                                       responseBytes=len(data)))
        return status, data

    provider.adapter_reply = reply
    return provider


def jobs(app, provider, plan, count, concurrency, prefix):
    deadline = time.monotonic() + plan["phaseTimeoutSec"]

    def client(c):
        result = []
        for i in range(c, count, concurrency):
            require(time.monotonic() < deadline, "capacity phase exceeded its deadline")
            result.append(bench.request(app, provider, "lookup", plan["size"], c,
                                        f"{prefix}-{i}", plan["statusPollMs"] / 1000))
        return result

    with ThreadPoolExecutor(max_workers=concurrency) as pool:
        return [sample for batch in pool.map(client, range(concurrency)) for sample in batch]


def drained(app):
    bench.wait_for(lambda: not bench.query(app.state,
        "SELECT id FROM triage_jobs WHERE delivery='pending' OR state IN ('accepted','running','recovering')"))


@contextmanager
def application(release, state, provider, support, snapshot=None, observer=None):
    if snapshot is None:
        bench.deploy.initialize(release, state, bench.settings(provider))
    else:
        bench.deploy.restore(release, snapshot, state)
    app = bench.deploy.Deployment(release, state, bench.base.FAKE_SECRET)
    try:
        yield app
    finally:
        if observer is not None:
            observer.stopping = True
        bench.stop_app(app, support)


def seed(release, root, provider, support, plan):
    provider.capacity_delay_ms = 0
    before = provider.count()
    started = time.monotonic()
    with application(release, root / "seed-state", provider, support) as app:
        samples = jobs(app, provider, plan, 900, plan["seedConcurrency"], "seed")
        bench.attach_audit(app, samples)
        drained(app)
        count = bench.query(app.state, "SELECT count(*) AS n FROM triage_jobs WHERE state='succeeded'")[0]["n"]
        require(count == 900 and provider.count() - before == 900, "seed jobs/reads mismatch")
    snapshot = root / "seed-snapshot"
    bench.deploy.backup(root / "seed-state", snapshot)
    return dict(jobs=900, authorizedReads=900, durationMs=(time.monotonic() - started) * 1000,
                sampleSha256=hashlib.sha256(json.dumps(samples, sort_keys=True).encode()).hexdigest(),
                snapshotSha256=bench.sha(snapshot / "snapshot.json"))


def round_run(release, root, provider, support, plan, identity):
    provider.capacity_delay_ms = identity["adapterDelayMs"]
    original = bench.deploy.http
    observer = paired.Observer(original)
    bench.deploy.http = observer
    state = root / f"namespace-{identity['sequence']}"
    reads_before = provider.count()
    try:
        with application(release, state, provider, support,
                         root / "seed-snapshot" if identity["retainedJobs"] else None, observer) as app:
            require(bench.query(state, "SELECT count(*) AS n FROM triage_jobs")[0]["n"] == identity["retainedJobs"], "initial retained count mismatch")
            jobs(app, provider, plan, plan["warmupRequests"], identity["concurrency"], "warm")
            drained(app)
            memory = bench.Memory(support, bench.roots(app), plan["memoryIntervalMs"] / 1000)
            try:
                pids_before = support.call(op="memory", roots=bench.roots(app))["pids"]
                cpu_before = cpu_snapshot(pids_before)
                harness_before = time.process_time()
                started = time.monotonic()
                samples = jobs(app, provider, plan, plan["requestsPerRound"], identity["concurrency"], "measure")
                elapsed_ms = (time.monotonic() - started) * 1000
                harness_ms = (time.process_time() - harness_before) * 1000
                pids_after = support.call(op="memory", roots=bench.roots(app))["pids"]
                cpu_after = cpu_snapshot(pids_after)
                cpu_window_ms = (time.monotonic() - started) * 1000
            finally:
                memory.close()
            ticks = os.sysconf("SC_CLK_TCK")
            native_ms = cpu_delta(cpu_before, cpu_after, ticks)
            require(all(set(m["pids"]) == set(pids_before) for m in memory.samples), "sampled process tree changed")
            bench.attach_audit(app, samples)
            drained(app)
            final_count = bench.query(state, "SELECT count(*) AS n FROM triage_jobs")[0]["n"]
            expected = identity["retainedJobs"] + plan["warmupRequests"] + len(samples)
            require(final_count == expected, "final retained count mismatch")
            require(provider.count() - reads_before == plan["warmupRequests"] + len(samples), "round physical read count mismatch")
            with provider.lock:
                require(all(r["authorized"] for r in provider.requests), "unauthorized fixture read")
            result = dict(identity, variant="lookup", samples=samples, durationMs=elapsed_ms,
                          jobsPerSec=len(samples) * 1000 / elapsed_ms,
                          cpu=dict(before=cpu_before, after=cpu_after, ticksPerSecond=ticks,
                                   windowMs=cpu_window_ms, nativeMs=native_ms,
                                   nativeMsPerJob=native_ms / len(samples), harnessMs=harness_ms),
                          memory=memory.samples, idlePssKiB=memory.samples[0]["valueKiB"],
                          peakPssKiB=max(m["valueKiB"] for m in memory.samples),
                          finalRetainedJobs=final_count, physicalReadsIncludingWarmup=plan["warmupRequests"] + len(samples))
    finally:
        bench.deploy.http = original
    result["httpObservation"] = observer.report()
    shutil.rmtree(state)
    result["namespaceRemoved"] = True
    return result


def summarize(data):
    paired.checks.finite_tree(data)
    require(data["status"] == "complete" and data["supportClosed"], "incomplete capacity evidence")
    plan = data["protocol"]
    expected = schedule(plan)
    require(len(data["rounds"]) == len(expected), "capacity round coverage mismatch")
    require(len(data["processCleanup"]) == len(expected) + 1, "capacity cleanup coverage mismatch")
    require(all(c["liveProcessesRemaining"] == 0 and not c.get("error") for c in data["processCleanup"]), "capacity process cleanup failed")
    seen = set()
    for row, identity in zip(data["rounds"], expected):
        require(all(row[k] == v for k, v in identity.items()), "capacity schedule changed")
        require(len(row["samples"]) == plan["requestsPerRound"] and row["namespaceRemoved"], "capacity sample/cleanup mismatch")
        require(Counter(s["client"] for s in row["samples"]) == {c: plan["requestsPerRound"] // row["concurrency"] for c in range(row["concurrency"])}, "client coverage mismatch")
        for sample in row["samples"]:
            paired.checks.sample_check(sample, "lookup", plan["size"], row["adapterDelayMs"], seen, clients=row["concurrency"])
        require(all(sum(s["e2eMs"] for s in row["samples"] if s["client"] == client) <= row["durationMs"]
                    for client in range(row["concurrency"])), "client latency exceeds round duration")
        paired.checks.same_number(row["jobsPerSec"], len(row["samples"]) * 1000 / paired.checks.number(row["durationMs"], "round duration", positive=True), "capacity throughput")
        paired.checks.memory_check(row)
        cpu = row["cpu"]
        require(all({str(pid) for pid in sample["pids"]} == set(cpu["before"]) for sample in row["memory"]), "CPU and memory process sets differ")
        paired.checks.same_number(cpu["nativeMs"], cpu_delta(cpu["before"], cpu["after"], cpu["ticksPerSecond"]), "native CPU duration")
        paired.checks.same_number(cpu["nativeMsPerJob"], cpu["nativeMs"] / len(row["samples"]), "native CPU/job")
        paired.checks.number(cpu["harnessMs"], "harness CPU duration")
        require(paired.checks.number(cpu["windowMs"], "CPU window", positive=True) >= row["durationMs"], "CPU window omits measured work")
        require(row["finalRetainedJobs"] == row["retainedJobs"] + 72 and row["physicalReadsIncludingWarmup"] == 72, "retention/read coverage mismatch")
        # Reuse the O4 observer's structural/count checks with this round's denominator.
        row_for_check = dict(row, controlRequestsPerJob=(row["httpObservation"]["counts"].get("pending", 0) + row["httpObservation"]["counts"].get("dispatch", 0)) / 72)
        paired.observation_check(row_for_check, 8, 64)
    cells = []
    for retained in plan["retainedJobs"]:
        for delay in plan["adapterDelayMs"]:
            for concurrency in plan["concurrency"]:
                rows = [r for r in data["rounds"] if (r["retainedJobs"], r["adapterDelayMs"], r["concurrency"]) == (retained, delay, concurrency)]
                samples = [s for r in rows for s in r["samples"]]
                cells.append(dict(retainedJobs=retained, adapterDelayMs=delay, concurrency=concurrency,
                    jobsPerSec=[r["jobsPerSec"] for r in rows], p95Ms=paired.checks.percentile([s["e2eMs"] for s in samples], .95),
                    nativeCpuMsPerJob=statistics.median(r["cpu"]["nativeMsPerJob"] for r in rows),
                    harnessCpuMsPerJob=statistics.median(r["cpu"]["harnessMs"] / 64 for r in rows),
                    peakPssKiB=max(r["peakPssKiB"] for r in rows),
                    statusRequestsPerJob=statistics.median(r["httpObservation"]["counts"].get("status", 0) / 72 for r in rows),
                    totalHttpPerJob=statistics.median(sum(r["httpObservation"]["counts"].get(k, 0) for k in ("admission", "status", "pending", "dispatch")) / 72 for r in rows)))
    return dict(status="valid-diagnostic", productionCandidateSelected=False, cells=cells)


def main():
    if not __debug__:
        raise RuntimeError("capacity runner requires assertions in reused application correctness checks; run without -O")
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--protocol", type=Path, default=HERE / "capacity-protocol.json")
    args = parser.parse_args()
    require(platform.system() == "Linux", "capacity diagnostic requires Linux CPU/PSS counters")
    plan = json.loads(args.protocol.read_text())
    validate_plan(plan)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    shutil.copy(args.protocol, output / "protocol.json")
    data = dict(status="running", protocol=plan, protocolSha256=bench.sha(args.protocol),
                startedAtUnixMs=time.time_ns() // 1000000, command=sys.argv,
                cpuScope="caller/plugin/worker utime+stime deltas; process identities must remain stable; excludes Python and benchmark helper",
                harnessCpuScope="combined Python supervisor, fixture, observer, load client and sampling threads; not a standalone product cost",
                httpScope="whole deployment round including eight warmups and shutdown; CPU/throughput cover 64 measured jobs",
                rounds=[], processCleanup=[], supportClosed=False)
    support = provider = None
    try:
        data["sources"] = {str(p.relative_to(bench.REPO)): bench.sha(p) for p in sorted(set(
            list(HERE.glob("*.py")) + list(HERE.glob("*.json")) + list(bench.EXAMPLE.rglob("*.ts")) + list(bench.EXAMPLE.glob("*.py"))))}
        data["nativeSourceTreeSha256"] = hashlib.sha256(b"".join(str(p.relative_to(bench.REPO)).encode() + b"\0" + p.read_bytes() + b"\0" for p in sorted(bench.REPO.glob("crates/*/src/**/*.rs")))).hexdigest()
        data["binaries"] = {n: dict(sha256=bench.sha(args.bin_dir / n), bytes=(args.bin_dir / n).stat().st_size) for n in ("tysel", "tysel-service", "tysel-worker", "tysel-bench-agent-support")}
        release = output / "release"
        data["package"] = bench.package.package(args.bin_dir, release)
        require(release.stat().st_dev == output.stat().st_dev, "release and namespace storage differ")
        support = bench.Support(args.bin_dir / "tysel-bench-agent-support")
        data["system"] = support.call(op="system")
        provider = fixture(plan)
        data["seed"] = seed(release, output, provider, support, plan)
        bench.save(output / "measurements.json", data)
        print("900 real seed jobs completed and backed up", flush=True)
        for identity in schedule(plan):
            data["rounds"].append(round_run(release, output, provider, support, plan, identity))
            bench.save(output / "measurements.json", data)
            print(json.dumps(identity), flush=True)
        data["status"] = "complete"
    except BaseException:
        data["status"] = "error"
        data["error"] = traceback.format_exc()
        raise
    finally:
        try:
            bench.cleanup_all(("fixture", lambda: provider.close() if provider is not None else None),
                              ("support", lambda: support.close() if support is not None else None))
            data["supportClosed"] = support is not None
        except BaseException:
            data["status"] = "error"
            data["cleanupError"] = traceback.format_exc()
            raise
        finally:
            data["processCleanup"] = support.cleanup if support is not None else []
            data["finishedAtUnixMs"] = time.time_ns() // 1000000
            bench.save(output / "measurements.json", data)
    try:
        summary = summarize(data)
    except BaseException:
        data["status"] = "error"
        data["error"] = traceback.format_exc()
        bench.save(output / "measurements.json", data)
        raise
    bench.save(output / "summary.json", summary)
    print(json.dumps(dict(status=summary["status"], rounds=len(data["rounds"]),
                          measuredJobs=sum(len(r["samples"]) for r in data["rounds"]))), flush=True)


if __name__ == "__main__":
    main()
