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
if [[ "${TYSEL_NPM_DRY_RUN:-0}" != 1 ]]; then
  for name in '@tysel/types' '@tysel/test'; do
    current_tagged="$(npm view "${name}@${dist_tag}" version 2>/dev/null || true)"
    if [[ -n "$current_tagged" ]]; then
      precedence="$(bash "$(dirname "$0")/semver-precedence.sh" "$version" "$current_tagged")"
      [[ "$precedence" != older ]] || {
        echo "refusing to move ${name} dist-tag ${dist_tag} backward from ${current_tagged} to ${version}" >&2
        exit 1
      }
    fi
  done
fi

for name in '@tysel/types' '@tysel/test'; do
  archive_name="tysel-${name#@tysel/}-${version}.tgz"
  archive="$(find "$package_directory" -maxdepth 1 -type f \
    -name "$archive_name" -print -quit)"
  [[ -n "$archive" ]] || { echo "packed artifact is missing for ${name}@${version}" >&2; exit 1; }
  packed_name="$(tar -xOf "$archive" package/package.json | jq -er '.name')"
  packed_version="$(tar -xOf "$archive" package/package.json | jq -er '.version')"
  [[ "$packed_name" == "$name" && "$packed_version" == "$version" ]] || {
    echo "packed npm identity mismatch for ${name}" >&2
    exit 1
  }

  local_integrity="sha512-$(openssl dgst -sha512 -binary "$archive" | openssl base64 -A)"
  remote_integrity="$(npm view "${name}@${version}" dist.integrity --json 2>/dev/null \
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
  if [[ "${TYSEL_NPM_DRY_RUN:-0}" != 1 ]]; then
    tagged_version=
    for attempt in {1..12}; do
      tagged_version="$(npm view "${name}@${dist_tag}" version 2>/dev/null || true)"
      [[ "$tagged_version" == "$version" ]] && break
      [[ "$attempt" -lt 12 ]] || break
      sleep 5
    done
    [[ "$tagged_version" == "$version" ]] || {
      echo "${name} dist-tag ${dist_tag} points to ${tagged_version:-nothing}, not ${version}" >&2
      echo "refusing to move a release channel implicitly; repair it explicitly after review" >&2
      exit 1
    }
  fi
done
