#!/usr/bin/env python3
"""Offline CPU accounting, coverage and evidence-corruption tests for O4.1."""
import copy
import gzip
import importlib.util
import json
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("capacity", HERE / "capacity.py")
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)
PLAN = json.loads((HERE / "capacity-protocol.json").read_text())


def evidence():
    with gzip.open(HERE / "fixtures/dispatch/o4-measurements.json.gz", "rt") as stream:
        archive = json.load(stream)
    templates = {r["adapterDelayMs"]: r for r in archive["warm"]
                 if r["variant"] == "lookup" and r["size"] == "bounded" and r["concurrency"] == 4}
    data = dict(status="complete", supportClosed=True, protocol=copy.deepcopy(PLAN), rounds=[],
                processCleanup=[dict(liveProcessesRemaining=0)])
    for identity in driver.schedule(PLAN):
        row = copy.deepcopy(templates[identity["adapterDelayMs"]])
        row.update(identity, namespaceRemoved=True, finalRetainedJobs=identity["retainedJobs"] + 72,
                   physicalReadsIncludingWarmup=72, durationMs=64000 / identity["concurrency"],
                   jobsPerSec=identity["concurrency"])
        row["samples"] = copy.deepcopy(row["samples"]) + copy.deepcopy(row["samples"])
        for index, sample in enumerate(row["samples"]):
            sample["jobId"] = f"job-{identity['sequence']}-{index}"
            sample["client"] = index % identity["concurrency"]
            sample["adapterAttempts"][0].update(customerId=f"bench-c{sample['client']}",
                                                ticketId=f"t{sample['client']}-00")
            sample["adapter"]["ticketId"] = f"t{sample['client']}-00"
            for event in sample["audit"]:
                event["job_id"] = sample["jobId"]
        pids = row["memory"][0]["pids"]
        for sample in row["memory"]:
            sample["pids"] = pids
        before = {str(pid): dict(userTicks=100, systemTicks=50, startTicks=7) for pid in pids}
        after = {str(pid): dict(userTicks=150, systemTicks=64, startTicks=7) for pid in pids}
        native_ms = len(pids) * 640
        row["cpu"] = dict(before=before, after=after, ticksPerSecond=100,
                          nativeMs=native_ms, nativeMsPerJob=native_ms / 64,
                          harnessMs=100, windowMs=row["durationMs"] + 1)
        observer = driver.paired.Observer(None)
        jobs = [s["jobId"] for s in row["samples"]] + [f"warm-{identity['sequence']}-{i}" for i in range(8)]
        for job in jobs:
            for kind, status, method in (("admission", 202, "POST"), ("status", 200, "GET"), ("dispatch", 200, "POST")):
                observer.calls.append(dict(index=len(observer.calls), kind=kind, status=status,
                                           method=method, jobId=job, durationMs=1.0, duringStop=False))
        observer.calls.append(dict(index=len(observer.calls), kind="pending", status=200,
                                   method="GET", durationMs=1.0, duringStop=False))
        observer.next_index = len(observer.calls)
        row["httpObservation"] = observer.report()
        data["rounds"].append(row)
        data["processCleanup"].append(dict(liveProcessesRemaining=0))
    return data


class CapacityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.raw = evidence()

    def test_complete_diagnostic_does_not_select_a_candidate(self):
        result = driver.summarize(self.raw)
        self.assertEqual(result["status"], "valid-diagnostic")
        self.assertFalse(result["productionCandidateSelected"])
        self.assertEqual(len(result["cells"]), 16)

    def test_cpu_fields_ignore_spaces_and_parentheses_in_comm(self):
        fields = ["S"] + ["0"] * 19
        fields[11], fields[12], fields[19] = "120", "30", "999"
        value = driver.parse_cpu_stat("17 (a name ) with (brackets)) " + " ".join(fields))
        self.assertEqual(value, dict(userTicks=120, systemTicks=30, startTicks=999))
        after = {"17": dict(userTicks=140, systemTicks=40, startTicks=999)}
        self.assertEqual(driver.cpu_delta({"17": value}, after, 100), 300)

    def test_cpu_identity_counter_and_clock_fail_closed(self):
        before = {"17": dict(userTicks=100, systemTicks=10, startTicks=3)}
        for after, hz in [({}, 100), ({"18": before["17"]}, 100),
                          ({"17": dict(userTicks=110, systemTicks=10, startTicks=4)}, 100),
                          ({"17": dict(userTicks=99, systemTicks=10, startTicks=3)}, 100),
                          (before, 0), (before, float("nan"))]:
            with self.subTest(after=after, hz=hz), self.assertRaises(ValueError):
                driver.cpu_delta(before, after, hz)

    def test_protocol_cannot_relax_capacity_or_observation(self):
        for key, value in [("concurrency", [1, 4]), ("rounds", 1), ("statusPollMs", 50),
                           ("retainedJobs", [0, 10]), ("requestsPerRound", 32)]:
            plan = copy.deepcopy(PLAN)
            plan[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                driver.schedule(plan)

    def test_missing_round_cleanup_and_invalid_terminal_state_rejected(self):
        for field in ["rounds", "processCleanup"]:
            data = copy.deepcopy(self.raw)
            data[field].pop()
            with self.subTest(field=field), self.assertRaises(ValueError):
                driver.summarize(data)
        data = copy.deepcopy(self.raw)
        data["status"] = "error"
        with self.assertRaises(ValueError): driver.summarize(data)

    def test_forged_cost_coverage_and_missing_http_rejected(self):
        mutations = [lambda r: r["cpu"].update(nativeMs=1),
                     lambda r: r["cpu"].update(nativeMsPerJob=1),
                     lambda r: r["cpu"].update(windowMs=1),
                     lambda r: r.update(physicalReadsIncludingWarmup=71),
                     lambda r: r.update(finalRetainedJobs=999),
                     lambda r: r["httpObservation"]["calls"].pop(),
                     lambda r: r["cpu"]["before"].pop(next(iter(r["cpu"]["before"])))]
        for index, mutate in enumerate(mutations):
            data = copy.deepcopy(self.raw)
            mutate(data["rounds"][0])
            with self.subTest(index=index), self.assertRaises(ValueError):
                driver.summarize(data)


if __name__ == "__main__":
    unittest.main()
