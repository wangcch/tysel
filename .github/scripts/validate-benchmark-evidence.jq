def gate($suite; $name; $unit; $limit):
  [.suites[]
    | select(.suite == $suite)
    | .metrics[]
    | select(.name == $name)] as $matches
  | ($matches | length) == 1
    and ($matches[0].unit == $unit)
    and ($matches[0].p50 | type == "number" and . >= 0)
    and ($matches[0].limit == $limit)
    and ($matches[0].passed | type == "boolean")
    and ($matches[0].passed == ($matches[0].p50 <= $limit))
    and ($matches[0].status == (if $matches[0].passed then "pass" else "fail" end));

. as $evidence
| .evidence_version == 2
and .source_commit == $source_commit
and .target == "linux-x64"
and .profile == "release"
and (.command | type == "string" and length > 0)
and .system.os == "linux"
and .system.arch == "x86_64"
and (.system.os_version | type == "string" and length > 0)
and (.system.cpu_model | type == "string" and length > 0)
and (.artifact.sha256 | type == "string" and test("^[0-9a-f]{64}$"))
and (.measurements.memory_kind == "pss")
and (.measurements.cold_start_p50_ms
  | (.measured | type == "number" and . >= 0)
    and .limit == 15
    and (.passed | type == "boolean")
    and (.passed == (.measured <= .limit)))
and (.measurements.idle_memory_mb
  | (.measured | type == "number" and . >= 0)
    and .limit == 32
    and (.passed | type == "boolean")
    and (.passed == (.measured <= .limit)))
and (.measurements.artifact_mb
  | (.measured | type == "number" and . >= 0)
    and .limit == 20
    and (.passed | type == "boolean")
    and (.passed == (.measured <= .limit)))
and (.suites | type == "array")
and ([.suites[]
  | .commit == $source_commit
    and .system == $evidence.system]
  | all)
and gate("startup"; "cold_start_p50_ms"; "ms"; 15)
and gate("memory"; "idle_memory_mb"; "MB"; 32)
and gate("binary-size"; "artifact_mb"; "MB"; 20)
and gate("isolate"; "warm_create_ms"; "ms"; 5)
and gate("isolate"; "reuse_1000_growth_kb"; "KB"; 16384)
and gate("task"; "backpressure_memory_delta_kb"; "KB"; 32768)
and gate("durable"; "resume_ms"; "ms"; 10)
