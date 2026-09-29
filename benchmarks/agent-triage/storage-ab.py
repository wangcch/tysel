#!/usr/bin/env python3
"""P5.2 paired state-placement diagnostic; never substitutes for cost acceptance."""
import argparse
from contextlib import contextmanager
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import statistics
import subprocess
import time
import traceback

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("agent_cost", HERE / "run.py")
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)


def storage(path):
    resolved = path.resolve()
    stats = os.statvfs(resolved)
    result = subprocess.run(["findmnt", "-J", "-T", str(resolved), "-o", "TARGET,SOURCE,FSTYPE,OPTIONS"],
                            capture_output=True, text=True, check=True)
    return dict(path=str(path), realpath=str(resolved), device=os.stat(resolved).st_dev,
                filesystem=json.loads(result.stdout), blockSize=stats.f_frsize,
                availableBytes=stats.f_bavail * stats.f_frsize)


def db_snapshot(path):
    with sqlite3.connect(f"file:{path}?mode=ro", uri=True) as db:
        return dict(path=str(path.resolve()), databaseList=db.execute("PRAGMA database_list").fetchall(),
                    journalMode=db.execute("PRAGMA journal_mode").fetchone()[0],
                    verifierConnectionSynchronous=db.execute("PRAGMA synchronous").fetchone()[0],
                    integrity=db.execute("PRAGMA integrity_check").fetchone()[0])


@contextmanager
def relocated_application(releases, variant, root, provider, support, placement, bind_root, volume_root, observations):
    assert variant == "lookup"
    destinations, app, observation = {}, None, None
    original_process = bench.deploy.Process
    root.mkdir()
    try:
        bench.deploy.initialize(releases / "lookup", root, bench.settings(provider))
        for kind, local in zip(("config", "jobs", "durable"), placement):
            destination = (volume_root if local else bind_root) / root.name / kind
            destination.mkdir(parents=True)
            destinations[kind] = destination
        config, jobs = root / "caller/config", root / "caller/data"
        # Do not copy host SELinux xattrs across filesystems. Preserve content and
        # the original private mode; labels belong to the destination mount.
        shutil.copyfile(config / "service.json", destinations["config"] / "service.json")
        (destinations["config"] / "service.json").chmod(0o600)
        shutil.rmtree(config)
        jobs.rmdir()
        config.symlink_to(destinations["config"], target_is_directory=True)
        jobs.symlink_to(destinations["jobs"], target_is_directory=True)
        runtime_db = destinations["durable"] / "durable-events.db"

        class DiagnosticProcess(original_process):
            def __init__(self, executable, cwd, worker, secret=""):
                # Only diagnostics inject this existing runtime path option.
                original_popen = subprocess.Popen
                def popen(*args, **kwargs):
                    if Path(executable) == releases / "lookup/caller":
                        kwargs["env"] = dict(kwargs["env"], TYSEL_DURABLE_SQLITE_PATH=str(runtime_db))
                    return original_popen(*args, **kwargs)
                subprocess.Popen = popen
                try:
                    super().__init__(executable, cwd, worker, secret)
                finally:
                    subprocess.Popen = original_popen

        bench.deploy.Process = DiagnosticProcess
        app = bench.deploy.Deployment(releases / "lookup", root, bench.base.FAKE_SECRET)
        # Capture actual open paths before timed warmup/measurement.
        fds = []
        for fd in Path(f"/proc/{app.caller.process.pid}/fd").iterdir():
            try:
                value = os.readlink(fd)
            except FileNotFoundError:
                continue
            if value.endswith(".db"):
                fds.append(value)
        expected = [str((destinations["jobs"] / "jobs.db").resolve()), str(runtime_db.resolve())]
        assert set(expected).issubset(fds), (expected, fds)
        assert not (destinations["jobs"] / "durable-events.db").exists(), "runtime override not applied"
        observation = dict(namespace=str(root), placement=list(placement), paths={k: storage(v) for k, v in destinations.items()},
                           openDatabaseFiles=fds, databases={"jobs": db_snapshot(Path(expected[0])), "durable": db_snapshot(runtime_db)})
        assert all(v["journalMode"] == "delete" and v["integrity"] == "ok" for v in observation["databases"].values()), observation
        observations.append(observation)
        yield app
    finally:
        bench.deploy.Process = original_process
        try:
            if app:
                bench.stop_app(app, support)
                if observation is not None:
                    observation["postRunDatabases"] = {"jobs": db_snapshot(destinations["jobs"] / "jobs.db"),
                                                       "durable": db_snapshot(runtime_db)}
                    assert all(v["integrity"] == "ok" for v in observation["postRunDatabases"].values())
                    capabilities = {}
                    for line in app.caller.logs:
                        try:
                            event = json.loads(line)
                        except ValueError:
                            continue
                        if all(k in event for k in ("capability", "operation", "ms")):
                            key = event["capability"] + "/" + event["operation"]
                            item = capabilities.setdefault(key, dict(calls=0, summedElapsedMs=0))
                            item["calls"] += 1
                            item["summedElapsedMs"] += event["ms"]
                    observation["nativeCapabilityTail"] = dict(scope="Last 200 log lines only; not whole-round call counts or time", summary=capabilities)
        finally:
            # Disposable split-layout diagnostics are not supported backups.
            shutil.rmtree(root)
            for destination in destinations.values():
                shutil.rmtree(destination)
                if not any(destination.parent.iterdir()):
                    destination.parent.rmdir()


