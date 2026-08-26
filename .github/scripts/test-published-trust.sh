#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/fake-bin"

printf '%s\n' '{"policy":"expected"}' > "$fixture/trust.json"
printf '%s\n' '{"signature":"expected"}' > "$fixture/trust.json.sig.json"
printf '%s\n' '{"policy":"stale"}' > "$fixture/stale-trust.json"
cat > "$fixture/fake-bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
output=
url=
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o) output="$2"; shift 2 ;;
    https://*) url="$1"; shift ;;
    *) shift ;;
  esac
done
[[ -n "$output" && -n "$url" ]]
if [[ "${TEST_CURL_FAIL_ONCE:-0}" == 1 && ! -e "$TEST_CURL_MARKER" ]]; then
  touch "$TEST_CURL_MARKER"
  exit 22
fi
if [[ "$url" == *trust.json.sig.json* ]]; then
  cp "$TEST_TRUST_SIGNATURE" "$output"
else
  cp "$TEST_TRUST_POLICY" "$output"
fi
EOF
printf '%s\n' '#!/bin/sh' 'exit 0' > "$fixture/fake-bin/sleep"
chmod +x "$fixture/fake-bin/curl" "$fixture/fake-bin/sleep"

export TEST_TRUST_POLICY="$fixture/trust.json"
export TEST_TRUST_SIGNATURE="$fixture/trust.json.sig.json"
export TEST_CURL_FAIL_ONCE=1
export TEST_CURL_MARKER="$fixture/curl-failed-once"
PATH="$fixture/fake-bin:$PATH" bash "$script_dir/wait-for-published-trust.sh" \
  https://example.invalid/releases/download/trust \
  "$fixture/trust.json" "$fixture/trust.json.sig.json" \
  "$fixture/published.json" "$fixture/published.json.sig.json" >/dev/null
cmp "$fixture/trust.json" "$fixture/published.json"
cmp "$fixture/trust.json.sig.json" "$fixture/published.json.sig.json"

export TEST_CURL_FAIL_ONCE=0
export TEST_TRUST_POLICY="$fixture/stale-trust.json"
if PATH="$fixture/fake-bin:$PATH" bash "$script_dir/wait-for-published-trust.sh" \
  https://example.invalid/releases/download/trust \
  "$fixture/trust.json" "$fixture/trust.json.sig.json" \
  "$fixture/published.json" "$fixture/published.json.sig.json" >/dev/null 2>&1; then
  echo "published trust check accepted stale policy bytes" >&2
  exit 1
fi

if bash "$script_dir/wait-for-published-trust.sh" \
  http://example.invalid/releases/download/trust \
  "$fixture/trust.json" "$fixture/trust.json.sig.json" \
  "$fixture/published.json" "$fixture/published.json.sig.json" >/dev/null 2>&1; then
  echo "published trust check accepted an insecure URL" >&2
  exit 1
fi

echo "published trust visibility tests passed"
