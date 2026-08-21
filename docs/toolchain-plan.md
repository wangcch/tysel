# Developer toolchain iteration plan

Status: proposed implementation contract.

This plan covers four connected deliverables:

- the `install.sh` bootstrap installer;
- `tysel doctor`;
- `tysel upgrade`;
- a complete and publishable `@tysel/types` package.

The user-facing installation instructions remain in [Install](install.md). This
document defines the target experience together with the architecture,
sequencing, safety properties, and release gates required to deliver it.
Commands described as future milestones are not available yet.

## Product outcome

A new user should be able to reach a passing Tysel project from a clean machine
without Rust, Node, npm, or administrator access:

```sh
curl -fsSL https://tysel.dev/install.sh | sh
tysel init hello-tysel
cd hello-tysel
tysel check
tysel test
```

Targets:

- less than one minute from starting the installer to `tysel --version` on a
  normal broadband connection;
- less than two minutes to the first passing `tysel test`;
- no partial or mixed-version installation after any failed install or upgrade;
- useful editor and type-check feedback after installing the optional npm
  development dependencies;
- one diagnostic command that identifies installation, platform, project, and
  optional network problems without running application code.

Tysel has two separate distribution products:

| Product | Contents | Contract |
| --- | --- | --- |
| Developer toolchain | `tysel`, `tysel-service`, and `tysel-worker` from one release | Installed, diagnosed, and upgraded as one unit |
| Production application | One executable emitted by `tysel build` | Runs without the developer toolchain, Node, npm, or `node_modules` |

`cargo install` and `npm install --global` remain unsupported native-toolchain
distribution paths because they cannot preserve the three-binary contract.

## Repository assessment

### Distribution

The repository already has a strong Linux production-release foundation:
reproducible `linux-x64` and `linux-arm64` archives, SHA-256 sidecars, artifact
signatures, release evidence, and centralized GitHub Release publication.

The following gaps block a safe public installer:

- release URL, stable-channel, version naming, minimum OS/libc, and release
  manifest contracts are not finalized;
- Darwin x64 and arm64 archives are not published;
- release scripts assume GNU `tar`, `sha256sum`, Linux linker flags, Linux
  service containers, and a hard-coded two-target signing loop;
- `tysel-service` and `tysel-worker` have no side-effect-free `--version` or
  build-information mode;
- first-install trust is unresolved: the existing verifier assumes an already
  trusted Tysel binary;
- the current archive extraction and temporary PATH workflow is evaluation
  guidance, not a persistent or atomic installation model.

### CLI lifecycle

There is no `doctor` or `upgrade` command. Target detection is currently private
to CLI build code, while release signing separately validates a hard-coded target
set. Stub and worker discovery prefer siblings, which is the right runtime
behavior but makes atomic three-binary activation essential.

The CLI already provides useful building blocks: manifest parsing, bundling,
compatibility scanning, structured fatal errors, release signature verification,
redaction, and platform security probes. Doctor and upgrade should reuse those
libraries instead of shelling out to normal project commands.

### TypeScript contract

`@tysel/types` currently contains eight lines: two profile aliases and one
capability interface. It is private and has no publication workflow.

The actual public surface is spread across:

- `packages/tysel`, which defines application, task, and durable helper types;
- `packages/tysel-test`, which defines test globals;
- `runtime-js/durable`, which duplicates part of the durable contract;
- the QuickJS host bindings, which install the global `tysel` object and
  capability APIs;
- example-local `tysel.d.ts` files for SQLite and Postgres;
- prose in the runtime API documentation.

This makes drift likely. Generated projects reference `@tysel/test`, but package
publication, editor loading, and compatibility with the native runtime are not
release-gated.

### Documentation

The repository currently mixes contributor requirements with application-user
requirements. Rust, Node, pnpm, and TypeScript are required to build the
repository, but only optional TypeScript/editor tooling should be presented to a
binary-install user. README, install, getting-started, generated-project, and
package-publication changes must land together with the relevant milestone.

## Non-negotiable invariants

1. **One release unit.** CLI, service, and worker share semantic version, source
   commit, canonical target, and release identity.
