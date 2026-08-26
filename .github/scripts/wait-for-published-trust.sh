#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "usage: $0 <trust-base-url> <expected-policy> <expected-signature> <published-policy> <published-signature>" >&2
  exit 2
fi

base="${1%/}"
expected_policy_path="$2"
expected_signature_path="$3"
published_policy_path="$4"
published_signature_path="$5"

[[ "$base" == https://* ]] || { echo "trust base URL must use HTTPS" >&2; exit 2; }
[[ -f "$expected_policy_path" && -f "$expected_signature_path" ]] || {
  echo "expected trust policy or signature is missing" >&2
  exit 2
}

expected_policy="$(sha256sum "$expected_policy_path" | awk '{print $1}')"
expected_signature="$(sha256sum "$expected_signature_path" | awk '{print $1}')"
nonce="$(date -u +%s)-$$"
for attempt in {1..12}; do
  if curl -fsSL --retry 3 --connect-timeout 10 --max-time 30 \
      -H 'Cache-Control: no-cache' \
      "${base}/trust.json?refresh=${nonce}-${attempt}" -o "$published_policy_path" \
    && curl -fsSL --retry 3 --connect-timeout 10 --max-time 30 \
      -H 'Cache-Control: no-cache' \
      "${base}/trust.json.sig.json?refresh=${nonce}-${attempt}" -o "$published_signature_path"; then
    actual_policy="$(sha256sum "$published_policy_path" | awk '{print $1}')"
    actual_signature="$(sha256sum "$published_signature_path" | awk '{print $1}')"
    if [[ "$actual_policy" == "$expected_policy" && "$actual_signature" == "$expected_signature" ]]; then
      echo "published trust policy and signature are visible"
      exit 0
    fi
  fi
  [[ "$attempt" -lt 12 ]] || {
    echo "published trust policy did not match the uploaded assets" >&2
    exit 1
  }
  sleep 10
done
