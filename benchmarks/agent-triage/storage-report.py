#!/usr/bin/env python3
"""Independently validate and render a complete P5.2 storage A/B diagnostic."""
import argparse
from collections import Counter
import gzip
import hashlib
import json
import math
from pathlib import Path
import statistics

SOURCES = ("baseline", "candidate")
SIZES = ("small", "bounded")
LAYOUTS = {
    "all-bind": [0, 0, 0],
    "config-volume": [1, 0, 0],
    "jobs-volume": [0, 1, 0],
    "durable-volume": [0, 0, 1],
    "all-volume": [1, 1, 1],
}
ORIGINAL_PROTOCOL = "62562d59185b04282aa10b15c81e981a883b27200cb23d532c10107e1d651fc1"


def require(condition, message):
    if not condition:
        raise ValueError(message)


def number(value, label, positive=False):
    require(type(value) in (int, float) and math.isfinite(value), label + " is not finite")
    require(value > 0 if positive else value >= 0, label + " is out of range")
    return value


def same_number(actual, expected, label):
    number(actual, label)
    require(math.isclose(actual, expected, rel_tol=1e-10, abs_tol=1e-9), label + " does not recompute")


def percentile95(samples):
    # Same nearest-rank-index convention as tysel-bench-compare::percentile.
    values = sorted(samples)
    return values[math.floor((len(values) - 1) * .95 + .5)]


def extent(values):
    return dict(min=min(values), max=max(values))


def validate_observation(observation, row, mounts):
    sequence = row["sequence"]
    require(Path(observation["namespace"]).name == "namespace-" + str(sequence), "observation order mismatch")
    require(observation["placement"] == LAYOUTS[row["layout"]], "observation placement mismatch")
    for kind, on_volume in zip(("config", "jobs", "durable"), observation["placement"]):
        path = observation["paths"][kind]
        filesystems = path["filesystem"]["filesystems"]
        require(len(filesystems) == 1, "ambiguous containing mount")
        mount = filesystems[0]
        signature = dict(device=path["device"], target=mount["target"], source=mount["source"],
                         filesystem=mount["fstype"], options=mount["options"])
        storage_class = "volume" if on_volume else "bind"
        if storage_class in mounts:
            require(mounts[storage_class]["mount"] == signature, "storage mount changed during diagnostic")
        else:
            mounts[storage_class] = dict(mount=signature, exampleRealPath=path["realpath"])
        require(Path(path["realpath"]).is_absolute(), "storage realpath is not absolute")
    for kind, filename in (("jobs", "jobs.db"), ("durable", "durable-events.db")):
        expected = str(Path(observation["paths"][kind]["realpath"]) / filename)
        require(expected in observation["openDatabaseFiles"], "caller did not open the declared database")
        for phase in ("databases", "postRunDatabases"):
            database = observation[phase][kind]
            require(database["path"] == expected, "database observer path mismatch")
            require([item[2] for item in database["databaseList"] if item[1] == "main"] == [expected],
                    "SQLite main database path mismatch")
            require(database["journalMode"] == "delete" and database["integrity"] == "ok",
                    "database journal/integrity check failed")
    tail = observation["nativeCapabilityTail"]
    require("200" in tail["scope"] and "not whole-round" in tail["scope"], "log-tail scope missing")
    require(sum(item["calls"] for item in tail["summary"].values()) <= 200, "log tail exceeds its scope")


