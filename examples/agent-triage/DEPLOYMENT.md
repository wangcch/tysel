# Packaged triage deployment

This is a single-caller deployment exercise using the existing Tysel build,
SQLite, Durable and isolated-worker APIs. It retains the example's fixed fake
identities and loopback adapter. It is not production authentication, a general
broker, a multi-replica deployment or a remote provider integration.

## Build and retain one release

Build matching tools for the actual host, then package the example:

```sh
cargo build --locked -p tysel-cli -p tysel-runtime -p tysel-isolate --bins
python3 examples/agent-triage/package.py \
  --bin-dir target/debug --output target/triage-release
```

Use a new or empty output directory. A Linux release must be built with Linux
tools; a macOS executable is not a Linux artifact. The normal workspace
TypeScript checks should run before packaging. The packager copies the entry
sources into disposable build directories, so its embedded build output reports
that a per-copy TypeScript check was skipped. It does not download dependencies.

The release contains exactly five files:

| File | Role |
| --- | --- |
| `caller` | Native executable containing the trusted service and Durable program |
| `plugin` | Native executable containing the isolated plugin supervisor and bundle |
| `tysel-worker` | Exact matching isolated execution worker |
| `deploy.py` | Python 3.9+ supervisor and offline state administration |
| `release.json` | Toolchain identity, source hashes and hashes/sizes of the four files |

There are three native artifacts, not one. Runtime needs Python and the host
libraries required by the native tools, but no checkout, `.ts` files, manifest,
Node.js or `node_modules`. Normal operation has the Python supervisor, two native
service processes and one isolated worker; the data source is separate.
Packaging preserves the matching worker's exact bytes and executable POSIX mode.
It does not require copying source security extended attributes onto the
destination filesystem.

Keep the exact release bytes. Embedded source maps can make a rebuild at another
path produce a different executable even for the same source. The namespace
binds executable hashes, worker and supervisor, not a mutable version label.
These checks detect missing/mixed/modified local files; `release.json` is not a
signature or an independent trust root. This example packaging command does not
claim a signed/reproducible official release.

## Configure and run from a clean directory

Provide a local read-only ticket adapter. It must serve `GET /tickets/ID`, verify
the bearer credential and `X-Customer-Id`, and return `{ticketId, subject}` plus
optional fields that the caller projects away. No adapter or fake database is
embedded in the package. The acceptance fixture supplies an independently
counted adapter; `run.py` remains the one-command disposable source demo.

Copy [the settings example](deployment-config.example.json), set the real local
fixture port and its stable adapter identity, and initialize an empty namespace:

```sh
python3 target/triage-release/deploy.py init \
  --state /absolute/path/triage-state --config /absolute/path/settings.json
python3 target/triage-release/deploy.py verify
python3 target/triage-release/deploy.py serve --state /absolute/path/triage-state
```

Supply `TRIAGE_FIXTURE_TOKEN` through the process environment before `serve`.
It is never written into the release or namespace. The token should match the
local adapter; this example has no secret-manager integration. The sample
`demo-a`/`demo-b` client identities are public development credentials.

`serve` prints the caller URL, listens on ephemeral loopback ports, starts a
bounded eight-thread dispatcher and stops its own children on SIGINT/SIGTERM.
The supervisor's HTTP control requests bypass ambient proxies; child processes
also bypass proxies for loopback. No listening port is exposed externally.
The caller retains four active jobs per customer, eight total and 1,000 retained
jobs, a 120-second deadline and at most two physical attempts per operation.

The state directory is independent of the release:

```text
triage-state/
  binding.json                 original artifact hashes and namespace identity
  namespace.lock               one supervisor/administration owner
  caller/config/service.json   identities, current grants, boot and dispatch IDs
  caller/data/jobs.db          application admission, budget, outcomes and audit
  caller/data/durable-events.db Durable programs, effects and completion
  plugin/                      worker supervisor working directory
```

SQLite may also create WAL/SHM files. Preserve them with the databases. The state
root is private to its owner; configuration files are mode 0600. Start only one
supervisor per namespace. The advisory lock coordinates this script's operators,
not arbitrary manual processes that bypass it. Do not launch extra callers or
independent schedulers against these stores.

