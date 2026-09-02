#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 4 ]]; then
  echo "usage: $0 <version> <release-tag> <source-commit> <publish-directory>" >&2
  exit 2
fi

version="$1"
release_tag="$2"
source_commit="$3"
publish_directory="$4"
repository="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
runner_temp="${RUNNER_TEMP:?RUNNER_TEMP is required}"

[[ "$release_tag" == "v${version}" ]] || {
  echo "release tag ${release_tag} does not match version ${version}" >&2
  exit 1
}
[[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || {
  echo "source commit must be one canonical Git commit ID" >&2
  exit 1
}
[[ -d "$publish_directory" ]] || {
  echo "signed publish directory is missing" >&2
  exit 1
}

channel="$(bash "$(dirname "$0")/release-channel.sh" "$version")"
dist_tag="$(bash "$(dirname "$0")/npm-dist-tag.sh" "$version")"
bash "$(dirname "$0")/check-published-channel.sh" \
  "$version" "https://github.com/${repository}/releases"

# Promotion runs after publication but can share npm's cache with earlier
# workflow steps. Always revalidate registry metadata instead of trusting a
# cached missing-package or stale dist-tag response.
npm_registry_view() {
  npm view "$@" \
    --prefer-online \
    --fetch-retries=3 \
    --fetch-retry-mintimeout=1000 \
    --fetch-retry-maxtimeout=10000
}

npm_ready=false
for attempt in {1..90}; do
  npm_ready=true
  for package in '@tysel/types' '@tysel/test' '@tysel/sdk'; do
    if [[ "$(npm_registry_view "${package}@${version}" version 2>/dev/null || true)" != "$version" ]] \
      || [[ "$(npm_registry_view "${package}@${dist_tag}" version 2>/dev/null || true)" != "$version" ]]; then
      npm_ready=false
      break
    fi
  done
  [[ "$npm_ready" == true ]] && break
  [[ "$attempt" -lt 90 ]] || break
  sleep 10
done
[[ "$npm_ready" == true ]] || {
  echo "npm packages or dist-tag did not become visible after 15 minutes" >&2
  exit 1
}

crate_url="https://crates.io/api/v1/crates/tysel-component-sdk/${version}"
for attempt in {1..12}; do
  published_version="$(curl -fsSL \
    -H 'User-Agent: tysel-release-workflow (https://github.com/wangcch/tysel)' \
    "$crate_url" | jq -r '.version.num' 2>/dev/null || true)"
  [[ "$published_version" == "$version" ]] && break
  [[ "$attempt" -lt 12 ]] || {
    echo "tysel-component-sdk@${version} is not available on crates.io" >&2
    exit 1
  }
  sleep 5
done

gh release view "$release_tag" --json isDraft \
  --jq '.isDraft' | grep -Fx true > /dev/null

if gh release view trust >/dev/null 2>&1; then
  gh release upload trust \
    "$publish_directory/trust.json" "$publish_directory/trust.json.sig.json" --clobber
else
  gh release create trust \
    "$publish_directory/trust.json" "$publish_directory/trust.json.sig.json" \
    --prerelease --latest=false --target "$source_commit" \
    --title "Tysel release trust" \
    --notes "Authenticated moving trust-policy endpoint for all release channels."
fi
trust_base="https://github.com/${repository}/releases/download/trust"
bash "$(dirname "$0")/wait-for-published-trust.sh" \
  "$trust_base" \
  "$publish_directory/trust.json" "$publish_directory/trust.json.sig.json" \
  "$runner_temp/published-trust.json" \
  "$runner_temp/published-trust.json.sig.json"

if [[ "$channel" == stable ]]; then
  gh release edit "$release_tag" --draft=false --latest
  channel_tag="$release_tag"
else
  gh release edit "$release_tag" --draft=false --prerelease --latest=false
  channel_assets=(
    "$publish_directory/channel-pointer.json"
    "$publish_directory/channel-pointer.json.sig.json"
    "$publish_directory/canary-version"
    "$publish_directory/install.sh"
    "$publish_directory/install.sh.sha256"
  )
  if gh release view canary >/dev/null 2>&1; then
    gh release upload canary "${channel_assets[@]}" --clobber
  else
    gh release create canary "${channel_assets[@]}" \
      --prerelease --latest=false --target "$source_commit" \
      --title "Tysel canary" \
      --notes "Moving pointer to the latest authenticated Tysel canary release."
  fi
  channel_tag=canary
fi

install_url="https://github.com/${repository}/releases/download/${channel_tag}/install.sh"
checksum_url="${install_url}.sha256"
curl -fsSL --retry 3 "$install_url" > "$runner_temp/install.sh"
expected="$(curl -fsSL --retry 3 "$checksum_url" | tr -d '[:space:]')"
actual="$(sha256sum "$runner_temp/install.sh" | awk '{print $1}')"
[[ "$actual" == "$expected" ]]
