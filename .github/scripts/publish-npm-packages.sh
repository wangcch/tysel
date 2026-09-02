#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <package-directory> <version>" >&2
  exit 2
fi

package_directory="$1"
version="$2"
[[ -d "$package_directory" ]] || { echo "npm package directory is missing" >&2; exit 1; }
dist_tag="$(bash "$(dirname "$0")/npm-dist-tag.sh" "$version")"
# Publish the newly introduced SDK identity first. If its scope permissions or
# name are misconfigured, no companion package for this version is published.
packages=('@tysel/sdk' '@tysel/types' '@tysel/test')

# A publish job queries these identities before and after publication. Force npm
# to revalidate registry responses so a cached pre-publish 404 cannot make the
# post-publish visibility check fail spuriously.
npm_registry_view() {
  npm view "$@" \
    --prefer-online \
    --fetch-retries=3 \
    --fetch-retry-mintimeout=1000 \
    --fetch-retry-maxtimeout=10000
}

archive_for() {
  local name="$1"
  local archive_name="tysel-${name#@tysel/}-${version}.tgz"
  find "$package_directory" -maxdepth 1 -type f -name "$archive_name" -print -quit
}

integrity_for() {
  local archive="$1"
  printf 'sha512-'
  openssl dgst -sha512 -binary "$archive" | openssl base64 -A
}

if [[ "${TYSEL_NPM_DRY_RUN:-0}" != 1 ]]; then
  for name in "${packages[@]}"; do
    current_tagged="$(npm_registry_view "${name}@${dist_tag}" version 2>/dev/null || true)"
    if [[ -n "$current_tagged" ]]; then
      precedence="$(bash "$(dirname "$0")/semver-precedence.sh" "$version" "$current_tagged")"
      [[ "$precedence" != older ]] || {
        echo "refusing to move ${name} dist-tag ${dist_tag} backward from ${current_tagged} to ${version}" >&2
        exit 1
      }
    fi
  done
fi

for name in "${packages[@]}"; do
  archive="$(archive_for "$name")"
  [[ -n "$archive" ]] || { echo "packed artifact is missing for ${name}@${version}" >&2; exit 1; }
  packed_name="$(tar -xOf "$archive" package/package.json | jq -er '.name')"
  packed_version="$(tar -xOf "$archive" package/package.json | jq -er '.version')"
  [[ "$packed_name" == "$name" && "$packed_version" == "$version" ]] || {
    echo "packed npm identity mismatch for ${name}" >&2
    exit 1
  }

  local_integrity="$(integrity_for "$archive")"
  remote_integrity="$(npm_registry_view "${name}@${version}" dist.integrity --json 2>/dev/null \
    | jq -r 'if type == "string" then . else empty end' || true)"
  if [[ -n "$remote_integrity" ]]; then
    [[ "$remote_integrity" == "$local_integrity" ]] || {
      echo "registry already contains different bytes for ${name}@${version}" >&2
      exit 1
    }
    echo "verified existing ${name}@${version}"
  else
    if [[ "${TYSEL_NPM_DRY_RUN:-0}" == 1 ]]; then
      echo "validated unpublished ${name}@${version}"
    else
      npm publish "$archive" --access public --provenance --tag "$dist_tag"
    fi
  fi
done

if [[ "${TYSEL_NPM_DRY_RUN:-0}" != 1 ]]; then
  npm_ready=false
  for attempt in {1..90}; do
    npm_ready=true
    for name in "${packages[@]}"; do
      archive="$(archive_for "$name")"
      local_integrity="$(integrity_for "$archive")"
      published_version="$(npm_registry_view "${name}@${version}" version 2>/dev/null || true)"
      tagged_version="$(npm_registry_view "${name}@${dist_tag}" version 2>/dev/null || true)"
      remote_integrity="$(npm_registry_view "${name}@${version}" dist.integrity --json 2>/dev/null \
        | jq -r 'if type == "string" then . else empty end' || true)"
      if [[ -n "$remote_integrity" && "$remote_integrity" != "$local_integrity" ]]; then
        echo "registry contains different bytes for ${name}@${version}" >&2
        exit 1
      fi
      if [[ "$published_version" != "$version" || "$tagged_version" != "$version" \
        || "$remote_integrity" != "$local_integrity" ]]; then
        npm_ready=false
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
fi