def summarize(data):
    require(data["status"] == "complete", "incomplete/error diagnostics cannot support selection")
    require(not data.get("error"), "diagnostic contains an error")
    plan = data["plan"]
    require(plan["originalProtocolSha256"] == ORIGINAL_PROTOCOL, "unexpected original protocol")
    require(plan["pairedBlocks"] == 4 and plan["sizes"] == list(SIZES), "unexpected blocks or payloads")
    require(plan["concurrency"] == 4 and plan["adapterDelayMs"] == 0, "unexpected diagnostic load")
    require(plan["rounds"] == 80 and plan["measuredJobs"] == 2560 and plan["warmups"] == 640,
            "unexpected declared sample counts")
    require(plan["layouts"] == [dict(name=name, volume=placement) for name, placement in LAYOUTS.items()],
            "unexpected storage layout plan")
    require({source: data["packages"][source]["artifacts"]["caller"]["sha256"] for source in SOURCES}
            == plan["callerArtifactSha256"], "caller packages differ from predeclaration")
    rounds = data["rounds"]
    require(len(rounds) == len(data["observations"]) == len(data["processCleanup"]) == 80,
            "expected 80 rounds, observations and cleanup checks")
    require([row["sequence"] for row in rounds] == list(range(80)), "missing/duplicate/out-of-order rounds")
    indexed, mounts = {}, {}
    for row, observation, cleanup in zip(rounds, data["observations"], data["processCleanup"]):
        block, size, layout, source = row["round"], row["size"], row["layout"], row["sourceVariant"]
        require(type(block) is int and 0 <= block < 4 and size in SIZES and layout in LAYOUTS
                and source in SOURCES, "unknown diagnostic cell")
        key = (block, size, layout, source)
        require(key not in indexed, "duplicate diagnostic cell")
        indexed[key] = row
        require(row["pair"] == f"b{block}-{layout}-{size}", "pair identity mismatch")
        require(row["variant"] == "lookup" and row["concurrency"] == 4 and row["adapterDelayMs"] == 0,
                "round changed the declared workload")
        require(row["errors"] == row["unexpectedDenials"] == 0, "round reported a correctness error")
        require(len(row["samples"]) == 32 and row["physicalReadsIncludingWarmup"] == 40,
                "round lacks 32 measurements plus 8 warmup reads")
        require(Counter(sample["client"] for sample in row["samples"]) == {client: 8 for client in range(4)},
                "round client allocation mismatch")
        require(len({sample["jobId"] for sample in row["samples"]}) == 32, "duplicate measured job")
        for sample in row["samples"]:
            number(sample["e2eMs"], "job latency", positive=True)
            require(sample["actualSleepMs"] == 0, "unexpected artificial adapter delay")
            attempts = sample["adapterAttempts"]
            require(len(attempts) == 1 and attempts[0]["authorized"] is True,
                    "measured job lacks exactly one authorized read")
            client = sample["client"]
            require(attempts[0]["customerId"] == f"bench-c{client}"
                    and attempts[0]["ticketId"] == sample["adapter"]["ticketId"] == f"t{client}-00",
                    "adapter observation does not match its client")
            audit = sample["audit"]
            require(len(audit) == 8 and all(event["job_id"] == sample["jobId"]
                    and event["decision"] != "deny" for event in audit), "audit contract mismatch")
            require({event["event_key"] for event in audit} == {
                "admission", "terminal", "reserve:select:1", "outcome:select:1",
                "reserve:read:1", "outcome:read:1", "reserve:summarize:1", "outcome:summarize:1"},
                "audit events missing or duplicated")
            require(any(event["event_key"] == "terminal" and event["outcome"] == "SUCCEEDED"
                        for event in audit), "job did not succeed")
        duration = number(row["durationMs"], "round duration", positive=True)
        same_number(row["jobsPerSec"], 32000 / duration, "round throughput")
        same_number(row["p95Ms"], percentile95([sample["e2eMs"] for sample in row["samples"]]), "round p95")
        require(cleanup["crash"] is False and cleanup["liveProcessesRemaining"] == 0
                and not cleanup.get("error") and len(cleanup["pids"]) >= 3, "process cleanup failed")
        validate_observation(observation, row, mounts)
    require(set(mounts) == {"bind", "volume"} and mounts["bind"]["mount"] != mounts["volume"]["mount"],
            "bind and volume storage were not distinguished")

    pairs, cells = [], []
    for layout in LAYOUTS:
        for size in SIZES:
            cell_pairs = []
            for block in range(4):
                before, after = (indexed[block, size, layout, source] for source in SOURCES)
                require(abs(before["sequence"] - after["sequence"]) == 1, "source pair was not adjacent")
                order = "AB" if before["sequence"] < after["sequence"] else "BA"
                expected = "AB" if (block + list(LAYOUTS).index(layout) + list(SIZES).index(size)) % 2 == 0 else "BA"
                require(order == expected, "source order differs from the predeclared schedule")
                pair = dict(pair=before["pair"], block=block, layout=layout, size=size, order=order,
                            throughputRatio=after["jobsPerSec"] / before["jobsPerSec"],
                            p95Ratio=after["p95Ms"] / before["p95Ms"],
                            baselineJobsPerSec=before["jobsPerSec"], candidateJobsPerSec=after["jobsPerSec"],
                            baselineP95Ms=before["p95Ms"], candidateP95Ms=after["p95Ms"])
                cell_pairs.append(pair)
            require(Counter(pair["order"] for pair in cell_pairs) == {"AB": 2, "BA": 2}, "unbalanced A/B order")
            pairs.extend(cell_pairs)
            cells.append(dict(layout=layout, size=size, pairs=cell_pairs,
                              baselineThroughputRange=extent([p["baselineJobsPerSec"] for p in cell_pairs]),
                              candidateThroughputRange=extent([p["candidateJobsPerSec"] for p in cell_pairs]),
                              baselineP95RangeMs=extent([p["baselineP95Ms"] for p in cell_pairs]),
                              candidateP95RangeMs=extent([p["candidateP95Ms"] for p in cell_pairs])))
    require(len(pairs) == 40 and len({pair["pair"] for pair in pairs}) == 40, "expected exactly 40 source pairs")
    raw_pairs = data["comparison"]["pairs"]
    require(len(raw_pairs) == 40 and len({pair["pair"] for pair in raw_pairs}) == 40, "raw pair summary mismatch")
    for pair in pairs:
        expected = next(item for item in raw_pairs if item["pair"] == pair["pair"])
        require(expected["layout"] == pair["layout"] and expected["size"] == pair["size"], "raw pair cell mismatch")
        for metric in ("throughputRatio", "p95Ratio"):
            same_number(expected[metric], pair[metric], "raw paired " + metric)

    decisions = []
    for size in SIZES:
        selected = [pair for pair in pairs if pair["layout"] == "all-volume" and pair["size"] == size]
        ratios = [pair["throughputRatio"] for pair in selected]
        rate, latency = statistics.median(ratios), statistics.median(pair["p95Ratio"] for pair in selected)
        decisions.append(dict(size=size, pairedThroughputRatios=ratios, medianThroughputRatio=rate,
                              medianP95Ratio=latency, passed=all(ratio >= 1 for ratio in ratios) and rate >= 1.05 and latency <= 1))
    require(data["comparison"]["targetDecision"] == decisions, "raw selection decisions do not recompute")
    retain = all(item["passed"] for item in decisions)
    require(data["comparison"]["retainCandidate"] is retain, "raw retain/revert decision does not recompute")

    storage_contrasts = []
    for source in SOURCES:
        for layout in LAYOUTS:
            for size in SIZES:
                ratios = [dict(block=block, throughputRatio=indexed[block, size, layout, source]["jobsPerSec"] /
                               indexed[block, size, "all-bind", source]["jobsPerSec"])
                          for block in range(4)]
                storage_contrasts.append(dict(source=source, layout=layout, size=size, byBlock=ratios,
                                             medianThroughputRatio=statistics.median(item["throughputRatio"] for item in ratios)))
    return dict(status="validated_complete_diagnostic", kind="Not cost acceptance", system=data["system"],
                planSha256=data["planSha256"], driverSha256=data["driverSha256"],
                originalProtocolSha256=plan["originalProtocolSha256"], callerArtifactSha256=plan["callerArtifactSha256"],
                counts=dict(rounds=80, sourcePairs=40, cells=10, measuredJobs=2560, warmupJobs=640,
                            physicalReadsIncludingWarmup=3200, measuredAuthorizedReadsRechecked=2560, cleanupChecks=80),
                warmupEvidence="Eight warmup reads per round are inferred from the retained count of 40 and 32 measured reads; "
                               "warmup authorization was asserted by the successful driver, whose individual warmup read records are not retained.",
                cells=cells, targetDecision=decisions, retainCandidate=retain, selectionRule=plan["selection"],
                storageContrasts=storage_contrasts, actualStorage=mounts,
                limitations=["Four A/B pairs per cell are the repeats; individual jobs are not independent experiment repetitions.",
                             "Storage contrasts match block, payload and source, but layouts were not simultaneous or adjacent pairs. "
                             "They are conditional descriptive comparisons, not additive effects or a factorial interaction analysis.",
                             "The 250 ms outbox and closed-loop status observer can mask smaller code benefits.",
                             "Capability summaries cover the last 200 native log lines only, not whole-round calls or time; they are not aggregated here.",
                             "SQLite synchronous observations belong to read-only verifier connections, not the native writers.",
                             "Split layouts are diagnostics, not a supported backup topology. This run does not replace full cost, recovery or deployment acceptance."])


