# Deployment and recovery acceptance

The runner reuses the real-project and fault-injection fixtures under
`tests/workflows`, `tests/p1` and the local agent-triage contract fixture. It exits
unsuccessfully on the first failed assertion, fixture error, timeout, cleanup
error, or changed tool binary. It does not download dependencies or connect to
a real LLM provider.

## Suites

| Suite | Coverage | Admission requirements |
| --- | --- | --- |
| `smoke` | Three example workflows; agent-triage contracts, P2 recovery, P3 attacks and P4 deployment; HTTP deadlines and shutdown in CLI/standalone mode; completion/storage/reload/error isolation regressions; worker/finalization regressions; three SQLite crash windows in both modes | Matching local tools and workspace TypeScript dependencies |
| `full` | Smoke plus all six SQLite crash windows and all six PostgreSQL crash windows with one and two instances, each in CLI/standalone mode | Matching tools and a disposable PostgreSQL database with `psql` |
| `release` | Same complete matrix as `full` | Release-profile tools, matching embedded source commit, clean checkout, no partial selection, and PostgreSQL |

The six crash windows are unacknowledged admission, before external write,
external commit before response, response before effect recording, after effect
recording, and projection update before signal-wait registration. The provider
reconciles by a stable operation ID. This tests recovery and deduplication; it
does not promise exactly-once writes to arbitrary providers.

The crash fixtures allow five seconds for a request and explicitly verify that
the execution is still running immediately before SIGKILL. This leaves time to
start the peer without accidentally replacing the crash scenario with a request
timeout. The dedicated HTTP fixture separately checks the shorter deadline.

## Local use

Build the current tools and workspace declarations first:

```sh
cargo build --locked -p tysel-cli -p tysel-runtime -p tysel-isolate --bins
pnpm --filter @tysel/types build
pnpm --filter @tysel/sdk build
python3 tests/acceptance/test_runner.py
python3 tests/acceptance/run.py --suite smoke \
  --bin-dir target/debug --profile debug --target linux-x64 \
  --output target/acceptance-local
```

Choose the actual host target (`linux-x64`, `linux-arm64`, `darwin-x64`, or
`darwin-arm64`). Each output directory must be new or empty. macOS passes are
development evidence; Linux isolation acceptance needs a Linux run.

To run the complete matrix, create a dedicated disposable PostgreSQL database
and set `TYSEL_GATE_POSTGRES_URL` through the test environment, then use
`--suite full`. The database user needs permission to create/drop schemas. Each
crash scenario creates its own random schema and drops it after success or an
assertion failure. A forcibly killed harness can leave a schema behind; destroy
the disposable test database after such a run, as CI does with its service.
Never point the fixture at application data.

The client uses `psql` by default. `TYSEL_GATE_PSQL` may name a local executable
wrapper, for example when the client is in a container. Credentials are passed
through libpq environment variables rather than command arguments. Fixture URLs
accept PostgreSQL URI credentials, host, port, database, and the `sslmode`,
`sslrootcert`, `sslcert`, `sslkey`, and `channel_binding` query options. The
fixture owns the PostgreSQL `options`/search-path setting.

For focused diagnosis, select a case:

```sh
python3 tests/acceptance/run.py --suite full --case crash-postgres-2 \
  --bin-dir target/debug --profile debug --target linux-x64 \
  --output target/acceptance-postgres-peer
```

`--case` is repeatable and recorded as a partial selection. It is forbidden for
`--suite release`. The runner removes inherited fixture flags and Python
optimization settings so the environment cannot silently omit modes or disable
fixture assertions.

## Evidence and cleanup

