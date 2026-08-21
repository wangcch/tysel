#!/usr/bin/env bash
set -euo pipefail

: "${TYSEL_LLM_ENDPOINT:?set TYSEL_LLM_ENDPOINT to an OpenAI-compatible Responses endpoint}"
: "${TYSEL_LLM_MODEL:?set TYSEL_LLM_MODEL to the provider model}"
: "${OPENAI_API_KEY:?set OPENAI_API_KEY to the provider credential}"

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
manifest="$repo_dir/examples/durable-agent/tysel.toml"
binary="$repo_dir/target/debug/tysel"
base_url="http://127.0.0.1:3000"
log_file="${TMPDIR:-/tmp}/tysel-durable-agent-$$.log"
app_pid=""

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

cd "$repo_dir"
cargo build --quiet -p tysel-cli

echo "1/5 Starting Tysel and calling the LLM"
start_app
started="$(curl --silent --fail \
  --request POST "$base_url/runs" \
  --header 'content-type: application/json' \
  --data '{"customerId":"customer-42","prompt":"Summarize this account in one sentence"}')"
run_id="$(RUN_JSON="$started" node -e 'process.stdout.write(JSON.parse(process.env.RUN_JSON).runId)')"
echo "$started"

echo "2/5 Confirming the task is suspended for human approval"
waiting="$(curl --silent --fail "$base_url/runs/$run_id")"
RUN_JSON="$waiting" node -e '
  const run = JSON.parse(process.env.RUN_JSON);
  if (run.status !== "awaiting_approval") throw new Error(JSON.stringify(run));
'
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
  status="$(RUN_JSON="$current" node -e 'process.stdout.write(JSON.parse(process.env.RUN_JSON).status)')"
  if [[ "$status" == "completed" ]]; then
    RUN_JSON="$current" node -e '
      const run = JSON.parse(process.env.RUN_JSON);
      if (run.saveCount !== 1) throw new Error(`expected saveCount=1: ${JSON.stringify(run)}`);
      console.log(JSON.stringify(run, null, 2));
    '
    echo "Durable Agent Golden Path completed."
    exit 0
  fi
  sleep 0.1
done

echo "Timed out waiting for run $run_id" >&2
echo "$current" >&2
exit 1
