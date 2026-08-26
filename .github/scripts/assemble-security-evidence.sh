#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "usage: $0 <policy-evidence> <release-output> <version> <source-commit> <lockfile> <fuzz-toolchain>" >&2
  exit 2
fi

policy="$1"
release_output="$2"
version="$3"
source_commit="$4"
lockfile="$5"
fuzz_toolchain="$6"

bash "$(dirname "$0")/release-channel.sh" "$version" > /dev/null
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid source commit" >&2; exit 2; }
[[ "$fuzz_toolchain" =~ ^nightly-[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]] || {
  echo "invalid fuzz toolchain" >&2
  exit 2
}
[[ -f "$policy" ]] || { echo "security policy evidence is missing" >&2; exit 2; }
[[ -d "$release_output" ]] || { echo "release output directory is missing" >&2; exit 2; }
[[ -f "$lockfile" ]] || { echo "Cargo lockfile is missing" >&2; exit 2; }

lock_sha256="$(sha256sum "$lockfile" | awk '{print $1}')"
jq -e --arg sourceCommit "$source_commit" --arg cargoLockSha256 "$lock_sha256" \
  --arg fuzzToolchain "$fuzz_toolchain" \
  '.evidenceVersion == 1 and .scope == "source"
    and .sourceCommit == $sourceCommit
    and .cargoLockSha256 == $cargoLockSha256
    and .dependencyGates == {cargoAudit: "0.22.2", cargoDeny: "0.20.2"}
    and .fuzz == {cargoFuzz: "0.13.2", toolchain: $fuzzToolchain,
      runsPerTarget: 10000,
      targets: ["tap_decode", "manifest_parse", "isolate_ipc", "task_rpc", "release_metadata"]}' \
  "$policy" > /dev/null

for target in linux-x64 linux-arm64; do
  archive="${release_output}/tysel-${version}-${target}.tar.gz"
  checksum="${archive}.sha256"
  [[ -f "$archive" ]] || { echo "release archive is missing for ${target}" >&2; exit 1; }
  [[ -f "$checksum" ]] || { echo "release checksum is missing for ${target}" >&2; exit 1; }
  expected_sha256="$(tr -d '[:space:]' < "$checksum")"
  [[ "$expected_sha256" =~ ^[0-9a-f]{64}$ ]] || {
    echo "release checksum is not canonical for ${target}" >&2
    exit 1
  }
  actual_sha256="$(sha256sum "$archive" | awk '{print $1}')"
  if [[ "$actual_sha256" != "$expected_sha256" ]]; then
    echo "release archive checksum mismatch for ${target}" >&2
    exit 1
  fi
  jq --arg target "$target" --arg artifactSha256 "$actual_sha256" \
    '. + {target: $target, artifactSha256: $artifactSha256} | del(.scope)' \
    "$policy" > "${release_output}/security-evidence-${target}.json"
done
