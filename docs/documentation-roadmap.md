# Documentation audit and optimization roadmap

Status: active internal plan
Baseline date: 2026-08-23
Canonical public language: English
Reference experience: [Bun documentation](https://bun.sh/docs) and
[Bun guides](https://bun.sh/guides)

This file evaluates the current repository documentation and turns the website
plan into measurable documentation work. It is excluded from the public site.
Product behavior must still be verified against code, schemas, tests, and a
named release before publication.

## Executive assessment

Tysel already documents its product boundaries, security model, release
evidence, and operational responsibilities more honestly than many projects at
the same maturity. The main risk is not lack of raw material. It is that strong
material is concentrated in a small number of dense pages and organized around
repository taxonomy rather than the reader's job.

Before this optimization tranche, the public documentation contained 28
Markdown pages and about 1,900 lines. Every page appeared in MkDocs navigation,
but there was no guide index, example gallery, reference landing page, ADR
landing page, machine-readable documentation index, or automated documentation
quality gate. Four public pages linked outside `docs/` with repository-relative
paths that would not resolve correctly on a separately hosted documentation
site.

The highest-return work is therefore progressive disclosure:

1. make the first decision obvious;
2. separate task guides from concepts and reference;
3. expose complete examples inside the documentation experience;
4. split or generate dense reference surfaces;
5. prevent documentation drift in CI.

## Scorecard

Scores describe the implemented repository documentation before the first
optimization tranche, not the future website specified in `website-plan.md`.

| Dimension | Score | Evidence | Main action |
| --- | ---: | --- | --- |
| Product truth and scope | 8/10 | Release availability, host-target builds, Linux isolation scope, and Node.js exclusions are explicit. | Add consistent stability, profile, and platform metadata to every relevant page. |
| Information architecture | 5/10 | Navigation covers all pages but primarily mirrors internal categories; no task or reference landing pages. | Add task entry points and keep exhaustive detail behind reference routes. |
| First successful run | 6/10 | The complete path exists, but the source-build prerequisite dominates and expected output is sparse. | Add verified expected output, troubleshooting, and a release-aware install switch. |
| Guide coverage | 4/10 | Useful procedures are embedded in concepts, the runbook, and example READMEs. | Publish atomic outcome-oriented guides with verification and recovery steps. |
| Reference completeness | 5/10 | CLI, manifest, runtime, and compatibility references exist, but CLI and runtime surfaces are highly compressed. | Generate command and schema reference, then split runtime APIs by concern. |
| Examples | 7/10 source / 3/10 discovery | Nine useful examples or SDK examples exist outside the public docs tree. | Maintain a filterable gallery and link each guide to a complete source revision. |
| Security and production evidence | 8/10 | Security boundaries, release admission, recovery, and evidence rules are unusually concrete. | Add evaluation checklists and keep quantitative claims bound to evidence. |
| Search and reading experience | 4/10 | Basic MkDocs navigation exists; the planned command menu, page actions, badges, and versioning are not implemented. | Implement the Fumadocs shell after content routes stabilize. |
| Maintainability and drift control | 4/10 | Several sources of truth are named, but documentation has no dedicated CI validation or generation. | Validate links/navigation now; generate CLI, schema, and compatibility pages next. |
| Machine and agent access | 1/10 | No `llms.txt`, compact index, or per-page Markdown action existed. | Publish full and compact indexes from the same content graph. |

## What to learn from Bun

The useful Bun pattern is not its product positioning. Bun is an all-in-one
toolkit and aims for broad Node.js compatibility; Tysel has a narrower runtime
and deployment contract. The transferable documentation patterns are:

- **Clear product gateways.** Bun's documentation home presents major product
  areas first, then installation and quick start. Tysel should present services,
  bounded execution, tasks, and durable work with one obvious action each.
- **At-a-glance before depth.** Bun reference pages state the primary command or
  API before long explanations. Tysel pages should lead with outcome, profile,
  required capability, minimal example, and important limit.
- **Guides are separate from reference.** Bun guides answer narrow jobs while
  reference pages describe a complete surface. Tysel should stop asking one
  page to serve tutorial, concept, and exhaustive reference roles.
- **Granular, stable routes.** A specific runtime behavior or test behavior has
  its own address and can be searched or linked directly. Tysel's CLI and
  runtime API need similar granularity, preferably generated from source.
- **Progressive examples.** Installation and common commands appear early;
  details, flags, and edge cases follow. Tysel should preserve its careful
  caveats without putting every caveat before the first success.
- **Machine-readable discovery.** Bun advertises a full documentation index for
  AI tools. Tysel should generate `llms.txt`, `llms-small.txt`, page Markdown,
  and canonical routes from one graph.

Patterns not to copy:

- Node.js replacement language or general npm compatibility claims;
- performance marketing without release, hardware, workload, and raw evidence;
- a mascot or visual vocabulary unrelated to Tysel's existing identity;
- public version selectors or stability claims before tagged releases exist;
- hand-maintained exhaustive CLI/API prose that can silently drift from code.

## Documentation goals

### G1 — First value without ambiguity

A TypeScript backend developer should identify the correct starting path within
30 seconds and complete the first verified HTTP service within 10 minutes after
the toolchain is available.

Measures:

- one primary quick start with copied commands and expected output;
- no more than two navigation decisions from the docs home to any launch job;
- explicit source-build versus tagged-release installation state;
- a troubleshooting path for every blocking quick-start step.

### G2 — Task discovery before taxonomy

The top evaluation and adoption jobs should be reachable by intent, not by
knowing Tysel's internal module names.

Measures:

- guide landing page covers at least eight launch jobs;
- every guide names outcome, time, release/platform/profile, prerequisites,
  verification, failure recovery, and next step;
- every runnable example appears in one gallery with profile and capability
  requirements;
- concepts and references link back to at least one relevant task.

### G3 — Reference that cannot drift silently

Command, configuration, compatibility, and type contracts should be generated
or checked against their implementation sources.

Measures:

- CLI command and option inventory matches the installed `--help` graph;
- manifest fields, defaults, and enums come from the bundled JSON Schema;
- Web API status comes from the runtime compatibility inventory;
- public runtime signatures come from `@tysel/types`;
- changed source contracts fail CI when generated documentation is stale.

### G4 — Publishable and machine-readable by default

The same content graph should serve human navigation, search, sitemap, and
machine-readable discovery.

Measures:

- zero broken local links and zero repository-relative escapes from public docs;
- every public Markdown page has exactly one H1 and a unique route;
- 100% of public Markdown pages are represented in navigation;
- full and compact LLM indexes ship on the docs origin;
- canonical URL, edit source, and release scope are available on every page.

### G5 — Evidence-bound adoption decisions

Readers should be able to distinguish implemented behavior, experimental
surface, platform-specific guarantee, and future plan without inference.

Measures:

- every security or performance claim links to its contract or evidence;
- quantitative cards name version, commit, artifact, environment, workload,
  aggregation, raw samples, and reproduction command;
- experimental and Linux-production surfaces use consistent visible labels;
- internal milestones, open decisions, and backlogs never enter public search.

## Prioritized backlog

### P0 — foundation

- Replace the link-directory home with task selection, a five-minute path, an
  at-a-glance product map, and visible boundaries.
- Add guide, example, reference, and ADR landing pages.
- Convert repository-relative source links to stable repository URLs.
- Add `llms.txt` and `llms-small.txt`.
- Validate navigation, local links, anchors, unrecognized links, and
  public/private boundaries with MkDocs native strict validation in CI.
- Add repository and edit-page metadata to the interim MkDocs site.

### P1 — launch learning paths

- Turn the first service into a tested golden path with captured expected
  output and troubleshooting.
- Publish focused guides for JSON API, Postgres, durable approval, MCP, Queue,
  Cron, isolated plugins, container packaging, systemd, and CI.
- Add dedicated compatibility, limits/defaults, error-code, and environment
  variable indexes.
- Add page metadata for stability, profile, capability, platform, verified
  release, and source revision.
- Add redirects for any route renamed during the Fumadocs migration.

### P2 — generated reference

- Generate one page per CLI command from the command graph and supplement it
  with hand-authored examples.
- Generate manifest field tables from the JSON Schema.
- Generate the Web API matrix from its JSON inventory.
- Extract public runtime signatures and cross-link them to guides.
- Verify every documentation code sample in an example project or bounded
  snippet harness.

### P3 — documentation experience

- Implement the planned Next.js and Fumadocs shell on the final route graph.
- Add indexed search, command menu, copy buttons, filename labels, stable
  anchors, previous/next navigation, edit links, and Markdown page actions.
- Add accessible stability/profile/platform badges and responsive tables.
- Meet WCAG 2.2 AA, reduced-motion, keyboard, zoom, and contrast acceptance.
- Add version selection only after two supported public release lines exist.

## First optimization tranche

This tranche starts P0 with framework-independent improvements:

- outcome-oriented documentation home;
- guide map and example gallery;
- reference and ADR landing pages;
- full and compact machine-readable indexes;
- stable links from hosted docs to source examples;
- native strict documentation build and link validation;
- MkDocs navigation and repository metadata aligned with the new entry points.

The next tranche should focus on one fully tested golden path and generated CLI
reference rather than adding more broad overview prose.

## Reference tranche update — 2026-08-23

The hand-authored reference foundation is now implemented on stable,
search-oriented routes:

- CLI reference split by project setup, development, task protocols, delivery,
  installation lifecycle, and release evidence;
- manifest reference split by application/server, permissions, limits,
  durable/observability, and task schema;
- runtime reference split by application exports, host capabilities, durable
  APIs, and `@tysel/test`;
- cross-cutting environment-variable, limits/defaults, and error/output indexes;
- `llms.txt`, site navigation, and existing public links updated to the same
  route graph;
- all pages validated by the existing MkDocs strict build in CI, avoiding a
  second overlapping documentation checker.

The source audit also exposed contract gaps that the previous dense pages hid.
`max_response_mb` and `max_in_flight` have since been propagated and enforced
by the packaged HTTP runtime; manifest trace and metric endpoints are not yet
runtime controls. The reference labels those facts explicitly.

G3 is therefore partially complete: public lookup and source reconciliation are
in place, but drift can still occur silently. The next reference work should be
generation and comparison, in this order:

1. export the Clap command graph and compare command/option inventory in CI;
2. render manifest field tables from the bundled JSON Schema;
3. render JavaScript compatibility from the versioned JSON inventory;
4. extract public signatures from `@tysel/types` and compile reference snippets;
5. fail the existing `docs:build` job when generated artifacts differ.

This should remain one strict documentation job. Separate bespoke link and
navigation scripts are not useful while MkDocs already validates those
properties; additional CI earns its cost only when it checks source-to-reference
drift that MkDocs cannot see.

## Product-surface coverage audit — 2026-08-24

This audit compares implemented CLI commands, public types, capabilities,
protocols, SDKs, examples, and operations against discoverable Reference and
task-oriented Guides. A symbol mention does not count as a usable guide; a
guide must contain prerequisites, commands, expected result, limits, recovery,
and a complete source path.

### Completed in the Wasm tranche

- promoted Wasm Components from one execution-profile paragraph to a dedicated
  six-page Reference covering the task ABI, restricted WASI, runtime/AOT,
  three-layer capability policy, and Rust/Go SDKs;
- added verified Rust and Go build/run/package guides;
- added runnable manifests to both SDK fixtures;
- integrated Component behavior into CLI, manifest, limits, errors,
  capability matrix, security model, example gallery, and machine indexes;
- distinguished Component Model binaries from unsupported Core Wasm and
  general WASI applications;
- marked the `0.0.1` HTTP, SQLite, LLM, secrets, MCP, and core WIT sketches as
  non-product contracts; only the task ABI and filesystem `0.4.0` imports are
  currently implemented.

### Reader-facing gap snapshot — 2026-08-24

This table records the state at the time of the audit. The dated P0 and P1
updates below supersede completed rows; it is retained as decision history,
not as the current backlog.

| Priority | Implemented product surface | Documentation at audit time | Required durable artifact at audit time |
| --- | --- | --- | --- |
| P0 | HTTP/1.1, cleartext HTTP/2, inbound and outbound WebSocket, outbound `fetch` | Compatibility/reference fragments only | Service networking guide with protocol setup, allowlists, streaming, errors, and verification. |
| P0 | Cron and Queue handlers | Public types and CLI reference; no complete repository examples | One Cron example and one Queue producer/handler guide with deadlines, JSON bounds, and failure behavior. |
| P0 | LLM gateway | Host API reference plus durable-agent usage | Standalone provider setup, secret selection, alias routing, timeout, usage, audit, and failure guide. |
| P0 | `server.workers` and bounded `max_in_flight` admission | Manifest field reference | Concurrency guide covering statelessness, per-isolate memory, overload `503`, WebSocket permit lifetime, and sizing evidence. |
| P1 | Filesystem, SQLite, and Postgres | Reference plus linked source examples | Focused capability guides with local setup, deployment injection, denial recovery, and production differences. |
| P1 | `tysel image` | CLI reference and production paragraph | End-to-end Linux image guide, plus non-Linux existing-ELF path and registry/signing boundary. |
| P1 | OTLP traces/metrics and structured logs | Environment reference and operations fragments | Observability guide with accepted endpoint forms, signal precedence, collector validation, redaction, and troubleshooting. |
| P1 | Source maps and structured runtime failures | Error reference | Debugging guide with development versus production output and safe error mapping. |
| P1 | Release evidence, artifact signing, and reproducibility | CLI reference and production policy | One reproducible release walkthrough with files produced and verification failure recovery. |
| P2 | TAP format, TaskRPC, scheduler leases/fencing, durable event model | Source and scattered architecture prose | Versioned Internals references, kept separate from application API stability. |
| P2 | Every CLI/schema/type reference | Hand-authored and strict-link checked | Generated drift checks and compiled snippets in the existing docs CI job. |

The scan also found implementation metadata drift: the workspace pins
Wasmtime `36.0.13`, while the Component AOT compatibility label remains
`32.0.1`. Track this as an engine/package fix; documentation now distinguishes
the two values instead of presenting the label as the runtime dependency.

### Recommended sequence

1. service networking plus concurrency/backpressure;
2. Cron and Queue runnable examples;
3. standalone LLM and observability guides;
4. container and signed release walkthroughs;
5. TAP, TaskRPC, scheduler, and durable internals;
6. generated source-to-document drift checks.

This order closes adoption blockers before documenting internal machinery. It
also gives every major homepage claim at least one task guide, one exact
reference, and one runnable verification path.

## P0 adoption-path update — 2026-08-26

The first P0 adoption tranche is implemented:

- service networking now has one task guide and a runnable inbound WebSocket
  example covering HTTP/1.1, h2c, TLS termination, outbound allowlists, redirect
  checks, and denial verification;
- concurrency and backpressure now has a sizing and overload guide covering
  per-isolate workers, `503 OVERLOADED`, response lifetime, WebSocket permits,
  retry behavior, and measurement order;
- Cron and Queue now have one combined runnable worker plus a guide covering UTC
  syntax, CLI production, message identity, deadlines, backpressure, catch-up,
  and the boundary with durable execution;
- the LLM gateway now has a standalone service and guide covering provider
  environment, alias routing, opaque secrets, validation, concurrency, timeout,
  usage, audit fields, and failure diagnosis;
- website CI now compares the documented inventory with the Clap command enum,
  manifest JSON Schema, and `@tysel/types` exports, then rejects uncommitted
  generated MDX changes;
- the website importer excludes architecture implementation notes and ADRs;
  both remain maintainer-only records in the repository and MkDocs workspace.

The second P0 adoption tranche is also implemented:

- filesystem now has a runnable transform and task guide covering root
  preparation, relative and absolute resolution, independent read/write
  authority, pinned directory descriptors, traversal/symlink denial, Unix
  scope, file types, encoding, size, and deployment mounts;
- SQLite now has a task guide covering path resolution, parameterization, the
  process-wide connection, busy timeout, fixed bounds, durable event-log
  separation, persistence verification, backup, and deployment selection;
- PostgreSQL now has a task guide covering grant-to-environment mapping,
  parameter and result types, read-only enforcement, safe SQLSTATE errors,
  four-connection pooling, TLS, database roles, and production checks;
- the example gallery and website catalog now treat Filesystem, SQLite, and
  PostgreSQL as complete runnable paths rather than reference-only recipes.

The reader-facing P0 adoption backlog is now complete. The next tranche should
proceed to the P1 image, observability, debugging, and reproducible-release
walkthroughs while continuing to extend contract-derived reference checks.

## P1 operations-path update — 2026-08-26

The first P1 operations tranche is implemented:

- the container guide covers listener admission, generated non-root context,
  Linux ELF validation, non-Linux `--binary` use, sidecar custody, base-image
  pinning, smoke testing, and the registry/signing boundary;
- the observability guide documents JSON and OTLP independence, active
  environment controls, accepted endpoints, exact metrics/spans and bounded
  attributes, redaction verification, alerts, and collector troubleshooting;
- the debugging guide maps every HTTP runtime envelope, correlation ID format,
  development-only source-map application, Component and CLI output
  differences, packaged working-directory drift, and safe application errors;
- the release guide inventories all application sidecars and verification
  failure recovery, then states the boundary between application evidence and
  maintainer-only toolchain reproduction without exposing that workflow.

The remaining P1 work is deeper worked deployment integration only; each major
implemented operations surface now has a discoverable task guide and exact
reference cross-links. P2 internals and compiled-snippet coverage remain the
next documentation tranche.