2. **Atomic activation.** Verification completes before activation. Failure
   leaves the previous toolchain runnable.
3. **No hidden prerequisites.** Install and the no-Node golden path require no
   Rust, Node, package manager, or `sudo`.
4. **No project execution.** Install, doctor, and upgrade never execute
   application source, npm lifecycle scripts, or untrusted project binaries.
5. **Offline diagnosis.** Doctor is local-only unless `--network` is explicit.
6. **Stable automation.** Doctor and upgrade expose schema-versioned JSON and
   defined exit behavior.
7. **Public types only.** `@tysel/types` excludes underscored host bindings and
   incidental implementation details.
8. **Production remains one file.** Toolchain lifecycle logic is never required
   by a packaged application.

## Shared distribution foundation

This foundation is required before implementing installer or upgrade behavior.

### Canonical targets

The first public developer-toolchain matrix is:

| Target | Gate |
| --- | --- |
| `linux-x64` | Declare and test minimum glibc/kernel support |
| `linux-arm64` | Native build and installer coverage |
| `darwin-x64` | Native archive and clean-machine test |
| `darwin-arm64` | Native archive; distinguish arm64 from Rosetta |

Windows is out of the first scope. Documentation should explicitly say so and
recommend WSL where appropriate.

Canonical target detection belongs in one Rust distribution module shared by
build, doctor, upgrade, and release validation. The POSIX shell installer must
duplicate a small `uname` mapping, with table-driven tests proving it agrees with
the Rust mapping.

### Binary build identity

All three executables gain side-effect-free `--version` and
`--build-info-json`. Build info includes:

- semantic version;
- source commit;
- canonical target;
- release identity/channel metadata needed to detect mixed artifacts.

The service must return build info before trying to read an appended TAP. The
worker must return it before opening IPC. CI compares all three documents in
every release archive.

### Release metadata

Each immutable version has a machine-readable release manifest. A mutable,
signed channel pointer may select the stable immutable manifest, but immutable
manifests and assets are never replaced.

The release manifest includes:

- schema version, Tysel version, source commit, publication time, and channel;
- canonical target, archive URL, byte size, SHA-256, and signature reference;
- expected binary paths and individual hashes;
- binary build identity and minimum platform requirements;
- minimum compatible updater version;
- relevant TAP, capability ABI, and `@tysel/types` compatibility versions.

Unknown schema versions, targets, algorithms, or required compatibility
features fail closed. Forward-compatible optional fields are explicitly marked
by the schema rather than assumed.

### Managed layout

The default root is `${TYSEL_HOME:-$HOME/.tysel}`:

```text
~/.tysel/
  versions/
    v0.1.0/
      bin/
        tysel
        tysel-service
        tysel-worker
      share/acceptance/...
      release-manifest.json
      LICENSE
      README.md
  bin -> versions/v0.1.0/bin
  state.json
  upgrade.lock
```

Replacing the single `bin` link atomically switches all executable names. Stage
under the same root to avoid cross-filesystem activation. Retain the immediately
previous version for rollback.

`state.json` is versioned convenience state, not a trust root. It records active
and previous versions, channel, target, install method, and manifest digest.
Signed metadata and verified hashes remain authoritative.

### Bootstrap trust decision

The present signature path assumes an already trusted Tysel verifier and cannot
authenticate the first downloaded binary before execution. Before public launch,
choose and document one model:

1. HTTPS plus release SHA-256 for bootstrap, then signed manifests for every
   `tysel upgrade`; or
2. an independent portable verifier or platform bootstrap with a pinned trust
   root.

Model 1 is practical for the first preview, but its checksum detects corruption,
not compromise of the distribution origin. Product copy must state that limit.
Model 2 is the long-term option if first-install publisher authentication is in
scope.

## Track A: `install.sh`

`install.sh` is a small POSIX bootstrapper. It is not a second package manager;
after bootstrap, `tysel upgrade` owns lifecycle management.

### User interface

```sh
curl -fsSL https://tysel.dev/install.sh | sh
curl -fsSL https://tysel.dev/install.sh | sh -s -- --version 0.1.0
curl -fsSL https://tysel.dev/install.sh | sh -s -- --no-modify-path
```

