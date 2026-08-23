#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: sustained profiling requires Linux" >&2
  exit 1
fi

output="target/benchmark-comparison/sustained-profile"
path="/bytes/64k"
response="bytes-64k"
concurrency=100
duration_seconds=60
sample_ms=200
require_perf=false
fail_on_degradation=false
max_degradation_pct=5
while (($#)); do
  case "$1" in
    --output) output="$2"; shift 2 ;;
    --path) path="$2"; shift 2 ;;
    --response) response="$2"; shift 2 ;;
    --concurrency) concurrency="$2"; shift 2 ;;
    --duration-seconds) duration_seconds="$2"; shift 2 ;;
    --sample-ms) sample_ms="$2"; shift 2 ;;
    --require-perf) require_perf=true; shift ;;
    --max-degradation-pct) max_degradation_pct="$2"; shift 2 ;;
    --fail-on-degradation) fail_on_degradation=true; shift ;;
    *) echo "error: unknown argument $1" >&2; exit 2 ;;
  esac
done

[[ "${concurrency}" =~ ^[1-9][0-9]*$ ]] || { echo "error: concurrency must be positive" >&2; exit 2; }
[[ "${duration_seconds}" =~ ^[0-9]+$ ]] || { echo "error: duration-seconds must be an integer" >&2; exit 2; }
((duration_seconds >= 3)) || { echo "error: duration-seconds must be at least 3" >&2; exit 2; }
[[ "${sample_ms}" =~ ^[1-9][0-9]*$ ]] || { echo "error: sample-ms must be positive" >&2; exit 2; }
[[ "${max_degradation_pct}" =~ ^[0-9]+([.][0-9]+)?$ ]] || { echo "error: max-degradation-pct must be non-negative" >&2; exit 2; }

root="$(git rev-parse --show-toplevel)"
case "${output}" in
  /*) ;;
  *) output="${root}/${output}" ;;
esac
mkdir -p "${output}"

cargo build --locked --profile profiling -p tysel-cli --bin tysel
cargo build --locked --release -p tysel-bench-compare --bin tysel-bench-load

server_log="${output}/server.log"
frequency_csv="${output}/cpu-frequency.csv"
thread_csv="${output}/thread-cpu.csv"
echo "timestamp_ms,cpu,frequency_khz" > "${frequency_csv}"
echo "timestamp_ms,tid,comm,user_ticks,system_ticks,processor" > "${thread_csv}"

server_pid=""
frequency_pid=""
thread_pid=""
perf_pid=""
cleanup() {
  for pid in "${frequency_pid}" "${thread_pid}" "${perf_pid}" "${server_pid}"; do
    if [[ -n "${pid}" ]]; then
      kill "${pid}" 2>/dev/null || true
      wait "${pid}" 2>/dev/null || true
    fi
  done
}
trap cleanup EXIT INT TERM

"${root}/target/profiling/tysel" run \
  --manifest "${root}/benchmarks/comparison/adapters/tysel/tysel-profile.toml" \
  >"${server_log}" 2>&1 &
server_pid=$!
for _ in {1..200}; do
  if curl --fail --silent --output /dev/null http://127.0.0.1:39001/health; then
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    echo "error: Tysel exited before readiness; see ${server_log}" >&2
    exit 1
  fi
  sleep 0.05
done
curl --fail --silent --output /dev/null http://127.0.0.1:39001/health

sample_seconds="$(awk -v ms="${sample_ms}" 'BEGIN { printf "%.3f", ms / 1000 }')"
(
  shopt -s nullglob
  while kill -0 "${server_pid}" 2>/dev/null; do
    timestamp_ms="$(date +%s%3N)"
    for cpu_dir in /sys/devices/system/cpu/cpu[0-9]*; do
      frequency_file="${cpu_dir}/cpufreq/scaling_cur_freq"
      [[ -r "${frequency_file}" ]] || frequency_file="${cpu_dir}/cpufreq/cpuinfo_cur_freq"
      if [[ -r "${frequency_file}" ]]; then
        printf '%s,%s,%s\n' "${timestamp_ms}" "${cpu_dir##*cpu}" "$(<"${frequency_file}")"
      fi
    done
    sleep "${sample_seconds}"
  done
) >> "${frequency_csv}" &
frequency_pid=$!

(
  shopt -s nullglob
  while kill -0 "${server_pid}" 2>/dev/null; do
    timestamp_ms="$(date +%s%3N)"
    for stat_file in "/proc/${server_pid}"/task/*/stat; do
      stat="$(<"${stat_file}")"
      tid="${stat%% *}"
      comm="${stat#*(}"
      comm="${comm%%)*}"
      fields="${stat#*) }"
      read -r -a values <<< "${fields}"
      printf '%s,%s,%s,%s,%s,%s\n' \
        "${timestamp_ms}" "${tid}" "${comm//,/ }" "${values[11]}" "${values[12]}" "${values[36]}"
    done
    sleep "${sample_seconds}"
  done
) >> "${thread_csv}" &
thread_pid=$!