def comparisons(result):
    pairs = []
    for pair in sorted({r["pair"] for r in result["rounds"]}):
        rows = {r["sourceVariant"]: r for r in result["rounds"] if r["pair"] == pair}
        assert set(rows) == {"baseline", "candidate"}, pair
        before, after = rows["baseline"], rows["candidate"]
        pairs.append(dict(pair=pair, layout=before["layout"], size=before["size"],
                          throughputRatio=after["jobsPerSec"] / before["jobsPerSec"],
                          p95Ratio=after["p95Ms"] / before["p95Ms"]))
    decision = []
    for size in result["plan"]["sizes"]:
        rows = [p for p in pairs if p["layout"] == "all-volume" and p["size"] == size]
        assert len(rows) == 4
        rate = statistics.median(p["throughputRatio"] for p in rows)
        latency = statistics.median(p["p95Ratio"] for p in rows)
        passed = all(p["throughputRatio"] >= 1 for p in rows) and rate >= 1.05 and latency <= 1
        decision.append(dict(size=size, pairedThroughputRatios=[p["throughputRatio"] for p in rows],
                             medianThroughputRatio=rate, medianP95Ratio=latency, passed=passed))
    return dict(pairs=pairs, targetDecision=decision,
                retainCandidate=all(d["passed"] for d in decision),
                interpretation="Predeclared engineering selection, not statistical significance or complete cost acceptance")


