#!/usr/bin/env bash
set -euo pipefail

strict=false
host_only=false
output="target/benchmark-comparison/runner-doctor.json"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --strict)
      strict=true
      shift
      ;;
    --host-only)
      host_only=true
      shift
      ;;
    --output)
      output="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "runner doctor supports Linux only" >&2
  exit 1
fi
required_commands=(cargo git jq sha256sum)
if [[ "${host_only}" == false ]]; then
  required_commands+=(node bun deno)
fi
for command in "${required_commands[@]}"; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "missing required command: ${command}" >&2
    exit 1
  fi
done

workspace_root="$(git rev-parse --show-toplevel)"
source_toolchains_json='{}'
runtimes_json='{}'
if [[ "${host_only}" == false ]]; then
  typescript_path="${workspace_root}/node_modules/.bin/tsc"
  if [[ ! -x "${typescript_path}" ]]; then
    echo "missing locked TypeScript compiler; run 'pnpm install --frozen-lockfile'" >&2
    exit 1
  fi

  node_version="$(node --version)"
  bun_version="$(bun --version)"
  deno_version="$(deno --version | head -n 1)"
  typescript_version="$(${typescript_path} --version)"
  [[ "${node_version}" == "v24.19.0" ]]
  [[ "${bun_version}" == "1.4.0" ]]
  [[ "${deno_version}" == "deno 2.9.5"* ]]
  [[ "${typescript_version}" == "Version 7.0.2" ]]

  node_path="$(command -v node)"
  bun_path="$(command -v bun)"
  deno_path="$(command -v deno)"
  node_sha256="$(sha256sum "${node_path}" | awk '{ print $1 }')"
  bun_sha256="$(sha256sum "${bun_path}" | awk '{ print $1 }')"
  deno_sha256="$(sha256sum "${deno_path}" | awk '{ print $1 }')"
  typescript_sha256="$(sha256sum "${typescript_path}" | awk '{ print $1 }')"
  pnpm_lock_sha256="$(sha256sum "${workspace_root}/pnpm-lock.yaml" | awk '{ print $1 }')"
  source_toolchains_json="$(jq -n \
    --arg version "${typescript_version}" \
    --arg path "${typescript_path}" \
    --arg sha256 "${typescript_sha256}" \
    --arg pnpmLockSha256 "${pnpm_lock_sha256}" \
    '{typescript: {version: $version, path: $path, sha256: $sha256, pnpmLockSha256: $pnpmLockSha256}}')"
  runtimes_json="$(jq -n \
    --arg nodeVersion "${node_version}" --arg nodePath "${node_path}" --arg nodeSha256 "${node_sha256}" \
    --arg bunVersion "${bun_version}" --arg bunPath "${bun_path}" --arg bunSha256 "${bun_sha256}" \
    --arg denoVersion "${deno_version}" --arg denoPath "${deno_path}" --arg denoSha256 "${deno_sha256}" \
    '{
      node: {version: $nodeVersion, path: $nodePath, sha256: $nodeSha256},
      bun: {version: $bunVersion, path: $bunPath, sha256: $bunSha256},
      deno: {version: $denoVersion, path: $denoPath, sha256: $denoSha256}
    }')"
fi

case "$(uname -m)" in
  x86_64) architecture="x86_64" ;;
  aarch64|arm64) architecture="aarch64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

logical_cpus="$(getconf _NPROCESSORS_ONLN)"
total_memory_kb="$(awk '/^MemTotal:/ { print $2 }' /proc/meminfo)"
load_average_1m="$(awk '{ print $1 }' /proc/loadavg)"
cpu_model="$(awk -F ': ' '/model name|Processor/ { print $2; exit }' /proc/cpuinfo)"
if [[ -z "${cpu_model}" ]] && command -v lscpu >/dev/null 2>&1; then
  cpu_model="$(lscpu | awk -F ':' '/^Model name:/ { sub(/^[[:space:]]+/, "", $2); print $2; exit }')"
fi
cpu_model="${cpu_model:-unknown}"
kernel="$(uname -srvo)"
source_commit="$(git rev-parse HEAD)"
workspace_dirty=false
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  workspace_dirty=true
fi

governors=""
for governor_file in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
  if [[ -f "${governor_file}" ]]; then
    governors+="$(<"${governor_file}")"$'\n'
  fi
done
governors_json="$(printf '%s' "${governors}" | awk 'NF' | sort -u | jq -R -s 'split("\n") | map(select(length > 0))')"

turbo_enabled="null"
if [[ -f /sys/devices/system/cpu/intel_pstate/no_turbo ]]; then
  if [[ "$(</sys/devices/system/cpu/intel_pstate/no_turbo)" == "0" ]]; then
    turbo_enabled=true
  else
    turbo_enabled=false
  fi
elif [[ -f /sys/devices/system/cpu/cpufreq/boost ]]; then
  if [[ "$(</sys/devices/system/cpu/cpufreq/boost)" == "1" ]]; then
    turbo_enabled=true
  else
    turbo_enabled=false
  fi
fi

container="none"
if command -v systemd-detect-virt >/dev/null 2>&1; then
  container="$(systemd-detect-virt --container 2>/dev/null || true)"
  container="${container:-none}"
fi
if [[ -f /.dockerenv || -f /run/.containerenv ]]; then
  container="container"
fi

mkdir -p "$(dirname "${output}")"
jq -n \
  --arg scope "$([[ "${host_only}" == true ]] && printf host-only || printf full)" \
  --arg architecture "${architecture}" \
  --arg kernel "${kernel}" \
  --arg cpuModel "${cpu_model}" \
  --argjson logicalCpus "${logical_cpus}" \
  --argjson totalMemoryKb "${total_memory_kb}" \
  --argjson loadAverage1m "${load_average_1m}" \
  --argjson cpuGovernors "${governors_json}" \
  --argjson turboEnabled "${turbo_enabled}" \
  --arg container "${container}" \
  --arg sourceCommit "${source_commit}" \
  --argjson workspaceDirty "${workspace_dirty}" \
  --argjson sourceToolchains "${source_toolchains_json}" \
  --argjson runtimes "${runtimes_json}" \
  '{
    schemaVersion: 1,
    scope: $scope,
    architecture: $architecture,
    kernel: $kernel,
    cpuModel: $cpuModel,
    logicalCpus: $logicalCpus,
    totalMemoryKb: $totalMemoryKb,
    loadAverage1m: $loadAverage1m,
    cpuGovernors: $cpuGovernors,
    turboEnabled: $turboEnabled,
    container: $container,
    sourceCommit: $sourceCommit,
    workspaceDirty: $workspaceDirty,
    sourceToolchains: $sourceToolchains,
    runtimes: $runtimes
  }' > "${output}"

if [[ "${strict}" == true ]]; then
  [[ "${workspace_dirty}" == false ]]
  if [[ "${container}" != "none" ]]; then
    echo "strict record mode must run directly on the fixed runner, not inside a container" >&2
    exit 1
  fi
  non_performance="$(printf '%s' "${governors}" | awk 'NF && $0 != "performance" { count += 1 } END { print count + 0 }')"
  if [[ "${non_performance}" -gt 0 ]]; then
    echo "CPU governor must be performance in strict mode" >&2
    exit 1
  fi
  if ! awk -v load_avg="${load_average_1m}" -v cpus="${logical_cpus}" 'BEGIN { exit !(load_avg / cpus < 0.25) }'; then
    echo "runner load is too high for strict mode: ${load_average_1m} / ${logical_cpus} CPUs" >&2
    exit 1
  fi
fi

printf '%s\n' "${output}"
