#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <package-directory> <version>" >&2
  exit 2
fi

package_directory="$1"
version="$2"
[[ -d "$package_directory" ]] || { echo "npm package directory is missing" >&2; exit 1; }

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
      npm publish "$archive" --access public --provenance
    fi
  fi
done