## Podman storage target

P5.2 uses Podman as the selected deployment target. In this local setup, named
volumes live on the Podman Linux VM filesystem and retain their contents across
container replacement. A host directory bind mount uses a different storage
path; report its measurements separately. A named-volume result does not make
the earlier bind-mount performance gate pass.

Mount the **whole namespace** on one persistent named volume, including both
SQLite databases, configuration and binding. Keep the original release available
for restart. The split-store symlinks and external Durable path overrides used
in storage diagnostics are not a supported deployment layout.

For a portable backup, drain and stop the original namespace, export its stopped
snapshot outside the named volume, then let the packaged restore helper verify
its hashes and original release binding before copying into a new directory.
Fence every old producer before starting the restored namespace, as described
below. Container restart and copy verification do not establish host power-loss
durability.

## Restart, version binding and retention

Restart the same release with the same namespace. A new boot ID and plugin port
are written without changing the saved artifact identity, original deadline,
admission key or attempt budget. A killed in-flight read consumes its original
slot. Recovery waits for the actual default 45-second Durable lease and can use
only the remaining slot. No lease timestamp is edited or accelerated.

A changed caller, plugin, worker or supervisor is rejected for an existing
namespace before work starts. Initialize a new namespace for new artifacts;
retain the original release to drain old work and resolve old keys. A source
update does not patch the complete programs already saved by Durable. This is a
strict local pinning convention, not rolling deployment or automatic migration.
The snapshot cleanup update also changes `deploy.py` and therefore the package
binding. Existing namespaces and their snapshots must keep using their complete
original package; replacing only its administration script is not an upgrade
path. Package the updated helper as a new release for a new namespace.

There is no pruning or reset command. At the 1,000-row retention cap, new keys
are rejected while existing keys continue to resolve. `init` never overwrites
existing state. Moving to a new empty namespace deliberately ends deduplication
against an old namespace; do not use it as an automatic retry strategy.

## Stopped backup and restore

This helper supports **terminal, drained namespaces only**. Stop the supervisor
and ensure no unmanaged process writes either database. Backup rejects an active
lock, unfinished business jobs or pending outbox delivery. An interrupted active
namespace must first recover/drain under its original release. The helper copies
both stores, WAL/SHM files, configuration and artifact binding as one stopped
snapshot, then hashes every copied file.

```sh
python3 target/triage-release/deploy.py backup \
  --state /absolute/path/triage-state --output /absolute/path/triage-backup
python3 target/triage-release/deploy.py restore \
  --backup /absolute/path/triage-backup --state /absolute/path/restored-triage-state
python3 target/triage-release/deploy.py serve --state /absolute/path/restored-triage-state
```

Backup and restore destinations must be new. Restore requires original artifact
hashes and intact snapshot hashes; it never overwrites a current namespace.
The restore destination must also be outside the snapshot directory.
`copy_namespace` copies file bytes and POSIX modes for files and directories;
it rejects symlinks and special files. It does not copy source SELinux labels,
ACLs, ownership or timestamps. The destination filesystem supplies its security
labels, allowing ordinary namespace contents to move between a Podman volume
and a host bind mount without importing the source mount's labels. Snapshot
hashes verify file contents, not those omitted metadata fields.

If copying or writing the snapshot manifest raises a caught exception, the
helper removes the destination it exclusively created for that operation. This
includes copied directories whose preserved modes are read-only. It leaves the
source, pre-existing destinations and paths created by a competing `mkdir`
untouched. Before cleanup it checks the destination root's device, inode and
type; a replaced root is retained rather than removed. Parent directories
created along the way are not reclaimed. After fixing the cause of a failure
whose cleanup succeeded, retry with the same destination path.

If cleanup itself fails, the error identifies the residual destination and
preserves the original operation error. Inspect that path and the reported
cause before handling the residue and retrying; do not assume the partial copy
is a usable snapshot. These guarantees apply to caught exceptions. They do not
provide atomicity under SIGKILL, power loss or arbitrary filesystem changes by
another process with the same user identity.

