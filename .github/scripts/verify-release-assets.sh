#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <release-output> <version>" >&2
  exit 2
fi

release_output="$1"
version="$2"
[[ -d "$release_output" ]] || { echo "release output directory is missing" >&2; exit 1; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
  echo "stable release version must be a final semantic version" >&2
  exit 2
}

temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
expected="$temporary/expected"
actual="$temporary/actual"
: > "$expected"

for target in linux-x64 linux-arm64 darwin-x64 darwin-arm64; do
  archive="tysel-${version}-${target}.tar.gz"
  printf '%s\n' \
    "release-asset-${target}.json" \
    "$archive" \
    "${archive}.sha256" \
    "${archive}.repro.json" \
    "${archive}.repro.json.sig.json" \
    "${archive}.sig.json" >> "$expected"
done
for target in linux-x64 linux-arm64; do
  printf '%s\n' \
    "benchmark-evidence-${target}.json" \
    "benchmark-evidence-${target}.json.sig.json" \
    "security-evidence-${target}.json" \
    "security-evidence-${target}.json.sig.json" >> "$expected"
done
printf '%s\n' \
  channel-pointer.json \
  channel-pointer.json.sig.json \
  install.sh \
  install.sh.sha256 \
  release-manifest.json \
  release-manifest.json.sig.json \
  stable-version \
  trust.json \
  trust.json.sig.json \
  tysel-component-starters.tar.gz \
  tysel-component-starters.tar.gz.sha256 \
  tysel-component-starters.tar.gz.sig.json >> "$expected"
sort -o "$expected" "$expected"

find "$release_output" -maxdepth 1 -type f -exec basename {} \; | sort > "$actual"
if ! diff -u "$expected" "$actual"; then
  echo "signed release asset inventory is incomplete or contains unexpected files" >&2
  exit 1
fi

[[ "$(tr -d '[:space:]' < "$release_output/stable-version")" == "$version" ]] || {
  echo "stable-version does not match the release version" >&2
  exit 1
}
install_sha256="$(sha256sum "$release_output/install.sh" | awk '{print $1}')"
[[ "$(tr -d '[:space:]' < "$release_output/install.sh.sha256")" == "$install_sha256" ]] || {
  echo "install.sh checksum does not match" >&2
  exit 1
}
starters="$release_output/tysel-component-starters.tar.gz"
starters_sha256="$(sha256sum "$starters" | awk '{print $1}')"
[[ "$(tr -d '[:space:]' < "${starters}.sha256")" == "$starters_sha256" ]] || {
  echo "Component starter archive checksum does not match" >&2
  exit 1
}
signature_sha256="$(jq -er '.document_sha256' "${starters}.sig.json")"
[[ "$signature_sha256" == "$starters_sha256" ]] || {
  echo "Component starter signature does not describe the archive" >&2
  exit 1
}
for member in \
  tysel-component-starters/LICENSE \
  tysel-component-starters/rust-echo/wit/component/task.wit \
  tysel-component-starters/go-echo/wit/component/task.wit; do
  tar -tzf "$starters" | grep -Fx "$member" > /dev/null || {
    echo "Component starter archive is missing ${member}" >&2
    exit 1
  }
done

for target in linux-x64 linux-arm64 darwin-x64 darwin-arm64; do
  archive="$release_output/tysel-${version}-${target}.tar.gz"
  sha256="$(sha256sum "$archive" | awk '{print $1}')"
  [[ "$(tr -d '[:space:]' < "${archive}.sha256")" == "$sha256" ]] || {
    echo "archive checksum does not match for ${target}" >&2
    exit 1
  }
  jq -e --arg target "$target" --arg sha256 "$sha256" \
    '.target == $target and .artifact_sha256 == $sha256' "${archive}.sig.json" > /dev/null
done

jq -e --arg version "$version" '
  .schemaVersion == 1 and .channel == "stable" and .version == $version
  and ([.assets[].target] | sort) == ["darwin-arm64", "darwin-x64", "linux-arm64", "linux-x64"]' \
  "$release_output/release-manifest.json" > /dev/null
jq -e --arg version "$version" '
  .schemaVersion == 1 and .channel == "stable" and .version == $version' \
  "$release_output/channel-pointer.json" > /dev/null
jq -e '
  .policy_version == 1
  and ([.keys[] | select(.status == "active")] | length) == 1
  and all(.keys[]; .status == "active" or .status == "retired")' \
  "$release_output/trust.json" > /dev/null

active_key="$(jq -er '.keys[] | select(.status == "active") | .key_id' "$release_output/trust.json")"
for document in \
  release-manifest.json \
  channel-pointer.json \
  tysel-component-starters.tar.gz; do
  jq -e --arg key "$active_key" '.key_id == $key' \
    "$release_output/${document}.sig.json" > /dev/null
done
for target in linux-x64 linux-arm64 darwin-x64 darwin-arm64; do
  archive="$release_output/tysel-${version}-${target}.tar.gz"
  jq -e --arg key "$active_key" '.key_id == $key' "${archive}.sig.json" > /dev/null
  jq -e --arg key "$active_key" '.key_id == $key' \
    "${archive}.repro.json.sig.json" > /dev/null
done
for target in linux-x64 linux-arm64; do
  for document in benchmark-evidence security-evidence; do
    jq -e --arg key "$active_key" '.key_id == $key' \
      "$release_output/${document}-${target}.json.sig.json" > /dev/null
  done
done
trust_key="$(jq -er '.key_id' "$release_output/trust.json.sig.json")"
jq -e --arg key "$trust_key" \
  'any(.keys[]; .key_id == $key and .status != "revoked")' \
  "$release_output/trust.json" > /dev/null

echo "verified complete signed release asset inventory for ${version}"
