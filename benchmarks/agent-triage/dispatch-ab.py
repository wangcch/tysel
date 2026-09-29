#!/usr/bin/env python3
"""O4 adjacent paired dispatcher diagnostic; final unobserved acceptance is separate."""
import argparse
from collections import Counter
from contextlib import contextmanager
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
PLAN_SHA256 = "3cbebc7ab7985f5c4e4a61c547b82eb50fff9e7a5dac9f971cd2e9291fdb38b8"


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


bench = load_module("dispatch_cost", HERE / "run.py")
checks = load_module("dispatch_evidence", HERE / "evidence_check.py")
require = checks.require


def plan_check(plan):
    digest = hashlib.sha256((json.dumps(plan, indent=2) + "\n").encode()).hexdigest()
    require(digest == PLAN_SHA256, "predeclared O4 plan changed")


def load_volume_protocol(plan):
    # The frozen plan's volumeProtocol records the original archive location.
    # Its hash still binds the same protocol bytes at the stable source path.
    path = HERE / "protocol-volume.json"
    require(bench.sha(path) == plan["volumeProtocolSha256"], "volume protocol changed")
    return path, checks.load_json(path.read_bytes())


def package_pair_check(packages):
    before, after = packages["baseline"], packages["candidate"]
    require(before["sources"] == after["sources"], "application/manifest source changed")
    require(before["toolchain"] == after["toolchain"], "package toolchain changed")
    for name in ("caller", "plugin", "tysel-worker"):
        require(before["artifacts"][name] == after["artifacts"][name], "package binary changed: " + name)


def schedule(plan):
    plan_check(plan)
    cells = [(size, delay) for size in plan["sizes"] for delay in plan["adapterDelayMs"]]
    rows = []
    for concurrency, blocks in ((plan["primaryConcurrency"], plan["primaryPairedBlocks"]),
                                (plan["guardConcurrency"], plan["guardPairedBlocks"])):
        for block in range(blocks):
            for cell in cells[block % len(cells):] + cells[:block % len(cells)]:
                size, delay = cell
                order = ("baseline", "candidate") if (block + cells.index(cell)) % 2 == 0 else ("candidate", "baseline")
                for variant in order:
                    rows.append(dict(sequence=len(rows), pair=f"c{concurrency}-b{block}-{size}-d{delay}",
                                     sourceVariant=variant, size=size, adapterDelayMs=delay,
                                     concurrency=concurrency, round=block))
    require(len(rows) == plan["expectedRounds"], "plan schedule size mismatch")
    return rows


class Observer:
    """Retain only route classes, timings, status codes and non-secret job identities."""
    def __init__(self, http):
        self.http, self.calls, self.lock, self.next_index = http, [], threading.Lock(), 0
        self.stopping = False

    def __call__(self, origin, path, method="GET", body=None, token=None, key=None, timeout=45):
        kind = ("admission" if path == "/jobs" and method == "POST" else
                "status" if path.startswith("/jobs/") and method == "GET" else
                "pending" if path == "/internal/pending" else
                "dispatch" if path == "/internal/dispatch" else "other")
        with self.lock:
            index = self.next_index
            self.next_index += 1
        event = dict(index=index, kind=kind, method=method, status=None)
        job = body.get("jobId") if kind == "dispatch" and isinstance(body, dict) else (
            path[len("/jobs/"):] if kind == "status" else None)
        started = time.monotonic()
        try:
            status, response = self.http(origin, path, method, body, token, key, timeout)
            event["status"] = status
            if kind == "admission" and isinstance(response, dict):
                job = response.get("jobId")
            if kind == "dispatch" and isinstance(response, dict) and response.get("status") in ("completed", "running", "pending", "accepted", "suspended"):
                event["dispatchState"] = response["status"]
            return status, response
        except BaseException as error:
            event["errorType"] = type(error).__name__
            raise
        finally:
            event["durationMs"] = (time.monotonic() - started) * 1000
            if isinstance(job, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,128}", job):
                event["jobId"] = job
            with self.lock:
                event["duringStop"] = self.stopping
                self.calls.append(event)

    def report(self):
        with self.lock:
            calls = sorted((dict(call) for call in self.calls), key=lambda call: call["index"])
            started = self.next_index
        counts = dict(Counter(call["kind"] for call in calls))
        return dict(calls=calls, started=started, counts=counts,
                    statusCounts=dict(Counter(str(call["status"]) for call in calls)),
                    durationMsByKind={kind: sum(call["durationMs"] for call in calls if call["kind"] == kind)
                                      for kind in counts})


