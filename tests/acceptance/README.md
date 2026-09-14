# Deployment and recovery acceptance

The runner reuses the real-project and fault-injection fixtures under
`tests/workflows` and `tests/p1`. It exits unsuccessfully on the first failed
assertion, fixture error, timeout, cleanup error, or changed tool binary. It
does not download dependencies or connect to a real LLM provider.

## Suites

| Suite | Coverage | Admission requirements |
| --- | --- | --- |
| `smoke` | Three example workflows; HTTP deadlines and shutdown in CLI/standalone mode; completion/storage/reload/error isolation regressions; worker/finalization regressions; three SQLite crash windows in both modes | Matching local tools and workspace TypeScript dependencies |
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