The `agent-triage` case runs `agent_triage.py` against the real trusted service,
isolated plugin and an independent local HTTP fixture. Its JSON report covers
C01–C06a: concurrent admission, customer isolation, bounded operations/messages,
timeout/overload, caller restart and loss of a client response after admission.
Redirect rejection causes one physical request; transport retries are separately
reserved and bounded at two attempts per logical operation.
P2 changes new-job restart behavior to bounded recovery; legacy P1 rows retain
interruption semantics. The separate `agent-triage-recovery` case covers C07–C09
with lease-aware crash barriers, changing reads, application ownership,
revocation, version pinning, storage failure and completion reconciliation.
The separate `agent-triage-adversarial` case expands C09 and covers C10 with
malicious isolated plugins, serial/concurrent cross-job and prior-attempt replies,
two-customer concurrent crash recovery, revocation after backoff, streamed byte
and Unicode text boundaries, permission probes and audit transaction failures.
Canary checks inspect plugin headers/bodies, business state, Durable history,
completion, audit metadata and process diagnostics. These three cases use fake
providers; local macOS passes do not establish P4 Linux/artifact delivery.
`agent-triage-deployment` covers C11–C12 with exact packaged caller/plugin/worker
bytes, a portable supervisor, clean-directory startup, the actual 45-second
lease, pinned artifact/state identities, stopped terminal-only backup/restore,
configuration failures and resource exhaustion. On Linux it additionally blocks
required Landlock/seccomp/rlimit setup and requires startup refusal. macOS records
those probes as not run. `--retain-release` on the focused deployment fixture
optionally saves successfully tested binaries; CI's default keeps only metadata
and diagnostics. See the [deployment guide](../../examples/agent-triage/DEPLOYMENT.md).
The packaged backup/restore group also rejects symlinks and special files,
requires failed copy destinations to disappear, and retries at the same paths
after removing the invalid entries. It preserves content, original artifact
binding and POSIX-mode checks. Because `deploy.py` participates in the package
binding, its cleanup update requires a new release and namespace; existing
namespaces and snapshots must retain their complete original package.
`agent-triage-initialization` preserves the initialization contracts added in
P5.1, including after P5.2 withdrew the experimental schema cache: a failed first request can
retry in the same process/isolate, concurrent first requests migrate legacy rows
without duplicate admission, and warmed handlers still enforce expiry, current
ownership, delivery horizons and changed credentials. A 648-row state matrix
checks deadline/horizon equality, expiry-before-recovery precedence, retained
terminal data and exactly one terminal audit row per transition. It uses the same real
runtime and disposable namespace fixture and runs in smoke/full/release suites.
See [the example](../../examples/agent-triage/README.md) for topology and commands.

`evidence.json` records the source commit, dirty state, tracked diff hash,
untracked file hashes, declared build profile, host/target, all three tools'
SHA-256 and embedded build information, selected cases, status, and timestamps.
Debug build metadata may have a null source commit; that is kept visible.
`--require-build-commit` rejects absent or stale embedded commits. The profile
field records the build command's profile; the existing binary metadata does not
independently report optimization settings.

Each case retains `case.json`, `runner.log`, and copies of fixture logs/results.
The workflow case also writes `workflow-report.json`. Databases, application
binaries, node_modules, and symlinked logs are excluded from diagnostic uploads.
Temporary fixture paths mentioned in logs are historical; their retained log
copies are under the case's `fixtures/` directory.

Cases have a bounded timeout and run in a separate process session. On exit or
timeout, the runner terminates and then kills that session's remaining child
processes before collecting logs. It never sends signals to unrelated services.

## CI and release wiring

- `ci.yml` validates harness failure handling in `changes`. The existing required
  `rust` job runs `smoke` on Linux x64 for full changes, with source-identified
  debug tools; `gate` already requires that job to succeed.
- The existing `rust-arm64` isolation job runs the packaged Agent deployment
  case with matching source-identified tools, including required setup failures,
  and retains its diagnostics. The native Landlock/seccomp/worker tests remain
  in the existing isolation crate gate.
- `release-linux.yml` runs `release` against `target/repro-1/release`, the exact
  tools copied into the independently reproduced archive. Both Linux targets use
  this workflow. Acceptance runs before unsigned release artifacts are uploaded;
  a failure prevents downstream signing/publication through existing job needs.