@contextmanager
def packaged_deploy(module):
    # warm_round's isinstance checks and every deploy HTTP call must use this
    # package's class, not whichever supervisor happens to be in the checkout.
    original_deploy, original_http, original_close = bench.deploy, module.http, module.Deployment.close
    observer = Observer(original_http)
    def close(app, *args, **kwargs):
        with observer.lock:
            observer.stopping = True
        return original_close(app, *args, **kwargs)
    bench.deploy, module.http = module, observer
    module.Deployment.close = close
    try:
        yield observer
    finally:
        module.http, module.Deployment.close, bench.deploy = original_http, original_close, original_deploy


def observation_check(row, warmups, requests):
    observation = row["httpObservation"]
    calls = observation["calls"]
    require(observation["started"] == len(calls) and [call["index"] for call in calls] == list(range(len(calls))),
            "HTTP calls missing or duplicated")
    counts = dict(Counter(call["kind"] for call in calls))
    require(observation["counts"] == counts and counts.get("admission") == warmups + requests,
            "HTTP admission coverage mismatch")
    require(observation["statusCounts"] == dict(Counter(str(call["status"]) for call in calls)),
            "HTTP status counts do not recompute")
    require(set(observation["durationMsByKind"]) == set(counts), "HTTP duration groups mismatch")
    admissions = set()
    for call in calls:
        require(call["kind"] in ("admission", "status", "pending", "dispatch", "other"), "unknown observed route")
        require(type(call["duringStop"]) is bool, "missing observer stop boundary")
        internal = call["kind"] in ("pending", "dispatch")
        cancelled = internal and call["duringStop"] and call.get("errorType") is not None
        require(not call.get("errorType") or cancelled, "observed HTTP exception before stopping")
        require(call["status"] is None if cancelled else
                call["status"] in ((200, 503) if internal else (202,) if call["kind"] == "admission" else (200,)),
                "observed HTTP status failure")
        require(call["method"] == ("POST" if call["kind"] in ("admission", "dispatch") else "GET"), "observed method mismatch")
        checks.number(call["durationMs"], "HTTP call duration")
        if call["kind"] == "admission":
            require(isinstance(call.get("jobId"), str) and call["jobId"] not in admissions, "missing/duplicate admitted job")
            admissions.add(call["jobId"])
    require({sample["jobId"] for sample in row["samples"]} <= admissions, "measured jobs absent from observed admissions")
    for call in calls:
        if call["kind"] in ("status", "dispatch"):
            require(call.get("jobId") in admissions, "observed job does not match an admission")
    for kind in ("status", "dispatch"):
        require({call["jobId"] for call in calls if call["kind"] == kind} == admissions,
                "observed " + kind + " coverage incomplete")
    for kind in counts:
        checks.same_number(observation["durationMsByKind"][kind],
                           sum(call["durationMs"] for call in calls if call["kind"] == kind), "HTTP aggregate duration")
    controls = counts.get("pending", 0) + counts.get("dispatch", 0)
    require(counts.get("pending", 0) > 0 and counts.get("dispatch", 0) >= warmups + requests,
            "control HTTP observation incomplete")
    checks.same_number(row["controlRequestsPerJob"], controls / (warmups + requests), "control requests per job")


