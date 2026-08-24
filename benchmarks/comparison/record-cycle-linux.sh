#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: record-cycle-linux.sh --arch <x86_64|arm64> --cycle <1|2|3> \
  --seeds "<space-separated 1..4>" --record-root <directory>
EOF
  exit 2
}

arch=""
cycle=""
seeds=""
record_root=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)
      [[ $# -ge 2 ]] || usage
      arch="$2"
      shift 2
      ;;
    --cycle)
      [[ $# -ge 2 ]] || usage
      cycle="$2"
      shift 2
      ;;
    --seeds)
      [[ $# -ge 2 ]] || usage
      seeds="$2"
      shift 2
      ;;
    --record-root)
      [[ $# -ge 2 ]] || usage
      record_root="$2"
      shift 2
      ;;
    *) usage ;;
  esac
done

[[ "${arch}" == "x86_64" || "${arch}" == "arm64" ]] || usage
[[ "${cycle}" =~ ^[1-3]$ ]] || usage
[[ -n "${seeds}" && -n "${record_root}" ]] || usage

declare -A selected=()
for seed in ${seeds}; do
  [[ "${seed}" =~ ^[1-4]$ ]] || usage
  [[ -z "${selected[${seed}]:-}" ]] || {
    echo "duplicate seed: ${seed}" >&2
    exit 2
  }
  selected[${seed}]=1
done

mkdir -p "${record_root}"
summary="${record_root}/summary-v1-${arch}-cycle${cycle}.json"
rm -f "${summary}" "${summary}.tmp"

for seed in ${seeds}; do
  output="${record_root}/comparison-v1-${arch}-cycle${cycle}-seed${seed}.json"
  temporary="${output}.tmp"
  rm -f "${output}" "${temporary}"
  cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-compare -- \
    --order-seed "${seed}" \
    --output "${temporary}"
  mv "${temporary}" "${output}"
done

complete=true
for seed in 1 2 3 4; do
  [[ -s "${record_root}/comparison-v1-${arch}-cycle${cycle}-seed${seed}.json" ]] \
    || complete=false
done

if [[ "${complete}" == true ]]; then
  cargo run --locked --release -p tysel-bench-compare --bin tysel-bench-report -- \
    --input "${record_root}"/comparison-v1-${arch}-cycle${cycle}-seed*.json \
    --output "${summary}.tmp"
  mv "${summary}.tmp" "${summary}"
else
  echo "cycle ${cycle} remains incomplete; summary was not generated" >&2
fi