Initial inputs:

| Input | Meaning |
| --- | --- |
| `--version <semver>` | Install an immutable version |
| `--channel stable` | Select the stable channel |
| `--prefix <absolute-path>` | Override the managed root |
| `--no-modify-path` | Do not edit a shell startup file |
| `--dry-run` | Resolve target, URLs, and paths without writing |
| `TYSEL_HOME` | Persistent root override |
| `TYSEL_DOWNLOAD_BASE` | Explicit CI, mirror, or enterprise endpoint override |

Reject unknown options, invalid versions, relative roots, unsupported targets,
and roots owned by another user. Never invoke `sudo`.

### Transaction

1. Detect target, including Rosetta handling.
2. Resolve channel to immutable manifest/version.
3. Download with bounded redirects, retries, connect/total timeouts, and normal
   proxy environment support.
4. Validate manifest, size, SHA-256, and the chosen bootstrap trust policy.
5. Inspect archive members before extraction. Reject absolute/traversal paths,
   unexpected links/devices, undeclared executable locations, missing binaries,
   and size limits.
6. Extract into same-filesystem staging under a restrictive umask.
7. Check executable permissions and matching build identities.
8. Move the verified version into `versions/` and atomically activate `bin`.
9. Write state while retaining the previous version.
10. Unless disabled, add one idempotent, marked PATH block to a supported
    user-owned shell startup file; report the exact file changed.
11. Run `tysel doctor --install`, or equivalent inline checks before doctor is
    available.
12. Print version, target, install path, PATH action, and the exact `tysel init`
    next step.

Any pre-activation failure removes staging without changing active state. Any
post-activation failure restores the prior link and state.

### Tests

Use a local fixture server, not the live release endpoint. Cover:

- four target mappings, stable and pinned versions;
- paths with spaces, repeat installation, and existing managed installs;
- bad checksum, truncation, traversal, missing/mixed companions, unknown schema,
  HTTP errors, read-only roots, and concurrent installers;
- supported shell PATH edits and `--no-modify-path`;
- clean-machine `--version`, `init`, `check`, and `test` without Node or Rust.

## Track B: `tysel doctor`

Doctor is the user support command and the reusable install/upgrade preflight
engine. Checks return structured results consumed by separate human and JSON
renderers.

### User interface

```text
tysel doctor [--project <path>] [--install] [--network] [--json]
```

- Default: managed installation, platform, and nearest `tysel.toml` if present.
- `--install`: installation/platform subset suitable for installer use.
- `--project`: select a project without executing it.
- `--network`: explicitly enable channel, DNS, TLS, proxy, and asset checks.
- `--json`: emit schema-versioned output without decoration.

Defer `--fix`. Version one produces exact remediation but performs no mutation,
package install, startup-file edit, or implicit network access.

### Check groups

| Group | Checks |
| --- | --- |
| Installation | Executable location, managed/unmanaged mode, sibling presence/permission, build-identity equality, manifest hashes, state/link consistency, PATH shadowing, writable upgrade root |
| Platform | Canonical target, minimum OS/libc/kernel, Rosetta mismatch, temp directory; correctly scoped Linux Landlock/seccomp/cgroup probes |
| Project | Manifest and entry, non-executing bundle/compat scan, profile, companion availability, optional `tsc`, installed types/test packages and version compatibility |
| Network | Stable pointer, clock sanity for signed data, TLS/proxy reachability, immutable manifest and target asset |

Checks use stable IDs such as `install.companion-version` and
`project.types-version`. Results contain `id`, `status`, `summary`, optional
details, and remediation. Status is `pass`, `warn`, `fail`, or `skip`.

Exit 0 means no failed check; warnings remain exit 0. Fatal argument/configuration
errors retain the global CLI error envelope. Output never includes secret values,
database URLs, authorization headers, or project source.

### Acceptance

- Healthy local-only doctor completes in under two seconds.
- Missing/mixed companions produce stable, actionable failures before build/dev.
- Source builds are reported as unmanaged warnings, not corrupt installs.
- JSON ordering/schema and secret redaction are snapshot-tested.
- Project checks never run user code or lifecycle scripts.