def paired_selection(pairs, plan):
    """Apply the predeclared ratios; callers must separately validate raw rounds."""
    plan_check(plan)
    expected = {item["pair"]: item for item in schedule(plan)}
    require(len(pairs) == len(expected) == plan["expectedPairs"]
            and {pair["pair"] for pair in pairs} == set(expected), "pair coverage mismatch")
    for pair in pairs:
        reference = expected[pair["pair"]]
        require(all(pair[key] == reference[key] for key in ("size", "adapterDelayMs", "concurrency")), "pair identity mismatch")
        for key in ("throughputRatio", "p95Ratio", "controlRequestsPerJobRatio"):
            checks.number(pair[key], key, positive=True)
    decisions = []
    for concurrency in (plan["primaryConcurrency"], plan["guardConcurrency"]):
        policy = plan["selection"]["eachC4Cell" if concurrency == 4 else "eachC1Cell"]
        for size in plan["sizes"]:
            for delay in plan["adapterDelayMs"]:
                rows = [pair for pair in pairs if (pair["concurrency"], pair["size"], pair["adapterDelayMs"]) == (concurrency, size, delay)]
                rate = statistics.median(pair["throughputRatio"] for pair in rows)
                latency = statistics.median(pair["p95Ratio"] for pair in rows)
                controls = statistics.median(pair["controlRequestsPerJobRatio"] for pair in rows)
                passed = (rate >= policy["medianThroughputRatioAtLeast"] and latency <= policy["medianP95RatioAtMost"]
                          and controls <= policy["medianControlRequestsPerJobRatioAtMost"]
                          and (concurrency != 4 or min(pair["throughputRatio"] for pair in rows) >= policy["allPairedThroughputRatiosAtLeast"]))
                decisions.append(dict(size=size, adapterDelayMs=delay, concurrency=concurrency,
                                      throughputRatios=[pair["throughputRatio"] for pair in rows],
                                      medianThroughputRatio=rate, medianP95Ratio=latency,
                                      medianControlRequestsPerJobRatio=controls, passed=passed))
    return decisions


