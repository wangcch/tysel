# tysel.dev website plan v2

Status: implementation-ready content, information architecture, and experience plan
Domain: `tysel.dev`
Canonical launch language: English
Primary audience: TypeScript backend developers, agent builders, and platform engineers
Stack: Next.js App Router, Fumadocs Core, MDX, and custom visual components

This is an internal website plan. It may contain workstreams, open decisions, and a
content backlog. Those sections must not be published as product documentation. Public
pages describe only behavior available in a named release and use explicit stability and
platform labels.

## 1. Current product truth

The website must start from the implemented product rather than completed internal plans.
The repository has completed its production engineering acceptance scope and the
developer-experience milestone, but that internal milestone is not itself a public stable
version. The first complete public release uses `0.1.1`; Component ABI versions remain
independent from the product release version and channel.

### Implemented surface

- TypeScript services, tasks, and agents packaged as one native application executable
- Web-standard `Request`, `Response`, `fetch`, streams, URL, encoding, timers, and Web Crypto
- HTTP/1.1, cleartext HTTP/2, inbound WebSocket, and allowlisted outbound WebSocket
- Explicit grants for network, secrets, filesystem, SQLite, and Postgres
- Service, isolated, and experimental Component execution profiles
- Linux isolated workers with process separation, Landlock, seccomp, and best-effort
  cgroup memory enforcement
- Cron, Queue, MCP, LLM, and durable handlers using the bounded TaskRPC lifecycle
- Durable steps, effects, sleep, retry, signals, persisted wakeups, and restart recovery
- Project creation, checking, compatibility reporting, isolated tests, source-mapped
  failures, development reload, packaging, and container-image generation
- Managed diagnostics, authenticated upgrades, rollback, release evidence, signing,
  reproducibility, SBOMs, and benchmark harnesses
- Postgres durable storage, redacted OTLP export, and a production runbook

### Boundaries that must remain visible

- Tysel is Web-API-first and does not promise general Node.js compatibility.
- Node native addons, ambient host authority, subprocesses, and arbitrary POSIX behavior
  are not part of the runtime contract.
- `tysel build` embeds the application bundle and native runtime into one executable. It
  does not claim arbitrary TypeScript AOT compilation to machine code.
- The developer toolchain installs `tysel`, `tysel-service`, and `tysel-worker`. A built
  application is still delivered as one executable.
- `tysel build --target` does not currently cross-compile. The target must match the host.
- Container packaging on a non-Linux host requires an existing Linux executable.
- Linux is the production security gate for the isolated profile. macOS is a supported
  development platform, not an equivalent sandbox claim.
- Implemented Durable behavior does not automatically make every public API stable.
- Tysel does not provide a hosted cloud platform.

## 2. Product story

Tysel should not present itself as another general-purpose JavaScript toolchain. Its
strongest story is:

> **Write TypeScript. Ship a binary.**

> A lightweight native TypeScript runtime for services and AI agents.

The website makes three promises, in this order:

1. **Ship one executable** — turn a TypeScript workload into one production artifact.
2. **Bound every capability** — grant only the resources code needs.
3. **Resume durable work** — persist effects, retries, signals, and suspension so work can
   continue after a restart.

The first visit should answer:

- What is Tysel?
- How is its production contract different from Node.js, Bun, and Deno?
- Can I build and run a useful example in a few minutes?
- What evidence and limits should I inspect before adopting it?

### Core transformation

| Before | With Tysel | Meaning |
| --- | --- | --- |
| Deploy an environment | Copy and run one executable | Delivery becomes an artifact problem |
| Trust application code | Grant explicit capabilities | Trust moves from code to enforced boundaries |
| Keep a worker alive | Suspend and resume durable work | Long-running intent does not require resident execution |

Chinese manifesto line:

> Tysel 让 TypeScript 从一种需要部署运行环境的语言，变成一种可以直接交付为可执行文件的语言。

Public English form:

> Tysel makes TypeScript directly shippable—as a single, capability-bounded executable.

The short slogan is the memorable expression. The longer statement explains it rather
than competing with it.

### Message hierarchy

1. Lightweight native TypeScript runtime for services and AI agents
2. Deploy one artifact instead of a JavaScript environment
3. Explicit capabilities and bounded execution profiles
4. Durable work that can suspend and resume
5. Compatibility, isolation, recovery, and release evidence

The AI-era framing comes after the runtime is understood. Lead with delivery, then show
why inexpensive isolation and durable execution matter for generated or third-party code.

