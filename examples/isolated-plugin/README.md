# Isolated Plugin

This example runs a Fetch handler in Tysel's `isolated` profile. The manifest
deliberately declares one fetch host and one filesystem root. The profile still
denies both host-facing capabilities because isolated workers receive no ambient
network or filesystem authority.

## Run

Build the CLI and worker, then start the example:

```bash
cargo build -p tysel-cli -p tysel-isolate --bin tysel --bin tysel-worker
TYSEL_WORKER=target/debug/tysel-worker \
  target/debug/tysel run --manifest examples/isolated-plugin/tysel.toml
```

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

## Automated acceptance

```bash
cargo test -p tysel-cli --test examples isolated_plugin_enforces_profile_and_recovers
```