def comparisons(data):
    checks.finite_tree(data)
    require(data["status"] == "complete" and data["supportClosed"] is True
            and "inFlightRound" not in data, "only fully closed complete runs can be selected")
    plan = data["plan"]
    require(data["planSha256"] == PLAN_SHA256, "retained plan hash mismatch")
    protocol = checks.protocol_check(data)
    require(data["protocolSha256"] == plan["volumeProtocolSha256"], "wrong storage protocol")
    expected = schedule(plan)
    require(len(data["rounds"]) == len(expected) == len(data["processCleanup"]), "round/cleanup coverage mismatch")
    require(not data.get("error") and not data.get("cleanupErrors"), "failed run cannot be selected")
    sizes = {size["name"]: size for size in protocol["sizes"]}
    jobs, budgets = set(), []
    for row, identity, cleanup in zip(data["rounds"], expected, data["processCleanup"]):
        require(all(row[key] == value for key, value in identity.items()), "round sequence/identity mismatch")
        require(row["variant"] == "lookup" and row["namespaceRemoved"] is True, "round namespace cleanup incomplete")
        require(row["errors"] == row["unexpectedDenials"] == 0 and row["physicalReadsIncludingWarmup"] == 40,
                "round read/output contract failed")
        samples, concurrency = row["samples"], row["concurrency"]
        require(len(samples) == plan["requestsPerRound"] and Counter(sample["client"] for sample in samples) ==
                {client: len(samples) // concurrency for client in range(concurrency)}, "sample/client coverage mismatch")
        for sample in samples:
            checks.sample_check(sample, "lookup", sizes[row["size"]], row["adapterDelayMs"], jobs)
        duration = checks.number(row["durationMs"], "round duration", positive=True)
        checks.same_number(row["jobsPerSec"], len(samples) * 1000 / duration, "throughput")
        require(all(sum(sample["e2eMs"] for sample in samples if sample["client"] == client) <= duration
                    for client in range(concurrency)), "client latency exceeds duration")
        checks.memory_check(row)
        checks.same_number(row["p95Ms"], checks.percentile([sample["e2eMs"] for sample in samples], .95), "p95")
        stats = row["stats"]
        require(len(stats["samples"]) == len(samples) and stats["p50Ci95"] is None
                and stats["p99"] is None, "helper stats coverage mismatch")
        for actual, sample in zip(stats["samples"], samples):
            checks.same_number(actual, sample["e2eMs"], "helper stats sample")
        for field, quantile in (("p50", .5), ("p95", .95)):
            checks.same_number(stats[field], checks.percentile(stats["samples"], quantile), field)
        require(cleanup["crash"] is False and cleanup["liveProcessesRemaining"] == 0 and not cleanup.get("error"), "live processes remain")
        require(len(set(cleanup["pids"])) == len(cleanup["pids"]) >= 3, "cleanup PID coverage mismatch")
        for pid in cleanup["pids"]:
            checks.number(pid, "cleanup PID", positive=True, integer=True)
        require(all(set(sample["pids"]) <= set(cleanup["pids"]) for sample in row["memory"]), "sampled process missing from cleanup")
        observation_check(row, plan["warmupRequests"], plan["requestsPerRound"])
        policy = protocol["budgets"]["lookup"]
        budgets.append(dict(sequence=row["sequence"], passed=(row["p95Ms"] <= policy["warmP95Ms"]
                            and row["jobsPerSec"] >= policy["minJobsPerSec"][str(concurrency)]
                            and row["peakPssKiB"] <= policy["peakPssKiB"])))
    pairs = []
    for index in range(0, len(data["rounds"]), 2):
        rows = {row["sourceVariant"]: row for row in data["rounds"][index:index + 2]}
        before, after = rows["baseline"], rows["candidate"]
        pairs.append(dict(pair=before["pair"], size=before["size"], concurrency=before["concurrency"],
                          adapterDelayMs=before["adapterDelayMs"], throughputRatio=after["jobsPerSec"] / before["jobsPerSec"],
                          p95Ratio=after["p95Ms"] / before["p95Ms"],
                          controlRequestsPerJobRatio=after["controlRequestsPerJob"] / before["controlRequestsPerJob"]))
    decisions = paired_selection(pairs, plan)
    passed = all(item["passed"] for item in decisions + budgets)
    return dict(pairs=pairs, cells=decisions, roundBudgets=budgets, pairedCriteriaPassed=passed,
                eligibleForFullValidation=passed, retainCandidate=False,
                interpretation="Adjacent observed engineering pairs; retention still requires the unobserved full matrix, product regressions, dispatcher fault tests and exact-package lifecycle")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("bin-dir", "baseline-release", "candidate-release", "output", "plan"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    require(platform.system() == "Linux", "paired PSS experiment requires Linux")
    require(sys.flags.optimize == 0, "run live sampling without -O; reused workload assertions must stay enabled")
    plan = checks.load_json(args.plan.read_bytes())
    require(bench.sha(args.plan) == PLAN_SHA256, "predeclared plan bytes changed")
    plan_check(plan)
    require(bench.sha(HERE / "protocol.json") == plan["originalProtocolSha256"], "original protocol changed")
    protocol_path, protocol = load_volume_protocol(plan)
    releases = {"baseline": args.baseline_release.resolve(), "candidate": args.candidate_release.resolve()}
    require(str(releases["baseline"]) == plan["baselineRelease"], "must use exact retained baseline release")
    require(releases["baseline"] != releases["candidate"] and all(path.name == "lookup" for path in releases.values()), "separate lookup release directories required")
    require(bench.sha(releases["baseline"] / "deploy.py") == plan["baselineSupervisorSha256"], "baseline supervisor changed")
    output = args.output.resolve()
    require(not output.exists(), "choose a fresh output directory; preserve previous evidence")
    output.mkdir(parents=True)
    result = dict(kind="O4 paired dispatcher diagnostic", status="running", plan=plan, planSha256=PLAN_SHA256,
                  protocol=protocol, protocolSha256=bench.sha(protocol_path), command=sys.argv,
                  startedAtUnixMs=time.time_ns() // 1000000, rounds=[], processCleanup=[])
    support, primary, cleanup_errors = None, None, []
    try:
        shutil.copyfile(args.plan, output / "predeclared.json")
        shutil.copyfile(protocol_path, output / "protocol.json")
        require(all(path.stat().st_dev == output.stat().st_dev for path in releases.values()), "packages and namespaces must share the same volume filesystem")
        modules = {key: load_module("dispatch_" + key, path / "deploy.py") for key, path in releases.items()}
        packages = {key: modules[key].verify(path) for key, path in releases.items()}
        result.update(packages=packages, releases={key: str(path) for key, path in releases.items()},
                      releaseJsonSha256={key: bench.sha(path / "release.json") for key, path in releases.items()})
        package_pair_check(packages)
        require(packages["candidate"]["artifacts"]["deploy.py"]["sha256"] == bench.sha(bench.EXAMPLE / "deploy.py"),
                "candidate supervisor differs from recorded checkout source")
        baseline_path = releases["baseline"].parents[1] / "measurements.json"
        baseline = checks.load_json(baseline_path.read_bytes())
        require(baseline["artifacts"]["lookup"] == packages["baseline"], "baseline package differs from retained measured package")
        binaries = {name: dict(sha256=bench.sha(args.bin_dir / name), bytes=(args.bin_dir / name).stat().st_size)
                    for name in ("tysel", "tysel-service", "tysel-worker", "tysel-bench-agent-support")}
        require(binaries == baseline["binaries"], "native tools differ from the measured baseline")
        sources = [path for folder in (HERE, bench.EXAMPLE, bench.REPO / "examples/isolated-plugin/src", bench.REPO / "crates/tysel-bench-compare/src")
                   for path in folder.rglob("*") if path.is_file() and path.suffix in (".py", ".ts", ".toml", ".rs", ".json")]
        native = hashlib.sha256(b"".join(str(path.relative_to(bench.REPO)).encode() + b"\0" + path.read_bytes() + b"\0"
                                       for path in sorted(bench.REPO.glob("crates/*/src/**/*.rs")))).hexdigest()
        require(native == baseline["nativeSourceTreeSha256"], "native source tree changed")
        result.update(sources={str(path.relative_to(bench.REPO)): bench.sha(path) for path in sorted(set(sources))},
                      driverSha256=bench.sha(Path(__file__)), nativeSourceTreeSha256=native,
                      binaries=binaries, baselineMeasurementSha256=bench.sha(baseline_path),
                      sourceCommit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=bench.REPO, text=True).strip(),
                      workspaceDirty=bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=bench.REPO)),
                      kernel=platform.release(), mountinfo=Path("/proc/self/mountinfo").read_text())
        support = bench.Support(args.bin_dir / "tysel-bench-agent-support")
        result["system"] = support.call(op="system")
        bench.save(output / "measurements.json", result)
        for identity in schedule(plan):
            variant = identity["sourceVariant"]
            root = output / ("namespace-" + str(identity["sequence"]))
            pending = dict(identity, namespace=str(root), startLoadAverage=list(os.getloadavg()))
            result["inFlightRound"] = pending
            with packaged_deploy(modules[variant]) as observer:
                try:
                    size = next(size for size in protocol["sizes"] if size["name"] == identity["size"])
                    row = bench.warm_round(releases[variant].parent, support, protocol, root, "lookup", size,
                                           identity["concurrency"], identity["adapterDelayMs"], identity["round"])
                finally:
                    pending["httpObservation"] = observer.report()
                    bench.cleanup_all(("round namespace", lambda: shutil.rmtree(root) if root.exists() else None))
            stats = support.call(op="stats", series={"e2e": [sample["e2eMs"] for sample in row["samples"]]})["e2e"]
            observation = pending["httpObservation"]
            row.update(pending, stats=stats, p95Ms=stats["p95"], endLoadAverage=list(os.getloadavg()), namespaceRemoved=not root.exists(),
                       controlRequestsPerJob=(observation["counts"].get("pending", 0) + observation["counts"].get("dispatch", 0)) / 40)
            result["rounds"].append(row)
            del result["inFlightRound"]
            result["processCleanup"] = support.cleanup
            bench.save(output / "measurements.json", result)
            print(f"{identity['sequence']+1}/48 {identity['pair']} {variant}: {row['jobsPerSec']:.3f} jobs/s; p95 {row['p95Ms']:.2f} ms", flush=True)
    except BaseException as error:
        primary = error
        result.update(status="error", error=traceback.format_exc())
    finally:
        result["processCleanup"] = support.cleanup if support is not None else []
        for label, action in (("support close", lambda: support.close() if support is not None else None),
                              ("finished metadata", lambda: result.update(finishedAtUnixMs=time.time_ns() // 1000000))):
            try:
                action()
                if label == "support close":
                    result["supportClosed"] = support is not None
            except BaseException as error:
                cleanup_errors.append((error, label + ":\n" + traceback.format_exc()))
        if primary is None and not cleanup_errors:
            try:
                result["status"] = "complete"
                result["comparison"] = comparisons(result)
            except BaseException as error:
                primary = error
                result.update(status="error", error=traceback.format_exc())
        if cleanup_errors:
            result.update(status="error", cleanupErrors=[detail for _, detail in cleanup_errors])
        try:
            bench.save(output / "measurements.json", result)
        except BaseException as error:
            cleanup_errors.append((error, "final evidence save failed:\n" + traceback.format_exc()))
    if cleanup_errors:
        raise RuntimeError("paired measurement failed; retained snapshot may be incomplete:\n" +
                           "\n".join(detail for _, detail in cleanup_errors)) from (primary or cleanup_errors[0][0])
    if primary is not None:
        raise primary.with_traceback(primary.__traceback__)
    print(json.dumps(result["comparison"]["cells"]), flush=True)
    return 0 if result["comparison"]["pairedCriteriaPassed"] else 2


if __name__ == "__main__":
    sys.exit(main())