### Competitive frame

> Bun and Deno optimize the general JavaScript runtime and toolchain. Tysel is designed
> around a narrower production contract: small deployable artifacts, capability-bounded
> execution, and durable TypeScript services and AI agents.

Do not describe Tysel as an all-in-one toolchain, package manager, Node.js replacement,
browser runtime, or hosted agent platform.

## 3. Public status and claims

### Release labels

The website reads the public version and channel from release metadata. It must not derive
a public label from internal milestone or acceptance-document names.

- Do not show a production version label until a corresponding public release exists.
- Hide the announcement bar when there is no meaningful public announcement.
- Hide the documentation version selector until two supported public versions exist.
- Examples that depend on release behavior link to the version they were verified against.

### Stability badges

- **Stable** — supported public contract for the current release line
- **Experimental** — may change without the stable compatibility guarantee
- **Platform-specific** — behavior or guarantees differ by operating system
- **Linux production** — security or operational guarantee validated only on Linux

Each reference page also states its execution profile and required capability.

### Safe factual claims

- Produces one native application executable.
- Production does not require Node.js, V8, npm, or `node_modules`.
- Host resources are deny-by-default and explicitly granted.
- The Linux isolated profile adds a worker process, Landlock, seccomp, and best-effort
  cgroup memory enforcement.
- Durable handlers persist suspension, signals, retries, and replayable effects.
- Release builds can emit deterministic evidence, compatibility, SBOM, license, and
  checksum sidecars.

Every claim still links to the relevant contract or evidence.

### Claims requiring named release evidence

- Exact executable size, cold-start time, memory usage, or isolate creation time
- Comparison with Docker, Node.js, Bun, Deno, or another sandbox
- Density or scale claims such as running 1,000 isolated functions
- Security claims beyond the documented threat model

Every quantitative claim appears in a benchmark card containing:

- Tysel version and source commit
- OS, architecture, CPU, and build profile
- Workload and artifact contents
- Metric definition, raw samples, and aggregation
- Comparison versions and equivalent configuration, when applicable
- Reproduction command and evidence download

The repository currently uses release-admission thresholds of at most 20 MiB artifact
size, 15 ms median cold start, 32 MiB idle Linux PSS, 5 ms warm isolate creation p50, and
10 ms durable resume p50. These are gates, not measured marketing results.

### Wording to avoid

- “The only runtime that…”
- “Node, Bun, and Deno have no isolation.”
- “Process-level security” without Linux scope
- “Persistence is a TypeScript language feature.”
- “Native compilation” when the intended claim is a native executable containing the
  runtime and application bundle
- “Exactly once” without naming the persisted boundary and external side-effect contract

## 4. Website architecture

### Technology stack

```text
Next.js App Router
├── Marketing and documentation routes
├── Metadata, sitemap, social cards, and redirects
└── Content and search endpoints where needed

Fumadocs Core
├── Documentation source and navigation model
├── Article layout and table of contents
├── Search integration
└── Generated page metadata

MDX
├── Hand-authored guides and concepts
├── Runnable examples and expected output
└── Custom documentation components

Custom visual system
├── Brand tokens and themes
├── Marketing sections
├── Terminal playback
├── Capability and durable-flow diagrams
└── Benchmark and evidence cards
```

Fumadocs provides documentation structure, not the final identity. Navigation, search,
code blocks, callouts, badges, and article layout share Tysel components with marketing.

### Content model

- MDX is canonical for public narrative content.
- CLI, manifest, runtime API, and compatibility tables are generated from code or schemas
  where practical. Manifest reference uses the bundled Draft 2020-12 schema; project
  discovery and task semantics use their CLI sources.
- Generated reference is never a second editable source of truth.
- Existing MkDocs content is migration input and a link-validation baseline only.
- Completed planning and acceptance records stay outside the public content collection.

### Route model

```text
tysel.dev/                         Product homepage
tysel.dev/docs/                    Documentation home
tysel.dev/docs/getting-started/    First successful application
tysel.dev/docs/guides/             Task-oriented learning paths
tysel.dev/docs/concepts/           Mental models and guarantees
tysel.dev/docs/reference/          CLI, manifest, API, compatibility, and limits
tysel.dev/examples/                Runnable example gallery
tysel.dev/benchmarks/              Evidence and methodology
tysel.dev/blog/                    Releases and engineering notes, when populated
```

