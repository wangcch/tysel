#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <semantic-version>" >&2
  exit 2
fi

case "$(bash "$(dirname "$0")/release-channel.sh" "$1")" in
  stable) printf '%s\n' latest ;;
  canary) printf '%s\n' canary ;;
esac
