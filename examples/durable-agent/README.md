# Durable Agent Golden Path

This example demonstrates the value proposition that is specific to Tysel:

1. start a durable TypeScript agent;
2. call an OpenAI-compatible LLM through the native LLM capability;
3. persist the draft and suspend without keeping an isolate resident;
4. stop and restart the Tysel process;
5. deliver a human approval signal;
6. replay completed effects, save the result once, and finish.

The application database is `data/tysel.db`. The durable event log is
`data/durable-events.db`. Keeping them separate makes the demo observable:
the first stores user-facing run state, while the second owns replay, signals,
wakeups, and immutable task programs.

## Run the complete demonstration

From this example directory, set a real OpenAI-compatible endpoint, model, and
credential:

```bash
export TYSEL_LLM_ENDPOINT=https://api.openai.com/v1/responses
export TYSEL_LLM_MODEL=YOUR_MODEL
export OPENAI_API_KEY=YOUR_KEY
./demo.sh
```

`demo.sh` uses the installed `tysel` command, starts a run, verifies that it is
waiting for approval, stops the process, starts a new process over the same
stores, sends approval, and polls until the saved result is visible. Set
`TYSEL_BIN` only when the installed command has a nonstandard name or path. The
script deliberately does not provide a fake draft when LLM configuration
fails.

Optional provider settings:

```bash
export TYSEL_LLM_ALIAS=default
export TYSEL_LLM_SECRET=OPENAI_API_KEY
```

## HTTP API

Start a run:

```http
POST /runs
Content-Type: application/json

{"customerId":"customer-42","prompt":"Summarize this account"}
```

The response contains a public `runId`, the internal durable `taskId`, the LLM
draft, and `status: "awaiting_approval"`.

Read its durable business state:

```http
GET /runs/:runId
```

Send the human decision:

```http
POST /runs/:runId/approval
Content-Type: application/json

{"approved":true}
```

Poll `GET /runs/:runId` until the status is `completed` or `rejected`.
`saveCount` must remain `1`, including after another process restart. The LLM
and database writes are wrapped in named durable effects, so completed effects
are replayed from history instead of being invoked again.

## Maintainer acceptance (source checkout only)

The CLI integration suite runs the same path against a local fake provider. It
asserts process restart recovery, one LLM request, one final save, and a stable
completed result after a second restart:

```bash
cargo test -p tysel-cli --test dev_check \
  durable_agent_resumes_after_restart_without_repeating_effects
```
