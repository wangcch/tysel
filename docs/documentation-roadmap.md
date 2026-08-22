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
