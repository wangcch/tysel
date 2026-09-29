# Isolated Plugin

This example runs a Fetch handler in Tysel's `isolated` profile. The manifest
deliberately declares one fetch host and one filesystem root. The profile still
denies both host-facing capabilities because isolated workers receive no ambient
network or filesystem authority.

## Run

Install Tysel, then run these commands from the example directory:

```bash
tysel doctor --install
tysel config validate
tysel run
```

The managed installation includes the matching `tysel-worker`; no separate
worker build or environment variable is required.

The command prints the selected address as `tysel listen HOST:PORT`. Use it in
the following requests:

```bash
curl http://HOST:PORT/
curl -i http://HOST:PORT/probe/fetch
curl -i http://HOST:PORT/probe/filesystem
```

The root route returns the plugin identity. Both probes return HTTP 403 and a
JSON document with `denied: true`. This is expected even though the resources
appear under `[permissions]`: the effective authority is the intersection of
the manifest, deployment policy, execution profile, and runtime support.

## Customer snapshot experiment

A trusted caller can read a customer record using its own permissions, select
only the fields needed by the plugin, and pass that snapshot as request data:

```bash
curl --fail-with-body http://HOST:PORT/summarize \
  -H 'content-type: application/json' \
  -d '{"customerId":"customer-42","name":"Acme","openTickets":3}'
```

```json
{"customerId":"customer-42","summary":"Acme has 3 open support tickets.","needsAttention":true}
```

This is a deterministic formatter, not an LLM-generated summary. It needs no
network access, filesystem access, credential, or new runtime capability.
`customerId` alone returns 400: the caller must supply the snapshot. Invalid
JSON, unknown fields, and invalid field values also return 400. GET returns 405.
Customer IDs and names are limited to 128 and 80 JavaScript string code units;
`openTickets` is an integer from 0 to 10000.

The caller must omit credentials and unrelated fields **before sending** the
request. Validation inside a plugin is not a confidentiality boundary: hostile
plugin code could read any data it receives. Likewise, a caller should validate
plugin output before using it for a privileged action. This local example has
no caller authentication or real customer-system integration.

### What this experiment establishes

The acceptance test holds a synthetic customer record in the trusted test
process, projects the three required fields, and sends the snapshot over HTTP to
the isolated worker. It verifies the summary, rejects incomplete or invalid
input, checks profile-specific network and filesystem denials, kills the worker,
and checks the same summary and continued network denial after replacement.
It does not call a real CRM or paid model provider.

For a bounded transformation whose input is known in advance, passing data
through the existing request boundary is sufficient. This experiment does not
justify a new host-operation broker API. The next experiment below explores a
lookup selected during computation. This macOS run checks application behavior;
production isolation still requires the Linux security gate below.

## Dynamic lookup experiment

`POST /triage` chooses the highest-priority ticket (0–3, larger is more urgent)
from at most 16 supplied ticket descriptors. It asks the caller for that ticket's
details using an application-level JSON response:

```json
{"customerId":"customer-42","tickets":[{"id":"ticket-low","priority":0},{"id":"ticket-urgent","priority":3}]}
```

```json
{"kind":"lookup","operation":"ticket.read","ticketId":"ticket-urgent"}
```

The trusted caller treats this response as untrusted data. It checks the exact
operation shape, permits only `ticket.read`, restricts IDs to its independently
authorized customer scope, and allows one data read. It never accepts a URL,
credential, customer scope, or permission grant from the plugin. After its data
adapter returns, the caller sends the original snapshot plus the selected fields:

```json
{"customerId":"customer-42","tickets":[{"id":"ticket-low","priority":0},{"id":"ticket-urgent","priority":3}],"detail":{"ticketId":"ticket-urgent","subject":"Checkout unavailable"}}
```

```json
{"kind":"done","customerId":"customer-42","summary":"Prioritize ticket-urgent: Checkout unavailable"}
```

The caller then accepts only a final result; it does not keep executing requests
in an unbounded loop. The plugin is stateless between these two HTTP requests.
The caller retains the snapshot and checks output before any subsequent action.

### Evidence and limits

The original trusted caller and its synthetic ticket adapter live in the Rust
acceptance test (`TicketCaller` in `crates/tysel-cli/tests/examples.rs`). The
[agent-triage example](../agent-triage/README.md) now provides a runnable caller
and independently counted HTTP fixture using the versioned `/triage/v1` route.
The test rejects unknown operations, out-of-scope IDs, extra URL fields, and a
second lookup before data access. It kills the worker between lookup and result,
then verifies the completed triage and continued denial of direct network access.

```bash
cargo test -p tysel-cli --test examples isolated_
```

This bounded two-request computation works with existing HTTP and isolation
primitives. No runtime API or worker authority is added. It is not a real CRM
integration, an LLM agent, or a production broker. It moves scope checks, budget,
result validation, and continuation state into the calling application. The
test caller's state is in memory; its own crash recovery is not demonstrated.
HTTP round-trip cost and larger branching workflows have not been benchmarked.

For this workload, keep the protocol local to the application. A reusable runtime
abstraction needs further evidence of repeated caller logic, unacceptable
round-trip cost, or a requirement to suspend and resume inside a single plugin
invocation. These experiments have not established those requirements.

## Run a packaged application

`tysel build` creates the application executable. Deploy a matching toolchain's
`tysel-worker` alongside it:

```text
dist/
  isolated-plugin
  tysel-worker
```

Alternatively, set `TYSEL_WORKER` to the matching worker's path. The build
command reports this dependency but does not copy the worker automatically.
The application does not need TypeScript source, the manifest, or `node_modules`
at runtime, but the isolated profile does need this separate worker executable.

## Crash recovery

On Unix, kill the worker child while leaving the `tysel` supervisor alive:

```bash
TYSEL_PID=THE_TYSEL_PROCESS_ID
WORKER_PID=$(ps -axo pid=,ppid=,comm= | awk -v parent="$TYSEL_PID" \
  '$2 == parent && $3 ~ /tysel-worker/ { print $1; exit }')
kill -KILL "$WORKER_PID"
curl http://HOST:PORT/
```

The next request succeeds after the supervisor replaces the worker and reloads
the embedded handler. Linux additionally applies the documented Landlock,
seccomp, rlimit, and best-effort cgroup controls; macOS is a development check,
not the production sandbox gate.

## Maintainer acceptance (source checkout only)

The [agent-triage example](../agent-triage/README.md) supplies a runnable trusted
caller and local HTTP data fixture. It uses `POST /triage/v1`, which wraps this
example's triage payload in a versioned job/step/attempt envelope. The existing
`/triage` endpoint remains available for the original experiment.

```bash
cargo test -p tysel-cli --test examples isolated_plugin_enforces_profile_and_recovers
```