def markdown(summary):
    def interval(value):
        return f"{value['min']:.2f}–{value['max']:.2f}"

    def ratios(values):
        return " / ".join(f"{value:.4f}" for value in values)

    choice = "retain candidate" if summary["retainCandidate"] else "restore P5 baseline"
    lines = ["# P5.2 paired storage diagnostic", "", f"Predeclared selection: **{choice}**. This is **not cost acceptance**.", "",
             "Validated 80 rounds, 40 adjacent source pairs and 10 cells. Each cell contains two AB and two BA pairs; "
             "A is P5 and B is P5.1. Ratios below are B/A in block order 0–3.", "",
             "| Layout / payload | A jobs/s range | B jobs/s range | A round p95 ms range | B round p95 ms range | Paired throughput ratios | Paired p95 ratios |",
             "| --- | ---: | ---: | ---: | ---: | --- | --- |"]
    for cell in summary["cells"]:
        lines.append(f"| {cell['layout']}/{cell['size']} | {interval(cell['baselineThroughputRange'])} | "
                     f"{interval(cell['candidateThroughputRange'])} | {interval(cell['baselineP95RangeMs'])} | "
                     f"{interval(cell['candidateP95RangeMs'])} | {ratios([p['throughputRatio'] for p in cell['pairs']])} | "
                     f"{ratios([p['p95Ratio'] for p in cell['pairs']])} |")
    lines += ["", "## Predeclared selection", "", summary["selectionRule"], "",
              "| All-volume payload | Nondecreasing throughput pairs | Median throughput ratio | Median p95 ratio | Selection gate |",
              "| --- | ---: | ---: | ---: | --- |"]
    for decision in summary["targetDecision"]:
        lines.append(f"| {decision['size']} | {sum(r >= 1 for r in decision['pairedThroughputRatios'])}/4 | "
                     f"{decision['medianThroughputRatio']:.4f} | {decision['medianP95Ratio']:.4f} | "
                     f"{'pass' if decision['passed'] else 'not met'} |")
    lines += ["", "The selection was independently recomputed and matches the raw result. Correctness assertions in this "
              "diagnostic do not replace the separate acceptance suite.", "", "## Storage comparisons within each source", "",
              "Each value compares a layout with all-bind for the same source, payload and block. These are conditional "
              "comparisons from different times, not additive storage contributions. The all-bind row is 1 by definition.", "",
              "| Source | Layout / payload | Throughput ratios to all-bind, blocks 0–3 | Median ratio |",
              "| --- | --- | --- | ---: |"]
    for contrast in summary["storageContrasts"]:
        lines.append(f"| {contrast['source']} | {contrast['layout']}/{contrast['size']} | "
                     f"{ratios([r['throughputRatio'] for r in contrast['byBlock']])} | {contrast['medianThroughputRatio']:.4f} |")
    lines += ["", "## Verified storage and evidence limits", "",
              "Caller open file descriptors match the declared application and Durable database paths. Both databases "
              "use DELETE journal mode and pass integrity checks before and after every round.", "",
              "| Placement | Filesystem | Device | Mount target | Mount source |", "| --- | --- | ---: | --- | --- |"]
    for name, storage in summary["actualStorage"].items():
        mount = storage["mount"]
        lines.append(f"| {name} | {mount['filesystem']} | {mount['device']} | `{mount['target']}` | `{mount['source']}` |")
    lines += ["", "The raw archive retains the actual paths, mount options and full mountinfo; summary.json retains representative mount metadata.", "",
              "Verified 2,560 measured jobs and their single authorized reads, 640 warmup reads by retained counts, "
              "3,200 total reads, and 80 successful process cleanups.", "", summary["warmupEvidence"], ""]
    lines.extend("- " + limitation for limitation in summary["limitations"])
    lines.append("")
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    raw = args.input.read_bytes()
    unpacked = gzip.decompress(raw) if args.input.suffix == ".gz" else raw
    summary = summarize(json.loads(unpacked))
    summary["inputJsonSha256"] = hashlib.sha256(unpacked).hexdigest()
    summary["reporterSha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    (args.output / "results.md").write_text(markdown(summary))


if __name__ == "__main__":
    main()
