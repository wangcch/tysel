#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 6 ]]; then
  echo "usage: $0 <release-output> <version> <source-commit> <published-at> <base-url> <output>" >&2
  exit 2
fi

release_output="$1"
version="$2"
source_commit="$3"
published_at="$4"
base_url="${5%/}"
output="$6"

channel="$(bash "$(dirname "$0")/release-channel.sh" "$version")"
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || { echo "invalid source commit" >&2; exit 2; }
[[ "$published_at" == *T*Z ]] || { echo "published-at must be UTC RFC 3339" >&2; exit 2; }
[[ "$base_url" == https://* ]] || { echo "base URL must use HTTPS" >&2; exit 2; }

temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

sha256_file() {
  if command -v sha256sum > /dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

asset_count=0
for metadata in "$release_output"/release-asset-*.json; do
  [[ -f "$metadata" ]] || { echo "release asset metadata is missing" >&2; exit 1; }
  target="$(jq -er '.target' "$metadata")"
  archive_name="tysel-${version}-${target}.tar.gz"
  archive="${release_output}/${archive_name}"
  signature="${archive}.sig.json"
  [[ -f "$archive" && -f "$signature" ]] || {
    echo "signed archive is incomplete for ${target}" >&2
    exit 1
  }
  archive_sha256="$(sha256_file "$archive")"
  archive_size="$(wc -c < "$archive" | tr -d '[:space:]')"
  jq -e --arg sha256 "$archive_sha256" '.artifact_sha256 == $sha256' "$signature" > /dev/null
  jq \
    --arg archiveUrl "${base_url}/${archive_name}" \
    --argjson byteSize "$archive_size" \
    --arg sha256 "$archive_sha256" \
    --arg signatureUrl "${base_url}/${archive_name}.sig.json" \
    --slurpfile signature "$signature" \
    '. + {
      archiveUrl: $archiveUrl,
      byteSize: $byteSize,
      sha256: $sha256,
      signature: {
        algorithm: "ed25519",
        url: $signatureUrl,
        keyId: $signature[0].key_id
      }
    }' "$metadata" > "${temporary}/${target}.json"
  asset_count=$((asset_count + 1))
done

[[ "$asset_count" -gt 0 ]] || { echo "release has no assets" >&2; exit 1; }
jq -s \
  --arg version "$version" \
  --arg sourceCommit "$source_commit" \
  --arg publishedAt "$published_at" \
  --arg channel "$channel" \
  '{
    schemaVersion: 1,
    version: $version,
    sourceCommit: $sourceCommit,
    publishedAt: $publishedAt,
    channel: $channel,
    minimumUpdaterVersion: "0.0.1",
    compatibility: {
      minimumTapVersion: 1,
      maximumTapVersion: 3,
      capabilityAbiVersion: "0.4.0",
      typesVersion: $version
    },
    requiredFeatures: ["atomic-bin-link", "build-info-v1", "ed25519-manifest"],
    optionalFeatures: {},
    assets: sort_by(.target)
  }' "$temporary"/*.json > "$output"