The acceptance case compares retained jobs, admission identity, attempt budget,
audit rows and Durable completion before and after restore, then proves that
resubmission returns the same result without another read. It also uses the
packaged helper to reject unsupported files, check that failed destinations are
absent, and retry backup and restore at the same paths with content and POSIX
mode checks.

**Fence every old producer before restoring.** Do not run source and restored
copies simultaneously. A snapshot cannot prove that no later read or admission
occurred in another copy; it does not reconcile those later events. Restore from
a known latest stopped snapshot, or reconcile subsequent activity separately.
The helper does not provide distributed fencing, live backup or rollback-safe
restoration of active work. Keep backups private: they contain projected
business data, development client identities and an internal dispatch token.

## Linux isolation and failures

Linux requires Landlock, seccomp and worker resource-limit setup to succeed.
P4 probes add a restrictive outer filter or lower a hard limit to make each setup
fail; the exact packaged plugin then exits before listening. They never disable
the container's filters or ask the runtime to fall back to weaker isolation.
The existing native tests separately exercise actual Landlock file denial,
seccomp syscall denial, over-allocation death and worker recovery.

The worker has `NoNewPrivs=1`, seccomp filter mode and a 64-file limit. CPU and
heap exhaustion in a packaged test plugin fail the request and allow a healthy
replacement/request afterward. cgroup memory control remains **best effort**;
this gate does not turn it into a mandatory or aggregate process-RSS promise.
macOS skips Linux-specific sandbox probes and is development evidence only.

| Failure | Required behavior |
| --- | --- |
| Missing worker | Explicit complete-release/matching-worker error; direct plugin also fails |
| Wrong or modified worker | Hash/build-identity mismatch before deployment |
| Different release with old state | Namespace binding error; original tasks/results untouched |
| Missing caller filesystem grant | Startup health check fails with configuration/permission guidance |
| Invalid adapter origin | Initialization rejects origins outside the loopback example |
| Required Linux sandbox setup failure | Worker and plugin refuse startup; no listening service |
| Modified backup or existing restore destination | Restore fails without overwriting state |
| Caught snapshot copy or manifest failure | Remove only this operation's destination; allow same-path retry after fixing the cause |
| Snapshot cleanup failure or replaced destination root | Retain the path and report it with the original operation error for inspection |

## Acceptance and CI

```sh
python3 tests/acceptance/run.py --suite smoke --case agent-triage-deployment \
  --bin-dir target/debug --profile debug --target linux-arm64 \
  --output target/agent-deployment-acceptance
```

Select the actual target. This case is included in smoke/full/release suites;
it packages using the exact tools selected by the parent runner. The existing
Linux x64 smoke and both Linux release paths therefore run it. The existing
ARM64 isolation job also builds matching tools and runs this case explicitly.
CI retains metadata and diagnostics, not temporary databases or native packages.

The separate offline function tests need only Python 3.9+:

```sh
python3 tests/acceptance/test_snapshot_cleanup.py
python3 -O tests/acceptance/test_snapshot_cleanup.py
```

Their eight groups cover 23 branches using temporary SQLite namespaces, real
copy/cleanup functions and artifact-binding comparisons. Restore's executable
verification is mocked; copy, manifest and cleanup errors are injected
explicitly. No runtime, container or external service is started. These tests
cover failure cleanup and retry behavior, while the existing deployment case
above checks the actual packaged artifacts. Neither establishes recovery from
SIGKILL or host power loss during snapshot creation.

To retain the actual tested bytes locally, use the focused fixture:

```sh
python3 tests/acceptance/agent_triage_deployment.py \
  --bin-dir target/debug --output target/agent-deployment.json \
  --retain-release target/tested-triage-release
```

It retains the release only after all applicable checks pass; state/fixture
namespaces are removed. See the [P4 evidence record](../../docs/agent-execution-evidence/README.md#validation)
for the observed host, artifacts, commands and limits of the local validation.
See the [P5.2 Podman evidence record](../../docs/agent-execution-evidence/README.md#storage-target)
for the separate storage comparisons, cross-filesystem copy and container
lifecycle observations and their validation scope.
