#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <semantic-version>" >&2
  exit 2
fi

version="$1"
if [[ ! "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-([0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*))?$ ]]; then
  echo "release version must be canonical SemVer without a v prefix or build metadata: ${version}" >&2
  exit 2
fi

prerelease="${BASH_REMATCH[5]:-}"
if [[ -z "$prerelease" ]]; then
  printf '%s\n' stable
  exit 0
fi

IFS=. read -r -a identifiers <<< "$prerelease"
for identifier in "${identifiers[@]}"; do
  if [[ "$identifier" =~ ^[0-9]+$ && "$identifier" != 0 && "$identifier" == 0* ]]; then
    echo "numeric prerelease identifiers must not contain leading zeroes: ${version}" >&2
    exit 2
  fi
done
printf '%s\n' canary
