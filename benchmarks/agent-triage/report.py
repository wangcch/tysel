#!/usr/bin/env python3
"""Render retained P5 measurements without rerunning or modifying their budgets."""
import argparse
import gzip
import importlib.util
import json
from pathlib import Path
import sys
import zlib

# Also support callers that load report.py directly with spec_from_file_location.
_spec = importlib.util.spec_from_file_location("agent_triage_evidence_check", Path(__file__).with_name("evidence_check.py"))
_checker = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_checker)
load_json = _checker.load_json


def summarize(data):
    _checker.validate(data)
    protocol, analysis = data["protocol"], data["analysis"]
    dist = analysis["distributions"]
    cells = []
    for variant in protocol["variants"]:
        for size in protocol["sizes"]:
            for concurrency in protocol["concurrency"]:
                for delay in protocol["adapterDelayMs"] if variant == "lookup" else [0]:
                    name = f"{variant}/{size['name']}/c{concurrency}/delay{delay}"
                    rows = [r for r in data["warm"] if (r["variant"], r["size"], r["concurrency"], r["adapterDelayMs"]) ==
                            (variant, size["name"], concurrency, delay)]
                    samples = [s for r in rows for s in r["samples"]]
                    def metric(key):
                        d = dist[name + "/" + key]
                        return dict(p50=d["p50"], p95=d["p95"])
                    checks = [c for c in analysis["checks"] if c["cell"] == name]
                    cells.append(dict(cell=name, variant=variant, size=size["name"], concurrency=concurrency,
                                      adapterDelayMs=delay, samples=len(samples), latencyMs=metric("e2eMs"),
                                      excludingAdapterSleepMs=metric("excludingAdapterSleepMs"),
                                      actualSleepMs=metric("actualSleepMs"),
                                      stagesMs={s: metric("stage/" + s) for s in samples[0]["stagesMs"]},
                                      throughputJobsPerSec=[r["jobsPerSec"] for r in rows],
                                      idlePssKiB=[r["idlePssKiB"] for r in rows],
                                      peakPssKiB=max(r["peakPssKiB"] for r in rows),
                                      processCounts=sorted({len(m["pids"]) for r in rows for m in r["memory"]}),
                                      requestBytes=sorted({s["requestBytes"] for s in samples}),
                                      canonicalResponseBytes=sorted({s["responseJsonBytes"] for s in samples}),
                                      resultBytes=sorted({s["resultBytes"] for s in samples}),
                                      adapterAttempts=sum(len(s["adapterAttempts"]) for s in samples),
                                      passed=all(c["passed"] for c in checks)))
    return dict(status=data["status"], protocolSha256=data["protocolSha256"], system=data["system"], kernel=data["kernel"],
                sourceCommit=data["sourceCommit"], workspaceDirty=data["workspaceDirty"], cells=cells,
                cold={v: {k: dist[v + "/coldTotalMs"][k] for k in ("p50", "p95")} for v in protocol["variants"]},
                recoveryMs=[r["recoveryMs"] for r in data["recovery"]], denials=data["denials"],
                warmMeasuredJobs=sum(len(r["samples"]) for r in data["warm"]),
                warmupJobs=len(data["warm"]) * protocol["warmupRequests"], coldSamples=len(data["cold"]),
                warmPhysicalReadsIncludingWarmup=sum(r["physicalReadsIncludingWarmup"] for r in data["warm"]),
                cleanupChecks=len(data["processCleanup"]), numericalChecks=len(analysis["checks"]),
                budgetMisses=[c for c in analysis["checks"] if not c["passed"]])


def markdown(summary):
    lines = ["# P5 cost results", "", f"Status: **{summary['status']}**. Linux ARM64 release on the named local VM.", "",
             "These are descriptive local measurements, not production SLOs or a cross-runtime ranking.", "",
             "| Variant / size / clients / adapter delay | p50 ms | p95 ms | Jobs/s, range of 3 rounds | Sampled peak PSS MiB | Budget |",
             "| --- | ---: | ---: | ---: | ---: | --- |"]
    for cell in summary["cells"]:
        rate = cell["throughputJobsPerSec"]
        latency = cell["latencyMs"]
        lines.append(f"| {cell['cell']} | {latency['p50']:.2f} | {latency['p95']:.2f} | {min(rate):.2f}–{max(rate):.2f} | {cell['peakPssKiB']/1024:.2f} | {'pass' if cell['passed'] else 'MISS'} |")
    lines += ["", "Cold process-to-first-result (20 samples each, warm filesystem caches):", "",
              "| Variant | p50 ms | p95 ms |", "| --- | ---: | ---: |"]
    for variant, latency in summary["cold"].items():
        lines.append(f"| {variant} | {latency['p50']:.2f} | {latency['p95']:.2f} |")
    lines += ["", "Lookup stages below are **p50 / p95 milliseconds** from transactional audit timestamps,",
              "with millisecond resolution. Stage percentiles do not sum to end-to-end percentiles.", "",
              "| Cell | Queue → select | Select | Read | Summarize | Between stages | Terminal commit | Actual adapter sleep | E2E excluding sleep |",
              "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    def pair(value):
        return f"{value['p50']:.2f} / {value['p95']:.2f}"
    for cell in summary["cells"]:
        if cell["variant"] == "lookup":
            stages = cell["stagesMs"]
            values = [stages[s] for s in ("queueToSelect", "select", "read", "summarize", "betweenStages", "terminalCommit")]
            values += [cell["actualSleepMs"], cell["excludingAdapterSleepMs"]]
            lines.append("| " + cell["cell"] + " | " + " | ".join(pair(v) for v in values) + " |")
    lines += ["", "Default-lease recovery samples (ms): " + ", ".join(f"{v:.2f}" for v in summary["recoveryMs"]) + ".",
              "Three samples support the maximum-budget check, not a p95 estimate.", "",
              f"Measured warm jobs: {summary['warmMeasuredJobs']}; warmups: {summary['warmupJobs']}; cold samples: {summary['coldSamples']}.",
              f"Warm adapter reads including warmups: {summary['warmPhysicalReadsIncludingWarmup']}. Two expected capability denials; no adapter access from probes.",
              f"Numerical checks: {summary['numericalChecks']}; misses: {len(summary['budgetMisses'])}; process cleanup checks: {summary['cleanupChecks']}.", ""]
    if summary["budgetMisses"]:
        lines += ["Budget misses:", "", "```json", json.dumps(summary["budgetMisses"], indent=2), "```", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        data = load_json(gzip.decompress(args.input.read_bytes()) if args.input.suffix == ".gz" else args.input.read_bytes())
        summary = summarize(data)
        rendered = markdown(summary)
        args.output.mkdir(parents=True, exist_ok=True)
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        (args.output / "results.md").write_text(rendered)
    except (ValueError, OSError, EOFError, zlib.error) as error:
        print("invalid cost evidence: " + str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