## Track C: `tysel upgrade`

Upgrade consumes the shared target, channel, release-manifest, verification, and
managed-layout code. It never scrapes HTML or independently invents asset names.

### User interface

```text
tysel upgrade [--check] [--version <semver>] [--channel stable]
              [--yes] [--force] [--rollback] [--json]
```

- Default: check, show transition, and confirm on an interactive terminal.
- `--check`: no mutation.
- `--yes`: required for non-interactive mutation.
- `--version`: select immutable version; downgrade also requires `--force`.
- `--rollback`: validate and activate the retained previous version.
- `--json`: stable output; incompatible with interactive prompting.

Version one upgrades only installations owned by `install.sh`/`tysel upgrade`.
It refuses source builds, unknown layouts, and package-manager-owned binaries
with owner-specific remediation.

### Transaction

1. Acquire `upgrade.lock` with a bounded wait.
2. Run installation doctor and reject an unsafe base.
3. Resolve and authenticate channel and immutable manifest using the currently
   trusted CLI's trust root.
4. Enforce semver, target, minimum updater, and compatibility rules.
5. Download and stage the complete release unit.
6. Verify archive/binary hashes, signature, identity, permissions, and policy.
7. Run install doctor against staging.
8. Atomically switch `bin` and update state.
9. Run post-switch doctor; restore prior link/state on failure.
10. Retain previous version and delete only older inactive verified versions.

The running updater may finish from the old executable; new invocations resolve
the new link. Upgrade never migrates application durable data.

### Acceptance

- Cover no-update, check-only, update, pin, downgrade rejection, rollback, and
  concurrency.
- Every verification/post-switch failure leaves the prior install runnable.
- Reject replayed/expired/revoked trust data, wrong target, malformed schema,
  mixed versions, and unsupported minimum updater.
- Discard partial downloads unless resumable chunks are cryptographically bound.
- Require no Node, Rust, npm, or administrator access.

## Track D: `@tysel/types`

`@tysel/types` becomes the declaration-only contract between application code
and the native runtime. It has no runtime dependency and replaces example-local
Tysel global declarations.

### Public surface

| Area | Required types |
| --- | --- |
| Values | JSON values, SQL parameters/results, opaque secret reference |
| Application | `TyselApp`, fetch handler, request/task context, Cron, Queue, MCP, durable handlers |
| Core host | Global `tysel`, `sleep`, `echo`, supported public identity and result/error shapes |
| Capabilities | Secrets, SQLite, Postgres, filesystem, LLM request/response/usage, durable start/signal |
| Durable | `step`, `effect`, `sleep`, `waitForSignal`, `retry`, `now`, `random` with useful generics |
| Web extensions | Accepted WebSocket and `WebSocket.opened` augmentation without copying `lib.dom` |
| Profiles | Public execution/trust profiles and capability requirements |

Before shipping, classify callable but undocumented names as public, deprecated,
or internal. Exclude `_httpStart`, `_sqliteExec`, `_durableLookup`, raw IPC, and
other underscored bindings.

`packages/tysel` imports/re-exports app and durable types instead of duplicating
them. `@tysel/test` owns `test`/`assert` but consumes common app types. Generated
projects explicitly include both type packages until loading is proven across
npm, pnpm, and Yarn.

### Publication and versioning

- Remove `private`; compile declarations into `dist`; define `types`, conditional
  exports, file whitelist, license/readme, and provenance.
- Never publish workspace source paths or depend on monorepo resolution.
- Before 1.0, synchronize native toolchain, `@tysel/types`, and `@tysel/test`
  versions. `tysel init` pins its compatible package versions.
- CI checks Cargo version, npm versions, generated templates, and release
  compatibility metadata together.
- Decide before 1.0 whether types stay release-locked or use an explicit runtime
  compatibility range. Never infer compatibility from npm `latest`.

### Drift prevention

1. Runtime integration tests snapshot public `globalThis.tysel` keys and nested
   capability keys.
2. Strict TypeScript fixtures exercise all APIs, inference, augmentation, and
   representative invalid calls. Remove example-local Tysel shims.
3. CI packs the package, installs it into an external temporary consumer, and
   checks minimum/current supported TypeScript versions.

