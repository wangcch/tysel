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
Send `{"approved":false}` to reject the draft. Both decisions save a final
decision record; `saveCount` counts that record, not an external business action.
Invalid approval JSON or a non-boolean decision returns HTTP 400. A decision
submitted after the run is completed or rejected returns HTTP 409.

`saveCount` must remain `1`, including after another process restart. The LLM
and database writes are wrapped in named durable effects, so completed effects
are replayed from history instead of being invoked again.
If the final business write commits before its effect is recorded, recovery
executes that effect again. The final SQL update uses the unique `runId` and
`result_json IS NULL` in one statement, preserving the first saved decision
without incrementing `saveCount` again.

This is a local, trusted-service example with no authentication. Its draft is
immutable and its approval URL identifies the run containing that draft. Before
exposing it to other users, the application needs authentication and authorization
for reading runs and submitting decisions. The terminal-state check does not
provide atomic arbitration between concurrent decisions.

The shell demonstration checks replay of already recorded effects. The
integration tests additionally inject a crash between the final business commit
and durable-history persistence. This verifies idempotency of this SQLite
update, not exactly-once external writes. Other writes and provider calls need
their own application or provider idempotency.

## Maintainer acceptance (source checkout only)

The CLI integration suite runs approval and rejection paths against a local fake
provider, without real credentials or paid calls. It kills the service while
waiting, starts a new process over the same stores, and checks:

- the persisted draft is unchanged and no decision was saved before approval;
- invalid decisions return 400 and leave the run waiting;
- approval and rejection each produce one final decision record;
- a second restart preserves the result without another LLM request;
- further decisions return 409 and leave the terminal result unchanged.

Two additional tests kill the service after the final business write but before
`save-result` is recorded, covering both approval and rejection. They verify
the history gap before restart, then wait for durable completion and compare
the business record, effect payload, and completion result. Recovery must leave
`saveCount=1` and must not call the LLM again. Each test uses separate stores so
the cases can run concurrently.

```bash
cargo test -p tysel-cli --test dev_check \
  durable_agent_
```

## Experiment 1: recovery and decision boundaries

The experiment reproduced two application-boundary failures: JSON `null` at the
approval endpoint returned 500, and a decision sent to an already terminal run
also returned 500. Input validation and a terminal-state check fix these in the
example without changing runtime APIs.

This supports continuing with the existing service and durable primitives for
this workflow. It does not test generated code in the isolated profile or prove
a need for brokered host operations.

## Experiment 2: business commit before effect history

The test instruments only a temporary copy of the example: after the final SQL
write, a persisted test marker and timer hold the effect open. The test observes
the marker, kills the process, and confirms that the business record exists but
the `save-result` history event does not. On restart, the same stored program
skips the hold and retries the effect after the previous execution lease expires.
No fault-injection endpoint or switch is added to the shipped example.

Before the fix, this reproduced `saveCount=2`. Guarding the final update with
`result_json IS NULL` makes retry a no-op at the business database while allowing
the runtime to record the effect and complete the task. The saved result and
timestamp remain unchanged. This is an application-level fix using existing
SQLite and durable APIs.

The business status may become `completed` or `rejected` before durable task
completion is recorded; those are separate observations. This experiment does
not cover the LLM response/history window, draft-write recovery, or concurrent
approval arbitration. Existing in-flight tasks retain their stored program;
deploying this source change does not rewrite their non-idempotent SQL.
