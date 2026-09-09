# Changelog

User-visible changes are recorded here. Release notes reuse the matching version
section; detailed contracts and procedures remain in the linked documentation.
Use an exact version heading (including any prerelease suffix), and replace
`Unreleased` with the release date before tagging. Keep entries focused on user
impact rather than individual commits. PR checks allow `Unreleased`; tag releases
require a matching version and a valid date before artifact builds start.

## [0.3.0] - 2026-09-09

### Upgrade notes

- Durable stores migrate from log v1/v2 to v3 on open. Stop old writers and back
  up the complete store first; rollback requires the pre-upgrade snapshot and
  matching runtime. Legacy tasks without wakeup or execution records are not
  automatically recovered. See the [upgrade procedure](https://tysel.dev/docs/operations/production#durable-log-v3-upgrade).
- External writes still need provider idempotency or reconciliation. Applications
  own outbox delivery: admission retries require the original bundle, and completed
  tasks must be retained beyond the retry horizon because pruning removes deduplication
  records. See the [durable contract](https://tysel.dev/reference/runtime/durable).

### Added and fixed

- Recover active durable tasks after process death and preserve recovery across
  hot reload, with execution fencing and retries for temporary completion-storage failures.
- Add stable idempotency keys for durable admission and signals, plus retained
  results and cleanup through `tysel durable result` and `tysel durable prune`.
- Add Streams, `TextEncoder.encodeInto`, `TextEncoderStream`, body `bytes()` helpers,
  `Response.error()` and `Response.redirect()`; fix HTTP body byte preservation,
  URL path handling and malformed query decoding.
- Add guided project setup, automatic manifest-type synchronization during
  development, and structured diagnostics for manifest and import errors.
- Include queue time and body consumption in HTTP deadlines, and drain connections
  during shutdown.
- Add reviewed Chinese documentation and localized website navigation.
- Fix promise retention during stream backpressure.

### Performance

- Cache durable export metadata per source revision and wake the local scheduler
  after signal commits, while retaining periodic polling for recovery and other instances.

## [0.2.0] - 2026-09-04

- Added verified cross-target packaging for Linux and macOS on x64 and arm64,
  with authenticated runtime downloads and offline reuse of verified cached
  runtimes. Cross-target builds require a managed installation and trust policy.
- Published Linux toolchain images and hardened `tysel image` handling of
  existing binaries, release sidecars and generated container metadata.
- Added `tysel test --filter` and `--list`, including machine-readable discovery
  and exact test-ID selection.
- Added source-mapped development diagnostic events and improved installer
  feedback and the development server's displayed URL.
- Upgraded QuickJS-NG to 0.16.2.

[Release](https://github.com/wangcch/tysel/releases/tag/v0.2.0) ·
[Changes since 0.1.1](https://github.com/wangcch/tysel/compare/v0.1.1...v0.2.0)

## [0.1.1] - 2026-09-02

- Corrected TypeScript SDK publication to the scoped `@tysel/sdk` package.
- Made registry publication recoverable after partial release failures.

[Release](https://github.com/wangcch/tysel/releases/tag/v0.1.1) ·
[Changes since 0.1.0](https://github.com/wangcch/tysel/compare/v0.1.0...v0.1.1)

## [0.1.0] - Unpublished draft

The tag exists, but its GitHub Release remains a draft with no publication date.
This section records the tagged baseline, not a completed public release.

- Introduced native packaging of TypeScript applications as a single executable,
  with host-platform builds and a Web-API-first runtime.
- Provided HTTP services, cron and queue workers, MCP tools and explicit host
  capability configuration.
- Added durable steps, effects, sleeps, retries and signal waits with persisted
  boundary history. Active-execution crash recovery was not guaranteed; the
  recovery and admission fixes are recorded under 0.3.0.
- Included experimental Rust and Go Wasm Component tasks with bounded execution.
- Provided project setup, development, testing, compatibility checks and signed
  release verification tooling.

[Tagged source](https://github.com/wangcch/tysel/tree/v0.1.0)
