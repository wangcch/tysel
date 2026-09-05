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

```bash
cargo test -p tysel-cli --test examples isolated_plugin_enforces_profile_and_recovers
```
