#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <package-directory> <version>" >&2
  exit 2
fi

package_directory="$1"
version="$2"
crate_name="tysel-component-sdk"
archive="${package_directory}/${crate_name}-${version}.crate"

[[ -f "$archive" ]] || {
  echo "packed crate is missing for ${crate_name}@${version}" >&2
  exit 1
}

cargo package --locked --package "$crate_name"
generated="target/package/${crate_name}-${version}.crate"
[[ -f "$generated" ]] || {
  echo "cargo package did not create ${generated}" >&2
  exit 1
}
cmp -s "$archive" "$generated" || {
  echo "downloaded and locally generated crate archives differ" >&2
  exit 1
}

local_checksum="$(sha256sum "$archive" | awk '{print $1}')"
response="$(mktemp)"
trap 'rm -f "$response"' EXIT
status="$(curl -sS -o "$response" -w '%{http_code}' \
  -H 'User-Agent: tysel-release-workflow (https://github.com/wangcch/tysel)' \
  "https://crates.io/api/v1/crates/${crate_name}/${version}")"

case "$status" in
  200)
    remote_checksum="$(jq -er '.version.checksum' "$response")"
    [[ "$remote_checksum" == "$local_checksum" ]] || {
      echo "crates.io already contains different bytes for ${crate_name}@${version}" >&2
      exit 1
    }
    echo "verified existing ${crate_name}@${version}"
    ;;
  404)
    if [[ "${TYSEL_CARGO_DRY_RUN:-0}" == 1 ]]; then
      echo "validated unpublished ${crate_name}@${version}"
    else
      cargo publish --locked --package "$crate_name"
    fi
    ;;
  *)
    echo "cannot query crates.io for ${crate_name}@${version}: HTTP ${status}" >&2
    exit 1
    ;;
esac