perf_status="unavailable"
if command -v perf >/dev/null 2>&1; then
  perf record -F 99 -g -p "${server_pid}" -o "${output}/perf.data" -- \
    sleep "${duration_seconds}" >"${output}/perf-record.log" 2>&1 &
  perf_pid=$!
  perf_status="recording"
elif [[ "${require_perf}" == true ]]; then
  echo "error: perf is required but unavailable" >&2
  exit 1
fi

"${root}/target/release/tysel-bench-load" \
  --address 127.0.0.1:39001 \
  --path "${path}" \
  --response "${response}" \
  --concurrency "${concurrency}" \
  --duration-seconds "${duration_seconds}" \
  --output "${output}/load.json"

for pid_name in frequency_pid thread_pid; do
  pid="${!pid_name}"
  kill "${pid}" 2>/dev/null || true
  wait "${pid}" 2>/dev/null || true
  printf -v "${pid_name}" ''
done
if [[ -n "${perf_pid}" ]]; then
  if wait "${perf_pid}"; then
    if perf report --stdio --sort comm,dso,symbol -i "${output}/perf.data" \
      > "${output}/perf-report.txt"; then
      perf_status="recorded"
    else
      perf_status="report-failed"
      [[ "${require_perf}" == false ]] || exit 1
    fi
  else
    perf_status="failed"
    [[ "${require_perf}" == false ]] || exit 1
  fi
  perf_pid=""
fi

awk -F, '
  NR == 1 { next }
  !($2 in count) { min[$2] = $3; max[$2] = $3 }
  { count[$2]++; sum[$2] += $3; if ($3 < min[$2]) min[$2] = $3; if ($3 > max[$2]) max[$2] = $3 }
  END {
    print "cpu,samples,min_khz,average_khz,max_khz"
    for (cpu in count) printf "%s,%d,%.0f,%.0f,%.0f\n", cpu, count[cpu], min[cpu], sum[cpu] / count[cpu], max[cpu]
  }
' "${frequency_csv}" > "${output}/cpu-frequency-summary.csv"

clock_ticks="$(getconf CLK_TCK)"
awk -F, -v ticks="${clock_ticks}" '
  NR == 1 { next }
  !($2 in first_total) { first_total[$2] = $4 + $5; first_ms[$2] = $1; names[$2] = $3 }
  { last_total[$2] = $4 + $5; last_ms[$2] = $1 }
  END {
    for (tid in last_total) {
      seconds = (last_ms[tid] - first_ms[tid]) / 1000
      cpu = seconds > 0 ? (last_total[tid] - first_total[tid]) / ticks / seconds * 100 : 0
      printf "%s,%s,%.3f\n", tid, names[tid], cpu
  }
  }
' "${thread_csv}" | sort -t, -k3,3nr > "${output}/thread-cpu-summary.rows"
{
  echo "tid,comm,cpu_core_pct"
  cat "${output}/thread-cpu-summary.rows"
} > "${output}/thread-cpu-summary.csv"
rm "${output}/thread-cpu-summary.rows"

awk -F, -v ticks="${clock_ticks}" '
  BEGIN { print "timestamp_ms,tid,comm,processor,cpu_core_pct" }
  NR == 1 { next }
  ($2 in previous_total) {
    elapsed = ($1 - previous_ms[$2]) / 1000
    cpu = elapsed > 0 ? (($4 + $5) - previous_total[$2]) / ticks / elapsed * 100 : 0
    printf "%s,%s,%s,%s,%.3f\n", $1, $2, $3, $6, cpu
  }
  { previous_total[$2] = $4 + $5; previous_ms[$2] = $1 }
' "${thread_csv}" > "${output}/thread-cpu-windows.csv"

awk -F, '
  NR == 1 { next }
  { key = $2 "," $3 "," $6; count[key]++ }
  END {
    for (key in count) printf "%s,%d\n", key, count[key]
  }
' "${thread_csv}" | sort -t, -k4,4nr > "${output}/thread-cpu-residency.rows"
{
  echo "tid,comm,cpu,samples"
  cat "${output}/thread-cpu-residency.rows"
} > "${output}/thread-cpu-residency.csv"
rm "${output}/thread-cpu-residency.rows"

