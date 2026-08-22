#!/usr/bin/env bash
set -euo pipefail

workspace_metadata="$(cargo metadata --locked --format-version 1 --no-deps)"
rust_version_count="$(jq -r '[.packages[].version] | unique | length' <<< "$workspace_metadata")"
rust_versions="$(jq -r '[.packages[].version] | unique | join(", ")' <<< "$workspace_metadata")"
if [[ "$rust_version_count" -ne 1 ]]; then
  echo "Rust workspace packages do not share one version: ${rust_versions}" >&2
  exit 1
fi

version="$rust_versions"
for manifest in \
  packages/tysel/package.json \
  packages/tysel-compat/package.json \
  packages/tysel-test/package.json \
  packages/tysel-types/package.json \
  runtime-js/package.json; do
  package_version="$(jq -er '.version' "$manifest")"
  if [[ "$package_version" != "$version" ]]; then
    echo "${manifest} version ${package_version} does not match Rust ${version}" >&2
    exit 1
  fi
done

if [[ -n "${GITHUB_REF_NAME:-}" && "${GITHUB_REF_TYPE:-}" == "tag" ]]; then
  if [[ "$GITHUB_REF_NAME" != "v${version}" ]]; then
    echo "release tag ${GITHUB_REF_NAME} does not match workspace version v${version}" >&2
    exit 1
  fi
  if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "stable release tags must use a final semantic version, got ${version}" >&2
    exit 1
  fi
fi

printf '%s\n' "$version"
