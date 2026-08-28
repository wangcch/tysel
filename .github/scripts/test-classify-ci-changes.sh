#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
classifier="$root/.github/scripts/classify-ci-changes.sh"
fixture="$(mktemp)"
trap 'rm -f "$fixture"' EXIT

assert_case() {
  local name="$1"
  local expected_mode="$2"
  local expected_docs_only expected_full
  local output
  shift 2

  if [[ "$expected_mode" == docs-only ]]; then
    expected_docs_only=true
    expected_full=false
  else
    expected_docs_only=false
    expected_full=true
  fi

  printf '%s\n' "$@" > "$fixture"
  output="$(bash "$classifier" "$fixture")"
  if ! grep -Fx "mode=${expected_mode}" <<< "$output" > /dev/null \
    || ! grep -Fx "docs_only=${expected_docs_only}" <<< "$output" > /dev/null \
    || ! grep -Fx "full=${expected_full}" <<< "$output" > /dev/null \
    || ! grep -Fx "benchmark_required=${expected_full}" <<< "$output" > /dev/null; then
    echo "classification case failed: $name" >&2
    echo "$output" >&2
    exit 1
  fi
}

assert_case docs docs-only docs/guides/install.md docs/reference/cli.md
assert_case website docs-only website/app/page.tsx website/pnpm-lock.yaml
assert_case brand docs-only brand/logo.svg README.md mkdocs.yml
assert_case rust full docs/guides/install.md crates/tysel-cli/src/main.rs
assert_case dependency full Cargo.lock
assert_case gitattributes full .gitattributes
assert_case example full examples/hello-service/src/index.ts
assert_case workflow full .github/workflows/ci.yml
assert_case release-script full .github/scripts/reproducible-release.sh
assert_case empty full

echo "CI change classification tests passed"
