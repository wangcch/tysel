#!/usr/bin/env python3
"""Diagnostic SQL call attribution in a temporary caller copy, not acceptance."""
import argparse
from contextlib import ExitStack
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import threading
import time
import traceback
import urllib.error
import urllib.request

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("agent_cost", HERE / "run.py")
bench = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bench)

INSTRUMENTATION = '''
const events: { kind: string; category: string; ms: number; atMs: number }[] = [];
export function takeProfile() { return events.splice(0); }
async function measured<T>(kind: string, sql: string, operation: () => Promise<T>): Promise<T> {
  const normalized = sql.trim().replace(/\\s+/g, " ").toUpperCase();
  const category = /^(CREATE|ALTER|PRAGMA) /.test(normalized) ? "schema" :
    normalized.startsWith("UPDATE TRIAGE_JOBS SET STATE = 'EXPIRED'") ||
    normalized.startsWith("UPDATE TRIAGE_JOBS SET STATE = CASE") ||
    normalized.startsWith("SELECT 1 AS MAINTENANCE_NEEDED FROM TRIAGE_JOBS") ||
    normalized.startsWith("UPDATE TRIAGE_JOBS SET DELIVERY = 'UNRESOLVED'") ? "maintenance" : "business";
  const started = Date.now();
  try { return await operation(); }
  finally { events.push({ kind, category, ms: Date.now() - started, atMs: Date.now() }); }
}
export function measuredExec(...args: Parameters<typeof tysel.sqlite.exec>) {
  return measured("exec", args[0], () => tysel.sqlite.exec(...args));
}
export function measuredQuery(...args: Parameters<typeof tysel.sqlite.query>) {
  return measured("query", args[0], () => tysel.sqlite.query(...args));
}
'''


def instrument(source, output):
    shutil.copytree(source, output)
    for path in output.glob("*.ts"):
        text = path.read_text()
        if "tysel.sqlite." in text:
            text = 'import { measuredExec, measuredQuery } from "./p51-profile.js";\n' + text
            path.write_text(text.replace("tysel.sqlite.exec(", "measuredExec(").replace("tysel.sqlite.query(", "measuredQuery("))
    index = output / "index.ts"
    text = index.read_text().replace('export default {', 'const app = {')
    text = text.replace('error: error instanceof Failure ? error.code : "STORAGE_UNAVAILABLE"',
                        'error: error instanceof Failure ? error.code : "STORAGE_UNAVAILABLE", profileError: String(error)')
    index.write_text('import { takeProfile } from "./p51-profile.js";\n' + text + '''
export default {
  ...app,
  async fetch(request: Request) {
    takeProfile();
    const response = await app.fetch(request);
    response.headers.set("x-p51-profile", JSON.stringify(takeProfile()));
    return response;
  },
} satisfies TyselApp;
''')
    (output / "p51-profile.ts").write_text(INSTRUMENTATION)


def main():
    bench.require_assertions()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--state-root", type=Path, help="Separate state-filesystem diagnostic only; never budget acceptance")
    args = parser.parse_args()
    output, binary_dir = args.output.resolve(), args.bin_dir.resolve()
    assert not output.exists(), "preserve earlier diagnostics"
    output.mkdir(parents=True)
    releases = output / "releases"
    protocol = json.loads((HERE / "protocol.json").read_text())
    support, primary = None, None
    cleanup_errors = []
    raw, lock = [], threading.Lock()
    original_http = bench.deploy.http
    failures, profiles = [], []
    def observed_http(origin, path, method="GET", body=None, token=None, key=None, timeout=45):
        headers = {"Content-Type": "application/json"}
        if token: headers["Authorization"] = "Bearer " + token
        if key: headers["Idempotency-Key"] = key
        request = urllib.request.Request(origin + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
        try:
            response = urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=timeout)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            profile = response.headers.get("x-p51-profile")
            if profile:
                with lock: profiles.append(dict(path="/jobs/:id" if path.startswith("/jobs/") else path, method=method,
                                                events=json.loads(profile)))
            result = response.status, json.loads(response.read())
        if result[0] >= 500: failures.append(result)
        return result
    class Capture(list):
        def append(self, line):
            with lock: raw.append(line)
            super().append(line)
    original_process = bench.deploy.Process
    class Process(original_process):
        def __init__(self, *args, **kwargs):
            super().__init__(*args, **kwargs)
            with lock: raw.extend(self.logs)
            self.logs = Capture(self.logs)
    result = dict(kind="instrumented diagnostic, not budget acceptance", status="running",
                  rounds=[], startedAtUnixMs=time.time_ns() // 1000000)
    try:
        support = bench.Support(binary_dir / "tysel-bench-agent-support")
        result.update(protocolSha256=bench.sha(HERE / "protocol.json"),
                      source={p.name: bench.sha(p) for p in args.source.glob("*.ts")}, instrumentation=INSTRUMENTATION)
        bench.deploy.http = observed_http
        bench.deploy.Process = Process
        with tempfile.TemporaryDirectory(prefix="p51-profile-source-") as temp, ExitStack() as stack:
            state_root = Path(stack.enter_context(tempfile.TemporaryDirectory(prefix="p51-state-", dir=args.state_root))) if args.state_root else output
            result["stateRoot"] = str(state_root)
            result["storageScope"] = "separate filesystem diagnostic" if args.state_root else "same bind-mounted output as P5"
            source = Path(temp) / "src"
            instrument(args.source, source)
            result["package"] = bench.package.package(binary_dir, releases / "lookup", caller_source=source)
            for i in range(protocol["rounds"]):
                result["rounds"].append(bench.warm_round(releases, support, protocol, state_root / f"state-{i}",
                                                       "lookup", protocol["sizes"][1], 4, 0, i))
                print(f"diagnostic round {i+1} complete", flush=True)
        events = [event for profile in profiles for event in profile["events"]]
        assert events and all(e["ms"] >= 0 for e in events), "missing SQL observations or clock reversal"
        result["sqlEvents"] = events
        result["httpProfiles"] = profiles
        result["sqlSummary"] = {category: dict(calls=sum(e["category"] == category for e in events),
                                               summedElapsedMs=sum(e["ms"] for e in events if e["category"] == category))
                                for category in ("schema", "maintenance", "business")}
        result["latency"] = support.call(op="stats", series={f"round{r['round']}": [s["e2eMs"] for s in r["samples"]]
                                                            for r in result["rounds"]})
    except BaseException as error:
        primary = error
        result["status"] = "error"
        result["error"] = traceback.format_exc()
    finally:
        bench.deploy.Process = original_process
        bench.deploy.http = original_http
        result["httpFailures"] = failures
        result["processCleanup"] = support.cleanup if support is not None else []
        # Finish the log before JSON so a log-write failure cannot leave a
        # completed diagnostic. Every teardown still gets an independent try.
        for label, action in (
            ("support close", lambda: support.close() if support is not None else None),
            ("process.log write", lambda: (output / "process.log").write_text("".join(raw))),
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
            bench.save(output / "profile.json", result)
        except BaseException as error:
            cleanup_errors.append((error, "final evidence save failed:\n" + traceback.format_exc()))
    if cleanup_errors:
        raise RuntimeError("caller diagnostic failed; final evidence may be incomplete:\n" +
                           "\n".join(detail for _, detail in cleanup_errors)) from (primary or cleanup_errors[0][0])
    if primary is not None:
        raise primary.with_traceback(primary.__traceback__)
    print(json.dumps(result["sqlSummary"]), flush=True)


if __name__ == "__main__":
    main()