def main():
    bench.require_assertions()
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("bin-dir", "baseline-release", "candidate-release", "bind-root", "volume-root", "output", "plan"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    output = args.output.resolve()
    assert not output.exists(), "preserve previous measurements"
    output.mkdir(parents=True)
    for path in (args.bind_root, args.volume_root):
        assert not path.exists(), "choose fresh diagnostic roots"
        path.mkdir(parents=True)
    plan = json.loads(args.plan.read_bytes())
    protocol = json.loads((HERE / "protocol.json").read_bytes())
    assert plan["originalProtocolSha256"] == bench.sha(HERE / "protocol.json")
    assert plan["pairedBlocks"] == 4 and plan["sizes"] == [s["name"] for s in protocol["sizes"]]
    releases = {"baseline": args.baseline_release.resolve(), "candidate": args.candidate_release.resolve()}
    packages = {k: bench.deploy.verify(p) for k, p in releases.items()}
    assert {k: v["artifacts"]["caller"]["sha256"] for k, v in packages.items()} == plan["callerArtifactSha256"]
    original_application = bench.application
    result = dict(kind="paired storage diagnostic, not cost acceptance", status="running", plan=plan,
                  startedAtUnixMs=time.time_ns() // 1000000, packages=packages, rounds=[], observations=[])
    support, primary = None, None
    cleanup_errors = []
    try:
        support = bench.Support(args.bin_dir / "tysel-bench-agent-support")
        result.update(planSha256=bench.sha(args.plan), driverSha256=bench.sha(Path(__file__)),
                      mountinfo=Path("/proc/self/mountinfo").read_text(), system=support.call(op="system"))
        # Match mechanism for every diagnostic layout; no instrumented TypeScript.
        index = 0
        layouts = plan["layouts"]
        for block in range(plan["pairedBlocks"]):
            ordered = layouts[block % len(layouts):] + layouts[:block % len(layouts)]
            sizes = protocol["sizes"] if block % 2 == 0 else list(reversed(protocol["sizes"]))
            for layout_index, layout in enumerate(ordered):
                for size_index, size in enumerate(sizes):
                    pair = f"b{block}-{layout['name']}-{size['name']}"
                    order = ("baseline", "candidate") if (block + layouts.index(layout) + protocol["sizes"].index(size)) % 2 == 0 else ("candidate", "baseline")
                    for variant in order:
                        start_load = list(os.getloadavg())
                        root = output / f"namespace-{index}"
                        # warm_round expects releases/lookup; original exact packages already have that name.
                        release_root = releases[variant].parent
                        def application(release_paths, kind, state, provider, helper):
                            return relocated_application(release_paths, kind, state, provider, helper,
                                                         layout["volume"], args.bind_root.resolve(), args.volume_root.resolve(), result["observations"])
                        bench.application = application
                        row = bench.warm_round(release_root, support, protocol, root, "lookup", size, 4, 0, block)
                        stats = support.call(op="stats", series={"e2e": [s["e2eMs"] for s in row["samples"]]})["e2e"]
                        row.update(pair=pair, sourceVariant=variant, layout=layout["name"], p95Ms=stats["p95"],
                                   sequence=index, startLoadAverage=start_load, endLoadAverage=list(os.getloadavg()))
                        result["rounds"].append(row)
                        bench.save(output / "measurements.json", result)
                        print(f"{index+1}/80 {pair} {variant}: {row['jobsPerSec']:.3f} jobs/s; p95 {row['p95Ms']:.2f} ms", flush=True)
                        index += 1
        assert len(result["rounds"]) == 80
        result["comparison"] = comparisons(result)
    except BaseException as error:
        primary = error
        result["status"] = "error"
        result["error"] = traceback.format_exc()
    finally:
        bench.application = original_application
        result["processCleanup"] = support.cleanup if support is not None else []
        for label, action in (
            ("support close", lambda: support.close() if support is not None else None),
            ("finished metadata", lambda: result.update(finishedAtUnixMs=time.time_ns() // 1000000)),
        ):
            try:
                action()
            except BaseException as error:
                cleanup_errors.append((error, label + ":\n" + traceback.format_exc()))
        if cleanup_errors:
            result["status"] = "error"
            result["cleanupErrors"] = [detail for _, detail in cleanup_errors]
            if primary is None:
                result["error"] = result["cleanupErrors"][0]
        elif primary is None:
            result["status"] = "complete"
        try:
            bench.save(output / "measurements.json", result)
        except BaseException as error:
            cleanup_errors.append((error, "final evidence save failed:\n" + traceback.format_exc()))
    if cleanup_errors:
        raise RuntimeError("storage diagnostic failed; final evidence may be incomplete:\n" +
                           "\n".join(detail for _, detail in cleanup_errors)) from (primary or cleanup_errors[0][0])
    if primary is not None:
        raise primary.with_traceback(primary.__traceback__)
    print(json.dumps(result["comparison"]["targetDecision"]), flush=True)


if __name__ == "__main__":
    main()
