#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: run-qjs-boundary-linux.sh --arch <x86_64|arm64> [--runs <positive integer>]
  [--iterations <positive integer>] [--warmup-iterations <positive integer>]
  [--output-dir <directory>]
EOF
  exit 2
}

arch=""
runs=3
iterations=2000
warmup_iterations=200
output_dir="target/benchmark-comparison/qjs-boundary"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch) arch="$2"; shift 2 ;;
    --runs) runs="$2"; shift 2 ;;
    --iterations) iterations="$2"; shift 2 ;;
    --warmup-iterations) warmup_iterations="$2"; shift 2 ;;
    --output-dir) output_dir="$2"; shift 2 ;;
    *) usage ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] || { echo "QuickJS boundary record supports Linux only" >&2; exit 1; }
[[ "${arch}" == "x86_64" || "${arch}" == "arm64" ]] || usage
[[ "${runs}" =~ ^[1-9][0-9]*$ ]] || usage
[[ "${iterations}" =~ ^[1-9][0-9]*$ ]] || usage
[[ "${warmup_iterations}" =~ ^[1-9][0-9]*$ ]] || usage

case "$(uname -m)" in
  x86_64) actual_arch="x86_64" ;;
  aarch64|arm64) actual_arch="arm64" ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac
[[ "${actual_arch}" == "${arch}" ]] || {
  echo "runner architecture mismatch: requested ${arch}, found ${actual_arch}" >&2
  exit 1
}

mkdir -p "${output_dir}"
cargo build --locked --release -p tysel-bench-compare --bin tysel-bench-qjs-boundary

for ((run = 1; run <= runs; run += 1)); do
  output="${output_dir}/qjs-boundary-linux-${arch}-run${run}.json"
  target/release/tysel-bench-qjs-boundary \
    --warmup-iterations "${warmup_iterations}" \
    --iterations "${iterations}" \
    --output "${output}"
  [[ -s "${output}" ]] || { echo "missing boundary evidence: ${output}" >&2; exit 1; }
  jq -e \
    --argjson iterations "${iterations}" \
    '.schemaVersion == 1 and .iterations == $iterations and (.measurements | length) == 16' \
    "${output}" >/dev/null
done