jq '
  .windows as $windows
  | (($windows | length) / 3 | floor) as $n
  | ($windows[0:$n] | map(.requestsPerSecond) | sort | .[length / 2 | floor]) as $first
  | ($windows[-$n:] | map(.requestsPerSecond) | sort | .[length / 2 | floor]) as $last
  | {
      firstThirdMedianRps: $first,
      lastThirdMedianRps: $last,
      changePct: (if $first == 0 then null else (($last - $first) / $first * 100) end),
      clientCpuCorePct,
      logicalCpus,
      clientCapacityPct: (if .clientCpuCorePct == null then null
        else (.clientCpuCorePct / (.logicalCpus * 100) * 100) end)
    }
' "${output}/load.json" > "${output}/window-analysis.json"

load_start_ms="$(jq -r '.startedAtUnixMs' "${output}/load.json")"
load_duration_ms="$(jq -r '.actualDurationMs' "${output}/load.json")"
first_end_ms="$(awk -v start="${load_start_ms}" -v duration="${load_duration_ms}" \
  'BEGIN { printf "%.0f", start + duration / 3 }')"
last_start_ms="$(awk -v start="${load_start_ms}" -v duration="${load_duration_ms}" \
  'BEGIN { printf "%.0f", start + duration * 2 / 3 }')"

awk -F, -v first_end="${first_end_ms}" -v last_start="${last_start_ms}" '
  NR == 1 { next }
  $1 <= first_end { first_sum += $3; first_count++ }
  $1 >= last_start { last_sum += $3; last_count++ }
  END {
    print "phase,samples,average_frequency_khz"
    printf "first,%d,%.3f\n", first_count, first_count ? first_sum / first_count : 0
    printf "last,%d,%.3f\n", last_count, last_count ? last_sum / last_count : 0
  }
' "${frequency_csv}" > "${output}/frequency-phase-summary.csv"

awk -F, -v first_end="${first_end_ms}" -v last_start="${last_start_ms}" '
  NR == 1 { next }
  $1 <= first_end { first_sum[$3] += $5; first_count[$3]++ }
  $1 >= last_start { last_sum[$3] += $5; last_count[$3]++ }
  { names[$3] = 1 }
  END {
    for (name in names) {
      first = first_count[name] ? first_sum[name] / first_count[name] : 0
      last = last_count[name] ? last_sum[name] / last_count[name] : 0
      printf "%s,%d,%.3f,%d,%.3f,%.3f\n", name, first_count[name], first, last_count[name], last, last - first
    }
  }
' "${output}/thread-cpu-windows.csv" | sort -t, -k5,5nr \
  > "${output}/thread-phase-summary.rows"
{
  echo "comm,first_samples,first_cpu_core_pct,last_samples,last_cpu_core_pct,change_core_pct"
  cat "${output}/thread-phase-summary.rows"
} > "${output}/thread-phase-summary.csv"
rm "${output}/thread-phase-summary.rows"

jq -n \
  --arg sourceCommit "$(git rev-parse HEAD)" \
  --arg binarySha256 "$(sha256sum "${root}/target/profiling/tysel" | awk '{print $1}')" \
  --arg kernel "$(uname -r)" \
  --arg architecture "$(uname -m)" \
  --arg perfStatus "${perf_status}" \
  --arg perfEventParanoid "$(< /proc/sys/kernel/perf_event_paranoid)" \
  --arg path "${path}" \
  --arg response "${response}" \
  --argjson concurrency "${concurrency}" \
  --argjson durationSeconds "${duration_seconds}" \
  --argjson frequencySamples "$(($(wc -l < "${frequency_csv}") - 1))" \
  --argjson threadSamples "$(($(wc -l < "${thread_csv}") - 1))" \
  '{schemaVersion: 1, sourceCommit: $sourceCommit, binarySha256: $binarySha256,
    kernel: $kernel, architecture: $architecture, perfStatus: $perfStatus,
    perfEventParanoid: $perfEventParanoid, frequencySamples: $frequencySamples,
    threadSamples: $threadSamples,
    workload: {path: $path, response: $response, concurrency: $concurrency,
      durationSeconds: $durationSeconds}}' > "${output}/metadata.json"

echo "Sustained profile ${output}"
if [[ "${fail_on_degradation}" == true ]] && ! jq -e \
  --argjson maximum "${max_degradation_pct}" \
  '.changePct != null and .changePct >= -$maximum' \
  "${output}/window-analysis.json" >/dev/null; then
  echo "error: sustained throughput degradation exceeds ${max_degradation_pct}%" >&2
  exit 1
fi
