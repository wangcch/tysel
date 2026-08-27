#!/usr/bin/env bash
set -euo pipefail

expected_sha="${1:-}"
root="$(cd "$(dirname "$0")/../.." && pwd)"

if ! [[ "$expected_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "verify-current-main: expected a 40-character commit SHA" >&2
  exit 2
fi

current_sha="$(git -C "$root" ls-remote origin refs/heads/main | awk 'NR == 1 { print $1 }')"
if ! [[ "$current_sha" =~ ^[0-9a-f]{40}$ ]]; then
  echo "verify-current-main: failed to resolve origin/main" >&2
  exit 1
fi

if [ "$expected_sha" != "$current_sha" ]; then
  echo "verify-current-main: refusing stale deployment" >&2
  echo "trigger: $expected_sha" >&2
  echo "main:    $current_sha" >&2
  exit 1
fi

echo "verify-current-main: $expected_sha is current"
