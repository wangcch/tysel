#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <candidate-version> <github-releases-base-url>" >&2
  exit 2
fi

candidate="$1"
base="${2%/}"
script_dir="$(cd "$(dirname "$0")" && pwd)"
channel="$(bash "$script_dir/release-channel.sh" "$candidate")"
case "$base" in
  https://*) ;;
  *) echo "release base URL must use HTTPS" >&2; exit 2 ;;
esac
if [[ "$channel" == stable ]]; then
  pointer_url="${base}/latest/download/channel-pointer.json"
else
  pointer_url="${base}/download/canary/channel-pointer.json"
fi

temporary="$(mktemp)"
trap 'rm -f "$temporary"' EXIT
status="$(curl -sS -L -o "$temporary" -w '%{http_code}' "$pointer_url")"
if [[ "$status" == 404 ]]; then
  echo "no published ${channel} channel pointer; ${candidate} may initialize it"
  exit 0
elif [[ "$status" != 200 ]]; then
  echo "cannot read published ${channel} channel pointer: HTTP ${status}" >&2
  exit 1
fi

current="$(jq -er --arg channel "$channel" \
  'select(.schemaVersion == 1 and .channel == $channel) | .version' "$temporary")"
[[ "$(bash "$script_dir/release-channel.sh" "$current")" == "$channel" ]] || {
  echo "published ${channel} pointer contains an incompatible version: ${current}" >&2
  exit 1
}
precedence="$(bash "$script_dir/semver-precedence.sh" "$candidate" "$current")"
[[ "$precedence" != older ]] || {
  echo "refusing to move ${channel} backward from ${current} to ${candidate}" >&2
  exit 1
}
echo "published ${channel} is ${current}; candidate ${candidate} is ${precedence}"