Preserve useful old URLs with permanent redirects. Do not duplicate a guide under both
`/guides/` and `/docs/guides/`.

### Global navigation

```text
Tysel | Docs | Examples | Benchmarks | GitHub
```

Add Blog only when it has durable content. Guides and Reference live inside Docs.

Global utilities:

- Search and command menu (`⌘K` / `Ctrl K`)
- Theme selector
- GitHub link
- Persistent **Get started** action on marketing pages
- Version selector only when multiple supported versions exist
- GitHub star count only after the public organization and repository are final

Marketing and docs remain on one origin so search, navigation, metadata, analytics, and
visual language feel like one product.

## 5. Homepage experience

The homepage moves from comprehension to evidence:

```text
Understand the artifact
→ See the trust boundary
→ See durable continuation
→ Choose a workload
→ Complete the developer loop
→ Inspect production evidence
```

### 5.1 Announcement bar

Render only for a real release or major available capability. Source its label and link
from release content. Do not use a generic welcome message or internal milestone name.

### 5.2 Hero

> **Write TypeScript. Ship a binary.**
>
> Build TypeScript services and AI agents as a single executable—with explicit
> capabilities and durable execution.

Primary CTA: **Get started**
Secondary CTA: **View on GitHub**

The interactive object shows one complete flow:

1. A small `fetch` handler in `src/index.ts`
2. `tysel build --release`
3. One named executable with an expandable release-evidence view

Use deterministic, user-controlled playback captured from a named release. It must not
run a build in the browser. Show release, platform, output name, and result. Provide play,
pause, replay, and direct step controls.

If an install command appears in the hero, use the current official release location. Do
not publish `https://tysel.dev/install.sh` until that endpoint exists and redirects to
authenticated release artifacts.

Include this clarification near the demo:

> The developer toolchain installs three native binaries. Your application ships as one.

Do not imply arbitrary TypeScript AOT compilation.

### 5.3 Immediate proof

- Web API first
- No Node.js or `node_modules` in production
- Deny-by-default capabilities
- Durable restart recovery

Each item links to a relevant page. Do not place point measurements here.

### 5.4 Three product contracts

#### Ship one executable

Show `tysel check` → `tysel test` → `tysel build --release`. Let users inspect the
executable, checksum, compatibility report, SBOM, and evidence index without suggesting
that sidecars are separately deployed runtimes.

#### Bound every capability

Show a small TOML manifest grant beside the code that uses it, with a JSON-format switch
for teams that standardize on JSON. Demonstrate one allowed and one denied operation. The
homepage introduces service and isolated profiles; the threat model lives in documentation.

The public contrast is “trust boundaries, not application code.” The defensible story is
the combination of explicit capabilities, supervisor-held secrets, restricted host APIs,
the Linux isolated worker, and single-artifact delivery.

#### Resume durable work

```text
LLM call → persisted effect → human approval → restart → resume → save once
```

Describe durability as a runtime primitive and programming model. Link to the runnable
example and explain deterministic boundaries and side-effect rules in its concept page.

### 5.5 Workload selector

- **HTTP service** — Fetch handlers, HTTP/1.1 and HTTP/2, WebSocket, SQLite, Postgres
- **AI agent** — LLM gateway, durable effects, signals, approvals, restart recovery
- **Task worker** — Cron, Queue, MCP, TaskRPC leases, retry, timeout, cancellation

Each tab contains realistic code, required permissions, its execution profile, and one
guide action. Keyboard focus and screen-reader state follow the selection.

### 5.6 Five minutes with Tysel

1. Verify installation: `tysel doctor --install`
2. Create: `tysel init hello-tysel`
3. Resolve configuration: `tysel config validate`
4. Validate: `tysel task verify`
5. Run and package: `tysel dev`, then `tysel task release`

Show expected output from a named release. Node.js is optional for editor declarations and
TypeScript compiler feedback, not required by the runtime or packaged application.

### 5.7 Production evidence

- Compatibility and release evidence
- SBOM, licenses, checksums, and signatures
- Reproducible Linux archives
- Security and isolation contract
- Benchmark methodology and raw samples
- Production operations runbook

Benchmarks come after product comprehension. Every chart identifies release, artifact,
hardware, workload, sample method, and reproduction command.

### 5.8 Final CTA

> From TypeScript source to one production artifact.

Primary: **Build your first service**
Secondary: **Run the durable agent example**

