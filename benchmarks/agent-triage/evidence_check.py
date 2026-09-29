"""Offline consistency checks for the frozen P5 cost experiment.

This reconstructs decisions from retained observations. It cannot authenticate
those observations or replace the live driver's output and cleanup assertions.
"""
from collections import Counter
import hashlib
import json
import math
from pathlib import Path

ORIGINAL_PROTOCOL_SHA256 = "62562d59185b04282aa10b15c81e981a883b27200cb23d532c10107e1d651fc1"
STEPS = ("select", "read", "summarize")
PROCESSES = {"direct": 1, "snapshot": 2, "lookup": 3}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_json(raw):
    """Do not silently discard duplicate object keys in retained evidence."""
    def object_pairs(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate JSON object key: " + key)
            result[key] = value
        return result
    return json.loads(raw, object_pairs_hook=object_pairs)


def number(value, label, positive=False, integer=False):
    require(type(value) in (int, float) and math.isfinite(value), label + " must be finite")
    require(value > 0 if positive else value >= 0, label + " is out of range")
    require(not integer or type(value) is int, label + " must be an integer")
    return value


def same_number(actual, expected, label):
    number(actual, label)
    require(math.isclose(actual, expected, rel_tol=1e-10, abs_tol=1e-9), label + " does not recompute")


def finite_tree(value):
    if type(value) is float:
        require(math.isfinite(value), "evidence contains a non-finite number")
    elif isinstance(value, dict):
        for item in value.values():
            finite_tree(item)
    elif isinstance(value, list):
        for item in value:
            finite_tree(item)


def percentile(values, quantile):
    # Rust f64::round rounds positive halves upward; Python round does not.
    ordered = sorted(values)
    return ordered[math.floor((len(ordered) - 1) * quantile + .5)]


def protocol_check(data):
    raw = Path(__file__).with_name("protocol.json").read_bytes()
    require(hashlib.sha256(raw).hexdigest() == ORIGINAL_PROTOCOL_SHA256, "frozen protocol file changed")
    original = json.loads(raw)
    protocol = data["protocol"]
    require(isinstance(protocol["environment"]["storage"], str)
            and protocol["environment"]["storage"].strip(), "storage description missing")
    expected = {**original, "environment": {**original["environment"],
                                           "storage": protocol["environment"]["storage"]}}
    require(json.dumps(protocol, sort_keys=True) == json.dumps(expected, sort_keys=True),
            "only environment.storage may differ from the frozen protocol")
    # The retained driver protocols use this exact JSON encoding, including LF.
    digest = hashlib.sha256((json.dumps(protocol, indent=2) + "\n").encode()).hexdigest()
    require(data["protocolSha256"] == digest, "embedded protocol/hash mismatch")
    return protocol


def events_check(events, job, recovery=False):
    wanted = {"admission", "terminal"} | {
        f"{kind}:{step}:1" for kind in ("reserve", "outcome") for step in STEPS}
    if recovery:
        wanted |= {"reserve:read:2", "outcome:read:2"}
    require(len(events) == len(wanted), "audit event count mismatch")
    by_key = {event["event_key"]: event for event in events}
    require(set(by_key) == wanted, "audit events missing or duplicated")
    require(all(event["job_id"] == job for event in events), "audit job identity mismatch")
    for key, event in by_key.items():
        number(event["at_ms"], "audit time", integer=True)
        if key == "admission":
            decision, outcome = "admit", "ACCEPTED"
        elif key == "terminal":
            decision, outcome = "terminal", "SUCCEEDED"
        elif key.startswith("reserve:"):
            decision, outcome = "reserve", "PENDING"
        elif recovery and key == "outcome:read:1":
            decision, outcome = "deny", "UNKNOWN_OUTCOME"
        else:
            decision, outcome = "accept", "OK"
        require((event["decision"], event["outcome"]) == (decision, outcome), "audit outcome mismatch")
        step = key if key in ("admission", "terminal") else key.split(":")[1]
        operation = {"admission": "job.create", "terminal": "job.complete", "read": "ticket.read",
                     "select": "plugin.select", "summarize": "plugin.summarize"}[step]
        require(event["step"] == step and event["operation"] == operation, "audit operation mismatch")
        if step in ("admission", "terminal"):
            require(event["ordinal"] == 0 and event["attempt_id"] is None, "audit boundary identity mismatch")
    previous = by_key["admission"]["at_ms"]
    attempts = set()
    for step in STEPS:
        for ordinal in range(1, 3 if recovery and step == "read" else 2):
            reserve, outcome = (by_key[f"{kind}:{step}:{ordinal}"] for kind in ("reserve", "outcome"))
            require(reserve["at_ms"] >= previous and outcome["at_ms"] >= reserve["at_ms"],
                    "audit time moved backwards")
            require(reserve["attempt_id"] and reserve["attempt_id"] == outcome["attempt_id"],
                    "audit attempt identity mismatch")
            require(reserve["attempt_id"] not in attempts, "audit attempt reused for another step")
            attempts.add(reserve["attempt_id"])
            require(reserve["ordinal"] == outcome["ordinal"] == ordinal
                    and reserve["step"] == outcome["step"] == step, "audit step mismatch")
            previous = outcome["at_ms"]
    require(by_key["terminal"]["at_ms"] >= previous, "terminal audit time moved backwards")
    return by_key


def stages_check(sample):
    events = events_check(sample["audit"], sample["jobId"])
    previous, gaps, expected = events["admission"]["at_ms"], 0, {}
    for step in STEPS:
        reserve, outcome = (events[f"{kind}:{step}:1"]["at_ms"] for kind in ("reserve", "outcome"))
        if step == "select":
            expected["queueToSelect"] = reserve - previous
        else:
            gaps += reserve - previous
        expected[step] = outcome - reserve
        previous = outcome
    expected.update(betweenStages=gaps, terminalCommit=events["terminal"]["at_ms"] - previous,
                    persistedTotal=events["terminal"]["at_ms"] - events["admission"]["at_ms"])
    require(sample["stagesMs"] == expected, "audit stages do not recompute")


def read_check(read, client):
    require(read["authorized"] is True and read["customerId"] == f"bench-c{client}"
            and read["ticketId"] == f"t{client}-00", "read authorization or client identity mismatch")
    number(read["at"], "read time", positive=True)


def sample_check(sample, variant, size, delay, jobs, *, clients=4):
    client = number(sample["client"], "client", integer=True)
    require(client < clients, "unknown client")
    elapsed = number(sample["e2eMs"], "latency", positive=True)
    slept = number(sample["actualSleepMs"], "adapter sleep")
    same_number(sample["excludingAdapterSleepMs"], elapsed - slept, "elapsed excluding adapter sleep")
    for key in ("requestBytes", "responseJsonBytes", "resultBytes"):
        number(sample[key], key, positive=True, integer=True)
    expected_result = dict(customerId=f"bench-c{client}", summary=f"Prioritize t{client}-00: " + "s" * size["subjectChars"])
    require(sample["resultBytes"] == len(json.dumps(expected_result, separators=(",", ":")).encode()),
            "result size differs from the declared payload")
    if variant != "lookup":
        body = dict(protocolVersion=1, jobId="bench-job-000000000000000000000000", stepId="2",
                    attemptId="bench-attempt-000000000000000000000", payload=dict(
                        customerId=f"bench-c{client}", tickets=[dict(id=f"t{client}-{i:02}", priority=3 if i == 0 else 0)
                        for i in range(size["tickets"])], detail=dict(ticketId=f"t{client}-00", subject="s" * size["subjectChars"])))
        require(sample["requestBytes"] == len(json.dumps(body).encode()), "transform request size mismatch")
        response = {**body, "payload": {"kind": "done", **expected_result}}
        require(sample["responseJsonBytes"] == len(json.dumps(response, separators=(",", ":")).encode()),
                "transform response size mismatch")
        require(sample["adapterAttempts"] == [] and slept == 0, "pure transform made an adapter read")
        require(set(sample["stagesMs"]) == {"httpTransform"}, "unexpected transform stages")
        same_number(sample["stagesMs"]["httpTransform"], elapsed, "HTTP transform time")
        return
    job = sample["jobId"]
    require(isinstance(job, str) and job and job not in jobs, "duplicate or missing job identity")
    jobs.add(job)
    body = dict(protocolVersion=1, ticketIds=[f"t{client}-{i:02}" for i in range(size["tickets"])])
    require(sample["requestBytes"] == len(json.dumps(body).encode()), "lookup request size mismatch")
    require(len(sample["adapterAttempts"]) == 1, "healthy lookup must have exactly one read")
    read_check(sample["adapterAttempts"][0], client)
    adapter = sample["adapter"]
    require(adapter["ticketId"] == f"t{client}-00", "adapter identity mismatch")
    require(adapter["end"] >= adapter["start"] >= sample["adapterAttempts"][0]["at"],
            "adapter timestamps out of order")
    same_number(adapter["actualSleepMs"], slept, "retained adapter sleep")
    same_number(slept, (adapter["end"] - adapter["start"]) * 1000 if delay else 0, "adapter sleep interval")
    require(slept >= delay, "adapter delay was shorter than the protocol")
    require(0 < number(sample["admissionMs"], "admission time") <= elapsed, "admission exceeds job latency")
    stages_check(sample)


def memory_check(row):
    memory = row["memory"]
    require(len(memory) >= 2, "initial/final memory observations missing")
    previous = -1
    for sample in memory:
        require(sample["kind"] == "pss", "memory scope must be Linux PSS")
        pids = sample["pids"]
        require(len(set(pids)) == len(pids) >= PROCESSES[row["variant"]], "memory process coverage mismatch")
        for pid in pids:
            number(pid, "memory PID", positive=True, integer=True)
        number(sample["valueKiB"], "PSS", positive=True, integer=True)
        at = number(sample["atMs"], "memory sample time")
        require(at >= previous, "memory samples out of order")
        previous = at
    same_number(row["idlePssKiB"], memory[0]["valueKiB"], "idle PSS")
    same_number(row["peakPssKiB"], max(item["valueKiB"] for item in memory), "peak PSS")


def recovery_check(rows, protocol, jobs):
    require(len(rows) == protocol["recoverySamples"]
            and {row["sample"] for row in rows} == set(range(protocol["recoverySamples"])),
            "recovery samples missing or duplicated")
    for row in rows:
        elapsed = number(row["recoveryMs"], "recovery time", positive=True)
        require(row["defaultLeaseMs"] == 45000 and row["originalDeadlinePreserved"] is True,
                "recovery lease/deadline mismatch")
        remaining = number(row["remainingLeaseAtCrashMs"], "remaining lease")
        require(0 < remaining <= 45000, "invalid remaining lease")
        # This same-VM experiment assumes normal wall/monotonic clock progress.
        # The adjacent crash/remaining-lease observations allow 1 ms quantization;
        # the first read predates the crash, so the two-read interval is not used.
        require(elapsed + 1 >= remaining, "recovery completed before the retained lease expired")
        reads, slots = row["physicalReads"], row["slots"]
        require(len(reads) == len(slots) == protocol["budgets"]["recoveryAdapterAttemptsPerLookup"],
                "recovery physical attempts mismatch")
        for read in reads:
            read_check(read, 0)
        require(reads[0]["at"] < reads[1]["at"], "recovery reads duplicated or out of order")
        job = slots[0]["job_id"]
        require(isinstance(job, str) and job and job not in jobs, "duplicate recovery job")
        jobs.add(job)
        require([slot["ordinal"] for slot in slots] == [1, 2]
                and all(slot["job_id"] == job and slot["step"] == "read" for slot in slots), "recovery slots mismatch")
        require(all(isinstance(slot[field], str) and slot[field].strip()
                    for slot in slots for field in ("owner", "attempt_id")), "recovery attempt ownership missing")
        require(slots[0]["owner"] != slots[1]["owner"] and slots[0]["attempt_id"] != slots[1]["attempt_id"],
                "recovery reused attempt ownership")
        first, second = (load_json(slot["outcome_json"]) for slot in slots)
        require(first == dict(ok=False, error="UNKNOWN_OUTCOME", retry=True)
                and second == dict(ok=True, value=dict(ticketId="t0-00", subject="s" * 64)),
                "recovery outcomes mismatch")
        events = events_check(row["audit"], job, recovery=True)
        for slot in slots:
            require(events[f"reserve:read:{slot['ordinal']}"]["attempt_id"] == slot["attempt_id"],
                    "recovery audit/slot attempt mismatch")


def cleanup_check(data):
    expected = [(PROCESSES[row["variant"]], False, None) for row in data["cold"]]
    expected += [(PROCESSES[row["variant"]], False, row["memory"]) for row in data["warm"]]
    expected += [(3, False, None)] + [(3, crash, None) for _ in data["recovery"] for crash in (True, False)]
    require(len(data["processCleanup"]) == len(expected), "process cleanup count mismatch")
    for row, (minimum, crash, memory) in zip(data["processCleanup"], expected):
        require(row["crash"] is crash and row["liveProcessesRemaining"] == 0 and not row.get("error"),
                "process cleanup failed")
        pids = row["pids"]
        require(len(set(pids)) == len(pids) >= minimum, "cleanup process coverage mismatch")
        for pid in pids:
            number(pid, "cleanup PID", positive=True, integer=True)
        if memory:
            require(all(set(sample["pids"]) <= set(pids) for sample in memory), "sampled processes missing from cleanup")
    require(data["cleanup"] == "all application processes stopped, request threads joined, temporary namespaces removed",
            "final cleanup completion missing")


def validate(data):
    """Reject inconsistent evidence, including under python -O, without mutation."""
    try:
        _validate(data)
    except (KeyError, TypeError, IndexError, AttributeError, OverflowError) as error:
        raise ValueError("malformed evidence: " + str(error)) from error


def _validate(data):
    finite_tree(data)
    require(data["status"] in ("passed", "budget_miss") and not data.get("error"),
            "incomplete/error evidence cannot close P5")
    protocol = protocol_check(data)
    require(isinstance(data["sourceCommit"], str) and len(data["sourceCommit"]) in (40, 64)
            and all(character in "0123456789abcdef" for character in data["sourceCommit"]),
            "source commit must be a full Git object ID")
    require(isinstance(data["kernel"], str) and data["kernel"].strip(), "kernel metadata missing")
    require(type(data["workspaceDirty"]) is bool, "workspaceDirty must be a boolean")
    require(isinstance(data["system"], dict), "system metadata must be an object")
    require(all(isinstance(data["system"][key], str) and data["system"][key].strip()
                for key in ("os", "arch", "os_version", "cpu_model")), "system metadata missing")
    require(data["system"]["os"] == "linux" and data["system"]["arch"] == "aarch64",
            "report requires the declared Linux ARM64 target")
    sizes = {size["name"]: size for size in protocol["sizes"]}
    cells = {(variant, size, concurrency, delay) for variant in protocol["variants"] for size in sizes
             for concurrency in protocol["concurrency"]
             for delay in (protocol["adapterDelayMs"] if variant == "lookup" else [0])}
    expected_rounds = {cell + (index,) for cell in cells for index in range(protocol["rounds"])}
    seen, groups, series, jobs = set(), {}, {}, set()
    require(len(data["warm"]) == len(expected_rounds), "warm round count mismatch")
    for row in data["warm"]:
        key = tuple(row[name] for name in ("variant", "size", "concurrency", "adapterDelayMs", "round"))
        require(key in expected_rounds and key not in seen, "warm rounds missing, duplicated or unknown")
        seen.add(key)
        variant, size, concurrency, delay, index = key
        samples = row["samples"]
        require(len(samples) == protocol["requestsPerRound"], "warm sample count mismatch")
        require(Counter(sample["client"] for sample in samples) ==
                {client: len(samples) // concurrency for client in range(concurrency)}, "client allocation mismatch")
        require(row["errors"] == row["unexpectedDenials"] == 0, "round correctness error")
        reads = len(samples) + protocol["warmupRequests"] if variant == "lookup" else 0
        require(row["physicalReadsIncludingWarmup"] == reads, "warm read count mismatch")
        for sample in samples:
            sample_check(sample, variant, sizes[size], delay, jobs)
        duration = number(row["durationMs"], "round duration", positive=True)
        same_number(row["jobsPerSec"], len(samples) * 1000 / duration, "round throughput")
        require(all(sum(s["e2eMs"] for s in samples if s["client"] == client) <= duration
                    for client in range(concurrency)), "client latency exceeds round duration")
        memory_check(row)
        name = f"{variant}/{size}/c{concurrency}/delay{delay}"
        groups.setdefault(name, []).append(row)
    for name, rows in groups.items():
        samples = [sample for row in rows for sample in row["samples"]]
        for metric in ("e2eMs", "excludingAdapterSleepMs", "actualSleepMs"):
            series[name + "/" + metric] = [sample[metric] for sample in samples]
        for stage in samples[0]["stagesMs"]:
            series[name + "/stage/" + stage] = [sample["stagesMs"][stage] for sample in samples]
        for row in rows:
            series[name + f"/round{row['round']}/e2eMs"] = [sample["e2eMs"] for sample in row["samples"]]
    cold_ids = {(variant, index) for variant in protocol["variants"] for index in range(protocol["coldSamples"])}
    require(len(data["cold"]) == len(cold_ids)
            and {(row["variant"], row["sample"]) for row in data["cold"]} == cold_ids,
            "cold samples missing, duplicated or unknown")
    for row in data["cold"]:
        require(row["client"] == 0, "cold sample has unexpected client")
        sample_check(row, row["variant"], sizes[protocol["coldSize"]], protocol["coldAdapterDelayMs"], jobs)
        startup = number(row["startupMs"], "cold startup", positive=True)
        require(number(row["coldTotalMs"], "cold total", positive=True) >= startup + row["e2eMs"],
                "cold total excludes startup or request")
    for variant in protocol["variants"]:
        for metric in ("coldTotalMs", "startupMs"):
            name = "coldStartupMs" if metric == "startupMs" else metric
            series[variant + "/" + name] = [row[metric] for row in data["cold"] if row["variant"] == variant]
    recovery_check(data["recovery"], protocol, jobs)
    denials = data["denials"]
    require(denials["expectedDenials"] == 2 and denials["physicalReads"] == 0, "denial probe counts mismatch")
    require(len(denials["probes"]) == 2 and {row["path"] for row in denials["probes"]} ==
            {"/probe/fetch", "/probe/filesystem"}, "denial probes missing or duplicated")
    for row in denials["probes"]:
        require(row["status"] == 403 and row["denied"] is True, "capability probe not denied")
        number(row["elapsedMs"], "denial time", positive=True)
    cleanup_check(data)
    analysis = data["analysis"]
    require(set(analysis["distributions"]) == set(series), "distribution coverage mismatch")
    for name, samples in series.items():
        distribution = analysis["distributions"][name]
        require(set(distribution) == {"samples", "p50", "p95", "p99", "p50Ci95"}, "distribution fields mismatch")
        require(len(distribution["samples"]) == len(samples), "distribution sample count mismatch: " + name)
        for actual, expected in zip(distribution["samples"], samples):
            same_number(actual, expected, "distribution sample: " + name)
        for field, quantile, minimum in (("p50", .5, 1), ("p95", .95, 20), ("p99", .99, 100)):
            if len(samples) >= minimum:
                same_number(distribution[field], percentile(samples, quantile), name + "/" + field)
            else:
                require(distribution[field] is None, "unsupported percentile: " + name + "/" + field)
        require(distribution["p50Ci95"] is None, "correlated samples cannot claim IID confidence")
    expected_checks = {}
    def check(cell, metric, actual, limit, minimum=False):
        expected_checks[cell, metric] = (actual, limit, ">=" if minimum else "<=", actual >= limit if minimum else actual <= limit)
    for name, rows in groups.items():
        budget = protocol["budgets"][rows[0]["variant"]]
        check(name, "warmP95Ms", percentile(series[name + "/e2eMs"], .95), budget["warmP95Ms"])
        for row in rows:
            index = row["round"]
            check(name, f"round{index}.p95Ms", percentile([s["e2eMs"] for s in row["samples"]], .95), budget["warmP95Ms"])
            check(name, f"round{index}.jobsPerSec", len(row["samples"]) * 1000 / row["durationMs"],
                  budget["minJobsPerSec"][str(row["concurrency"])], True)
        check(name, "peakPssKiB", max(m["valueKiB"] for row in rows for m in row["memory"]), budget["peakPssKiB"])
    for variant in protocol["variants"]:
        check(variant, "coldP95Ms", percentile(series[variant + "/coldTotalMs"], .95), protocol["budgets"][variant]["coldP95Ms"])
    check("lookup", "recoveryMaxMs", max(row["recoveryMs"] for row in data["recovery"]), protocol["budgets"]["recoveryMaxMs"])
    checks = analysis["checks"]
    require(len(checks) == len(expected_checks) == 132, "numerical checks missing or duplicated")
    seen = set()
    for row in checks:
        key = row["cell"], row["metric"]
        require(key in expected_checks and key not in seen, "numerical checks missing, duplicated or unknown")
        seen.add(key)
        actual, limit, relation, passed = expected_checks[key]
        same_number(row["actual"], actual, "budget actual: " + str(key))
        require(row["limit"] == limit and row["relation"] == relation, "budget limit/relation changed")
        require(row["passed"] is passed, "budget pass flag does not recompute")
    passed = all(row[3] for row in expected_checks.values())
    require(analysis["passed"] is passed and data["status"] == ("passed" if passed else "budget_miss"),
            "overall status does not match recomputed budgets")