Every runtime API change must update or affirm the runtime snapshot, type
fixture, and runtime API documentation.

### Acceptance

- All TypeScript examples compile without local Tysel global declarations.
- After optional package installation, generated projects perform a real type
  check rather than skipping due to unavailable packages.
- Database, LLM, and durable APIs have useful generics and no accidental `any`.
- Packed output is declaration-only, resolves in supported module modes, and has
  no install scripts.
- Native archives cannot publish if matching packages are missing or consumer
  fixtures fail.

## Milestones and dependency order

| Milestone | Scope | Dependency | Exit gate |
| --- | --- | --- | --- |
| T0 — Distribution foundation | Shared target/build identity, manifest schemas, managed layout, synchronized versions | None | Release CI proves three matching identities per supported target |
| T1 — Types MVP | Complete types, SDK/test deduplication, init update, pack/consumer tests | Version policy from T0 | Examples compile without local shims; publishable package |
| T2 — Doctor | Read-only install/platform/project checks and stable JSON | T0 identity/layout; T1 package metadata | Mixed installs and incompatible types diagnosed deterministically |
| T3 — Cross-platform archives | Darwin x64/arm64, portable packaging, checksums/signatures, platform baselines | T0 | Four immutable archives pass clean-machine tests |
| T4 — Installer | POSIX bootstrap, atomic layout, PATH integration, recovery | T2 and T3 | One line reaches passing `init/check/test` on four targets |
| T5 — Upgrade | Signed resolution, staging, atomic switch, rollback, JSON | T2 and T4 | Fault injection proves prior install survives every failed stage |
| T6 — Hardening/adapters | Independent bootstrap verification, mirrors, Homebrew, retention, support bundle | Stable T4/T5 evidence | Adapters preserve the same release identity and atomicity |

T1 and T3 can proceed in parallel after T0. T2 can start local checks while T3
finishes. The website hero one-liner launches only after T4 passes on both Darwin
architectures; an explicitly labeled Linux preview can ship earlier.

## Implementation map

| Area | Primary repository changes |
| --- | --- |
| Distribution model | Shared Rust module/crate for targets, identity, manifests, state, validation; remove private target duplication |
| Companions | Early build-info argument paths in service/worker binaries |
| Release | Separate platform build, portable archive, evidence, and centralized signing stages in release workflow/scripts |
| Installer | Reviewed POSIX script published byte-for-byte to `tysel.dev`; local fixture tests |
| Doctor | CLI module with reusable result types and independent human/JSON renderers |
| Upgrade | CLI module plus injectable retrieval, verification, locking, staging, and activation services |
| Types | Expand `packages/tysel-types`; update SDK/test/init/examples; add package and consumer fixtures |
| CI | Version sync, drift, installer fault, doctor snapshots, four-target golden path, npm pack, staged publishing |

Avoid putting network or mutation directly in CLI argument dispatch. Inject
release retrieval, verification, locking, staging, and activation so failure
tests do not touch the live network or real user installation.

## Stable release gates

1. Four-target build and three-binary identity equality.
2. Platform-appropriate archive policy, checksum/signature, and reproducibility.
3. Installer fault injection and clean-machine golden path.
4. Doctor human/JSON snapshots and redaction tests.
5. Upgrade interruption, rollback, trust, and concurrency tests.
6. Packed npm consumer and runtime/type drift tests.
7. Consistent README/install/getting-started requirements, platform status,
   optional Node path, upgrade, uninstall, and security limitations.

Stage and verify npm artifacts and native archives first. Publish the matching
immutable version set, then advance the signed stable channel pointer last, so a
generated project never references an unavailable type package.

## Success metrics

Collect from CI and opt-in aggregate release evidence rather than invasive
installer telemetry:

- install success and failure stage by target;
- download bytes/time, verification time, and total install time;
- golden paths reaching `tysel test` without Node;
- doctor failure IDs included in support reports;
- upgrade success, rollback, and mixed-version prevention;
- runtime/type drift caught before release;
- packed package size and external-consumer type-check time.

The product KPI is time to first passing Tysel test. Network performance is
reported separately and never substitutes for correctness.