- Diagnostic artifacts are uploaded on success or failure: CI retains them for
  30 days; Linux release retains them for 90 days. These reports supplement the
  existing signing/reproducibility evidence; they are not signatures themselves.

Harness tests inject assertion failure, a hung process with a descendant that
ignores SIGTERM, cleanup failure, stale/mixed binaries, invalid release admission,
and ambient flags that could weaken coverage. They also verify PostgreSQL client
credential handling and schema cleanup. They require only Python 3.9+ and normal
process inspection permissions, not a runtime build or network service.

The separate snapshot administration tests are also offline:

```sh
python3 tests/acceptance/test_snapshot_cleanup.py
python3 -O tests/acceptance/test_snapshot_cleanup.py
```

`test_snapshot_cleanup.py` has eight groups covering 23 branches. It uses real
temporary SQLite namespaces, copy/cleanup functions and artifact-binding
comparisons. Only executable verification at restore admission is mocked;
copy, manifest-write and cleanup failures are injected explicitly. It starts no
runtime, container or external service and does not replace the real-artifact
`agent-triage-deployment` case.

The tests require caught copy/manifest failures to remove only the destination
exclusively created by that operation, including copied read-only directories,
so the same path can be retried after fixing the cause. Source contents and
modes, unrelated siblings, pre-existing destinations, competing creators and
replaced destination roots remain untouched. Created parent directories are
not reclaimed. If cleanup fails, the residual path and original operation
error must remain visible for inspection before further cleanup or retry.
This is not a claim of atomicity under SIGKILL, power loss or arbitrary
filesystem changes by another process with the same user identity.

The O4 supervisor tests are separate offline checks using real threads and
Futures with fake HTTP and child processes:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tests/acceptance/test_dispatcher.py
PYTHONDONTWRITEBYTECODE=1 python3 -O tests/acceptance/test_dispatcher.py
```

`test_dispatcher.py` has 11 groups / 42 branches across the packaged and demo
supervisors. It covers completed-future wakeup and the short follow-up window,
eight-slot and per-job bounds, unresolved-job retry floors, slow-work/503 fallback,
HTTP/future errors, child liveness, fresh wake events after restart, and disabled
dispatch. Both normal and optimized Python pass on macOS and Linux. These tests
start no sockets, runtime or container; they do not establish throughput or
Durable/application recovery correctness.

The [paired dispatcher tool](../../benchmarks/agent-triage/README.md) also has ten
offline test groups, run with `python3 benchmarks/agent-triage/test_dispatch_ab.py`
and `python3 -O benchmarks/agent-triage/test_dispatch_ab.py`. They cover the frozen
pair schedule, inclusive selection thresholds, complete evidence, exact package
selection, safe HTTP observation and Python/Rust floating-point round-tripping.
These two suites are wired into `ci.yml`'s `changes` job; remote CI has not run.

O4 responds to the user's concurrent-throughput goal. AB2's 24 pairs / 48 rounds
have passed their gates and independent recomputation on the local Podman volume;
AB1's final float-validator failure is retained and was followed by a complete
rerun. The unobserved original matrix now passes 132/132 cost checks and 115
process cleanups, all 34 related real-artifact regression groups pass, and the
exact measured package passes two-container and off-volume backup/restore tests.
Recovery takes 45,572.73 ms within the 50,000 ms budget; healthy/crash authorized
reads remain 1/2 and old-key replay adds no reads. Both supervisor changes are
retained for the local Linux ARM64 Podman target; paired c4 controls per job rose
68.1–85.2%, and this does not establish a production SLO or power-loss durability.
The paired diagnostic keeps `retainCandidate=false` because it only proves
eligibility; the final
[retention gate](../../docs/agent-execution-evidence/README.md#dispatcher-throughput)
records `accepted` / `retainCandidate=true` after all gates pass. See
[O4 evidence](../../docs/agent-execution-evidence/README.md#dispatcher-throughput).
