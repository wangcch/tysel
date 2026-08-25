#!/usr/bin/env bash
set -euo pipefail

: "${TYSEL_LLM_ENDPOINT:?set TYSEL_LLM_ENDPOINT to an OpenAI-compatible Responses endpoint}"
: "${TYSEL_LLM_MODEL:?set TYSEL_LLM_MODEL to the provider model}"
: "${OPENAI_API_KEY:?set OPENAI_API_KEY to the provider credential}"

example_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="$example_dir/tysel.toml"
binary="${TYSEL_BIN:-tysel}"
base_url="http://127.0.0.1:3000"
log_file="${TMPDIR:-/tmp}/tysel-durable-agent-$$.log"
app_pid=""

command -v "$binary" >/dev/null 2>&1 || {
  echo "Tysel is not installed or is not on PATH" >&2
  exit 1
}

cleanup() {
  if [[ -n "$app_pid" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill -TERM "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

wait_ready() {
  for _ in {1..100}; do
    if curl --silent --fail "$base_url/" >/dev/null 2>&1; then
      return
    fi
    if ! kill -0 "$app_pid" 2>/dev/null; then
      echo "Tysel exited before becoming ready" >&2
      sed -n '1,160p' "$log_file" >&2
      exit 1
    fi
    sleep 0.1
  done
  echo "Timed out waiting for $base_url" >&2
  sed -n '1,160p' "$log_file" >&2
  exit 1
}

start_app() {
  "$binary" run --manifest "$manifest" >"$log_file" 2>&1 &
  app_pid=$!
  wait_ready
}

stop_app() {
  kill -TERM "$app_pid"
  wait "$app_pid"
  app_pid=""
}

cd "$example_dir"

echo "1/5 Starting Tysel and calling the LLM"
start_app
started="$(curl --silent --fail \
  --request POST "$base_url/runs" \
  --header 'content-type: application/json' \
  --data '{"customerId":"customer-42","prompt":"Summarize this account in one sentence"}')"
run_id="$(printf '%s\n' "$started" \
  | sed -E 's/.*"runId"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
[[ -n "$run_id" && "$run_id" != "$started" ]] || {
  echo "Response did not contain runId: $started" >&2
  exit 1
}
echo "$started"

echo "2/5 Confirming the task is suspended for human approval"
waiting="$(curl --silent --fail "$base_url/runs/$run_id")"
waiting_status="$(printf '%s\n' "$waiting" \
  | sed -E 's/.*"status"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
[[ "$waiting_status" == "awaiting_approval" ]] || {
  echo "Expected awaiting_approval: $waiting" >&2
  exit 1
}
echo "$waiting"

echo "3/5 Stopping the process and starting a fresh process"
stop_app
start_app

echo "4/5 Sending human approval after restart"
curl --silent --fail \
  --request POST "$base_url/runs/$run_id/approval" \
  --header 'content-type: application/json' \
  --data '{"approved":true}'
echo

echo "5/5 Waiting for the replayed task to save its result exactly once"
for _ in {1..100}; do
  current="$(curl --silent --fail "$base_url/runs/$run_id")"
  status="$(printf '%s\n' "$current" \
    | sed -E 's/.*"status"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
  if [[ "$status" == "completed" ]]; then
    printf '%s\n' "$current" | grep -Eq '"saveCount"[[:space:]]*:[[:space:]]*1([,}])' || {
      echo "Expected saveCount=1: $current" >&2
      exit 1
    }
    echo "$current"
    echo "Durable Agent Golden Path completed."
    exit 0
  fi
  sleep 0.1
done

echo "Timed out waiting for run $run_id" >&2
echo "$current" >&2
exit 1