## 6. Documentation information architecture

Use progressive disclosure. Keep first-level groups short and exhaustive detail in
Reference.

### Start

- Welcome to Tysel
- Install and verify
- Quick start: HTTP service
- Quick start: durable agent
- How Tysel works
- Project structure
- Project discovery and `-C`
- TOML and JSON configuration
- Choose an execution profile

### Build services

- Request and Response handlers
- HTTP/1.1 and HTTP/2
- Outbound fetch
- Inbound and outbound WebSocket
- Timers, streams, encoding, and Web Crypto
- SQLite, Postgres, and filesystem access

### Build agents and tasks

- Task model overview
- LLM generation gateway
- Durable execution
- Steps and effects
- Sleep, retry, and deterministic values
- Signals and human approval
- Cron, Queue, and MCP handlers
- Leases, cancellation, fencing, and late-result rejection

### Capabilities and security

- Capability model and manifest permissions
- Secrets and opaque handles
- Service and isolated profiles
- Linux isolation boundary
- Component profile and stability
- Resource limits
- Threat model and non-goals

### Develop and test

- Project discovery and root-relative execution
- `tysel.toml` and `tysel.json`
- Interactive and non-interactive `tysel init`
- `tysel config` inspection, conversion, and JSON Schema
- Manifest-native `tysel task` workflows
- Development server and reload
- Type checking and capability scanning
- Test runner
- npm and Web API compatibility
- Structured errors, source maps, logs, and debugging

### Build and ship

- Build one executable
- Host targets and profiles
- Container images
- Release evidence and SBOMs
- Sign and verify artifacts
- Reproducible builds
- CI guide

The target guide states that cross-compilation is not implemented. Container guides
distinguish building on Linux from supplying a Linux ELF on another host.

### Install and manage

- Managed toolchain layout
- Diagnose with `tysel doctor`
- Upgrade and trust refresh
- Roll back
- Release channels and immutable versions
- Build from source

### Operate

- Production checklist
- Configuration and secrets
- TLS termination and ingress
- Durable Postgres
- Backup and restore
- Capacity and resource sizing
- OpenTelemetry
- Upgrade and application rollback
- Monitoring, alerts, and incident response

### Reference

- CLI: `init`, `config`, `task`, `check`, `compat`, `test`, `dev`, `run`, `queue`,
  `mcp`, `inspect`, `doctor`, `upgrade`, `build`, `image`, `bench`, and `release`
- Project discovery and path-resolution rules
- TOML/JSON Manifest schema and conversion
- Runtime and Durable APIs
- Supported Web APIs
- HTTP and WebSocket behavior
- Environment variables
- Capability matrix
- npm compatibility catalog
- Limits and defaults
- Error codes and machine-readable schemas
- Release compatibility guarantees

### Internals

- Architecture overview
- TAP format
- Capability ABI and WIT
- TaskRPC
- Durable event model
- Architecture decision records
- Contributing

Internal acceptance records never appear in public navigation or search.

## 7. Guide strategy

Reference pages describe surfaces. Guides complete jobs. The first learning paths are:

1. Build and package a JSON API
2. Connect Postgres without exposing credentials to JavaScript
3. Build an approval-based durable AI agent
4. Expose a TypeScript function as an MCP tool
5. Process a Queue message with retry and cancellation
6. Schedule a Cron task
7. Run untrusted plugin code with the Linux isolated profile
8. Sign, verify, and ship a release artifact
9. Deploy a Tysel executable with systemd
10. Package a Tysel service as a non-root container image

Every guide includes:

- Outcome and approximate completion time
- Supported release, operating system, and execution profile
- Prerequisites and complete runnable files
- Commands and expected output
- Permission explanation
- Verification, failure, and recovery path
- API reference and versioned example source

## 8. Page templates

### Concept

1. Definition
2. Why it exists
3. Mental model
4. Smallest useful example
5. Guarantees
6. Limits and non-goals
7. Stability and platform scope
8. Related guides and reference

### API and manifest reference

1. Signature or schema
2. Stability
3. Execution profile and platform
4. Required capability
5. Parameters and return value
6. Limits and defaults
7. Errors
8. Minimal and production examples

### CLI reference

1. Synopsis
2. Arguments and flags
3. Platform requirements
4. Output and side effects
5. Exit behavior and machine-readable output
6. Examples and related configuration

### Guide

