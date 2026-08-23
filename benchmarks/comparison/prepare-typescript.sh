#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tsc="${root}/node_modules/.bin/tsc"
expected_version="$(jq -r '.toolchains[] | select(.id == "typescript") | .expectedVersion' "${root}/benchmarks/comparison/runtimes.lock.json")"
expected="Version ${expected_version}"

if [[ ! -x "${tsc}" ]]; then
  echo "TypeScript 7 is not installed; run 'pnpm install --frozen-lockfile'" >&2
  exit 1
fi

actual="$(${tsc} --version)"
if [[ "${actual}" != "${expected}" ]]; then
  echo "TypeScript version mismatch: expected '${expected}', got '${actual}'" >&2
  exit 1
fi

exec "${tsc}" --project "${root}/benchmarks/comparison/tsconfig.json"
