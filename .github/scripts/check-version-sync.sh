#!/usr/bin/env bash
set -euo pipefail

workspace_metadata="$(cargo metadata --locked --format-version 1 --no-deps)"
rust_version_count="$(jq -r '[.packages[].version] | unique | length' <<< "$workspace_metadata")"
rust_versions="$(jq -r '[.packages[].version] | unique | join(", ")' <<< "$workspace_metadata")"
if [[ "$rust_version_count" -ne 1 ]]; then
  echo "Rust workspace packages do not share one version: ${rust_versions}" >&2
  exit 1
fi

public_rust_crates="$(jq -r '
  [.packages[] | select(.publish != []) | .name] | sort | join(",")
' <<< "$workspace_metadata")"
if [[ "$public_rust_crates" != "tysel-component-sdk" ]]; then
  echo "unexpected publishable Rust crates: ${public_rust_crates:-none}" >&2
  exit 1
fi
sdk_registries="$(jq -r '
  .packages[] | select(.name == "tysel-component-sdk") | .publish | join(",")
' <<< "$workspace_metadata")"
if [[ "$sdk_registries" != "crates-io" ]]; then
  echo "tysel-component-sdk must publish only to crates.io" >&2
  exit 1
fi

version="$rust_versions"
expected_npm_packages=(
  "packages/tysel/package.json:@tysel/sdk"
  "packages/tysel-test/package.json:@tysel/test"
  "packages/tysel-types/package.json:@tysel/types"
  "runtime-js/package.json:@tysel/runtime-js"
)
for package_entry in "${expected_npm_packages[@]}"; do
  manifest="${package_entry%%:*}"
  expected_name="${package_entry#*:}"
  package_name="$(jq -er '.name' "$manifest")"
  if [[ "$package_name" != "$expected_name" ]]; then
    echo "${manifest} name ${package_name} does not match ${expected_name}" >&2
    exit 1
  fi
done

for manifest in \
  packages/tysel/package.json \
  packages/tysel-test/package.json \
  packages/tysel-types/package.json \
  runtime-js/package.json; do
  package_version="$(jq -er '.version' "$manifest")"
  if [[ "$package_version" != "$version" ]]; then
    echo "${manifest} version ${package_version} does not match Rust ${version}" >&2
    exit 1
  fi
done

runtime_js_version="$(jq -er '.runtimeJsVersion' runtime-js/compatibility.json)"
if [[ "$runtime_js_version" != "$version" ]]; then
  echo "runtime-js compatibility version ${runtime_js_version} does not match Rust ${version}" >&2
  exit 1
fi

comparison_runtime_locks=(
  benchmarks/comparison/runtimes.lock.json
  benchmarks/comparison/runtimes-tysel-workers-2.json
  benchmarks/comparison/runtimes-tysel-workers-4.json
)
for runtime_lock in "${comparison_runtime_locks[@]}"; do
  comparison_version="$(jq -er '
    .runtimes
    | map(select(.id == "tysel"))
    | if length == 1 then .[0].expectedVersion else error("expected exactly one tysel runtime") end
  ' "$runtime_lock")"
  if [[ "$comparison_version" != "$version" ]]; then
    echo "${runtime_lock} Tysel version ${comparison_version} does not match Rust ${version}" >&2
    exit 1
  fi
done

for license_copy in \
  crates/tysel-component-sdk/LICENSE \
  packages/tysel/LICENSE \
  packages/tysel-test/LICENSE \
  packages/tysel-types/LICENSE; do
  if ! cmp -s LICENSE "$license_copy"; then
    echo "${license_copy} must match the repository LICENSE" >&2
    exit 1
  fi
done

if [[ -n "${GITHUB_REF_NAME:-}" && "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  if [[ "$GITHUB_REF_NAME" != "v${version}" ]]; then
    echo "release tag ${GITHUB_REF_NAME} does not match workspace version v${version}" >&2
    exit 1
  fi
  bash "$(dirname "$0")/release-channel.sh" "$version" > /dev/null
fi

printf '%s\n' "$version"