1. Finished outcome
2. Supported release and time estimate
3. Prerequisites
4. Sequential steps
5. Verification
6. Troubleshooting and rollback
7. Next step

## 9. Documentation experience

- Search across docs, guides, examples, APIs, manifest fields, and CLI commands
- Keyboard-accessible command menu
- Copy button and filename label on every code block
- Accessible announcement when copy succeeds
- OS tabs only when commands differ
- Stable heading links and visible focus targets
- Previous/next navigation based on learning order
- “Edit this page” and source revision links
- Stability, profile, and platform badges
- Copy page as Markdown
- `/llms.txt`, `/llms-small.txt`, and Markdown for every public page
- Machine-readable manifest, CLI, evidence, and API schemas where available
- No documentation chatbot at launch; authoritative search comes first

Search prioritizes task pages for natural-language queries and exact reference pages for
commands, flags, APIs, and manifest tokens.

## 10. Visual system

Use the existing Tysel identity. Do not adopt a mascot, editorial illustration system, or
generic purple AI gradients.

### Brand idea

The identity is built around **compression and continuity**:

- Familiar TypeScript becomes one compact executable.
- Explicit boundaries constrain authority.
- Durable intent continues across suspension and restart.

### Character

- Infrastructure-native, compact, and precise
- Confident rather than decorative
- Monochrome-first with controlled Tysel Blue and Byte Lime
- Code and terminal output as primary visual material
- Motion that explains compression, boundaries, suspension, and continuation

### Brand anchors

| Role | Token | Value |
| --- | --- | --- |
| Primary dark | Binary Ink | `#111318` |
| Primary light | Runtime White | `#FFFFFF` |
| Brand accent | Tysel Blue | `#5B5CE2` |
| Completion/output | Byte Lime | `#C9FF63` |
| Light surface | Runtime Mist | `#F1F3F7` |

These are anchors, not a complete UI palette. Derive tested text, border, muted, warning,
error, focus, syntax, and interaction-state tokens before implementation.

### Layout

- Wide editorial homepage sections with one primary demonstration each
- Three-column docs layout on wide screens
- Prose width of approximately 70–78 characters
- Code wider than prose without clipping at common laptop widths
- Borders before shadows, small radii, restrained layering
- Independently designed light and dark themes
- The same information order on mobile, without hiding proof or limitations

### Graphic motif

```text
source → boundary → executable
event → checkpoint → continuation
```

Use the brand line field and wordmark logic. Use the full wordmark in navigation and the
`ty` mark only where the full name cannot fit.

## 11. Accessibility

Target WCAG 2.2 AA for the implemented site.

- Navigation, search, tabs, playback, copy, and theme controls work by keyboard with a
  visible focus indicator.
- Pages have one `h1`, ordered headings, landmarks, and a skip link.
- Text, controls, diagrams, syntax, and badges meet contrast targets in both themes.
- Color is never the only signal for permission, denial, stability, or completion.
- Terminal playback has pause, replay, step controls, and a static transcript.
- `prefers-reduced-motion` removes nonessential motion without removing explanation.
- State changes expose accessible names and restrained announcements.
- Code and diagrams remain operable at 200% zoom.
- Touch targets are at least 24 by 24 CSS pixels and larger for primary actions.
- Content remains readable when custom fonts are unavailable.

Acceptance requires keyboard, screen-reader, contrast, zoom, reduced-motion, and
responsive testing. A static design review alone cannot establish compliance.

## 12. Content source map

| Website area | Current source |
| --- | --- |
| Product overview | `README.md` |
| Existing entry and URL map | `docs/index.md`, `mkdocs.yml` |
| Installation and lifecycle | `docs/install.md`, CLI doctor and upgrade sources |
| Getting started | `docs/getting-started.md` |
| Projects and configuration | `docs/concepts/projects-and-configuration.md`, CLI project/init/task sources |
| CLI reference | `docs/reference/cli/`, `crates/tysel-cli/src/main.rs` |
| Manifest reference | `docs/reference/manifest/`, bundled manifest JSON Schema |
| Runtime APIs | `docs/reference/runtime/`, `packages/tysel-types/`, `runtime-js/`, `wit/` |
| Wasm Components | `docs/reference/component/`, `docs/guides/wasm-component-*.md`, `sdk/`, `wit/component/`, `wit/fs/` |
| Capability and security | `docs/capabilities/`, `docs/security/`, ADRs |
| npm compatibility | `docs/compatibility/`, compatibility source |
| Performance | `docs/performance/`, `benchmarks/` |
| Operations | `docs/operations/production.md` |
| Examples | `examples/` |
| Brand | `brand/README.md`, `brand/logo/` |
| Internal history | Completed planning and acceptance records; never public navigation |

Before visual implementation, commit the approved brand assets and reconcile references
to assets that do not exist in the repository.

## 13. Technical requirements

### Content and search

- Type-check MDX components and validate internal links in CI.
- Fail on duplicate slugs, missing required metadata, or public links to internal docs.
- Index headings, aliases, CLI flags, API names, and manifest keys.
- Generate `llms.txt`, Markdown pages, sitemap, RSS, and canonical URLs from the same graph.

### Performance

- Prefer server components and static generation for content.
- Isolate homepage interaction to small client components.
- Do not ship a browser terminal emulator when deterministic playback is sufficient.
- Reserve layout space for fonts, diagrams, code, and charts.
- Set budgets for JavaScript, fonts, images, and Core Web Vitals before acceptance.

### SEO and sharing

- Unique titles, descriptions, canonical URLs, Open Graph images, and structured metadata
  for product, guide, reference, example, benchmark, and release pages
- Social cards generated from the real wordmark and page metadata
- No reference to a social asset until it exists in the repository

### Analytics and privacy

Measure the evaluation path:

```text
Homepage → Get started → Install → Quick-start verification → Build guide
```

Track outbound GitHub/evidence links, zero-result searches, and guide completion signals.
Never capture source code, command input, secrets, or selected documentation content.

## 14. Internal implementation workstreams

This section is never published.

### Foundation

- Create the Next.js and Fumadocs application in the repository.
- Implement tokens, fonts, themes, navigation, search, metadata, and redirects.
- Migrate public Markdown into MDX without exposing internal planning documents.

### Credible evaluation path

- Homepage
- Install and HTTP quick start
- Durable-agent quick start
- Capability, security, compatibility, and limits pages
- Build-one-executable guide
- CLI, manifest, runtime, and durable reference
- Production evidence and operations entry points
- `llms.txt`, Markdown pages, sitemap, and social metadata

### Complete learning paths

- Service, task, agent, security, shipping, and operations guides
- Runnable examples
- Generated reference and schema validation
- Benchmark explorer with evidence downloads

### Ecosystem content

- Release notes and engineering blog
- Approved, evidence-backed adoption stories
- Framework and deployment-provider guides
- Community and contribution hub

## 15. Success criteria

- A new visitor understands the artifact, capability, and durable-work contracts in under
  one minute.
- A supported machine reaches a verified HTTP response in under five minutes using a named
  public release.
- The durable-agent guide demonstrates approval, restart recovery, and the exact boundary
  that prevents the saved result from repeating.
- Every capability states stability, profile, permission, limits, and failure behavior.
- Every homepage security or performance claim links to a contract or reproducible evidence.
- Any command, flag, API, or manifest option is reachable through one search or two
  navigation actions.
- Documentation works for developers and coding agents without duplicate prose sources.
- Keyboard, responsive, contrast, zoom, and reduced-motion acceptance passes for the
  homepage, search, docs shell, and primary quick starts.

## 16. Decisions

### Resolved

- Domain: `tysel.dev`
- One origin for marketing and documentation
- Stack: Next.js App Router, Fumadocs Core, MDX, custom visual components
- Primary story: “Write TypeScript. Ship a binary.”
- Existing lowercase wordmark and compression/continuity brand direction
- Implementation workstreams remain internal
- Managed three-binary developer toolchain; one-executable application delivery

### Open before public launch

1. Assign the public release version and channel from an actual tagged release.
2. Finalize the GitHub organization, repository URL, and release-asset location.
3. Decide when `tysel.dev/install.sh` becomes a supported authenticated entry point.
4. Publish the stability map for Durable and Component APIs.
5. Commit the complete brand assets, including social-card source and exports.
6. Decide whether Chinese follows the canonical English launch under `/zh/`.
7. Approve website performance budgets and analytics implementation.

## References

- [Bun homepage](https://bun.sh/)
- [Bun documentation](https://bun.sh/docs)
- [Deno homepage](https://deno.com/)
- [Deno runtime documentation](https://docs.deno.com/runtime/)
- [Deno security and permissions](https://docs.deno.com/runtime/fundamentals/security/)
- [Node.js permission model](https://nodejs.org/api/permissions.html)
