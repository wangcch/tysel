# tysel.dev website plan

Status: content and information-architecture draft
Domain: `tysel.dev`
Primary audience: TypeScript backend developers, agent builders, and platform engineers

## 1. Product story

Tysel should not present itself as another general-purpose JavaScript toolchain. Its
strongest story is narrower and more defensible:

> Write TypeScript. Ship a binary.

> A lightweight native runtime for TypeScript services and AI agents.

The website should make three promises, in this order:

1. **Simple delivery** — turn a TypeScript service or agent into one native executable.
2. **Bounded execution** — capabilities are explicit, secrets stay opaque, and isolated
   workloads have a smaller authority surface.
3. **Durable agents** — LLM calls, retries, signals, human approval, and restart recovery
   use one durable task model.

The first visit should answer four questions quickly:

- What is Tysel?
- Why would I use it instead of Node.js, Bun, or Deno?
- Can I run a useful example in a few minutes?
- Is it credible enough to evaluate for production?

### Core transformation

The deeper positioning is not “another faster JavaScript runtime.” Tysel changes three
developer contracts:

| Before | With Tysel | Meaning |
| --- | --- | --- |
| Deploy an environment | Copy and run one executable | Delivery becomes an artifact problem |
| Trust application code | Grant explicit capabilities | Trust moves from code to enforced boundaries |
| Keep a worker alive | Suspend and resume durable work | Long-running intent does not require resident execution |

Chinese manifesto line:

> Tysel 让 TypeScript 从一种需要部署运行环境的语言，变成一种可以直接交付为可执行文件的语言。

Public English form:

> Tysel makes TypeScript directly shippable—as a single, capability-bounded executable.

This is the narrative spine for the homepage, launch post, and project README. Use the
shorter “Write TypeScript. Ship a binary.” as the memorable expression of the same idea.

### Message hierarchy

1. **Category statement:** A native runtime for TypeScript services and AI agents.
2. **Transformation:** From deploying an environment to shipping one executable.
3. **Trust model:** Treat code as untrusted; trust explicit boundaries.
4. **Execution model:** Durable work can suspend, release resources, and resume.
5. **Proof:** Reproducible artifact, startup, memory, isolation, and recovery evidence.

The AI-era framing belongs after the product is understood. Lead with a concrete
delivery model, then explain why cheap isolation and durable execution matter for
generated code at scale.

## 2. What to borrow from Bun and Deno

### From Bun

- Put a concrete install or quick-start command in the hero.
- Show the product through real commands and real code, not feature adjectives.
- Organize product capabilities into understandable groups.
- Put benchmark methodology next to benchmark claims.
- Keep guides and reference material distinct.

### From Deno

- Provide task-oriented guides, conceptual documentation, and precise reference pages.
- Make security and permissions a first-class concept rather than an appendix.
- Support AI-assisted documentation with `llms.txt` and a Markdown representation of
  every page.
- Give migration and compatibility questions their own clear entry point.
- Let users navigate by intent: get started, build, deploy, diagnose, and reference.

### What Tysel should not copy

- Bun's all-in-one toolchain story: Tysel does not replace a package manager, formatter,
  linter, and the entire JavaScript workflow.
- Deno's ecosystem breadth: the first release should not look larger than the available
  runtime and documentation.
- Unqualified speed claims: publish only reproducible measurements already backed by the
  repository's benchmark harnesses.
- Customer-logo walls before there are public, approved production users.

## 3. Site model

### Existing baseline

The repository already contains a small MkDocs site with a Read the Docs theme,
top-level navigation, getting-started content, CLI documentation, and a runtime API
page. Treat this as the content bootstrap and link-validation baseline. Preserve its
working URLs during the first implementation pass, but do not let the current theme or
flat navigation determine the final product experience.

Use one domain and one coherent navigation system:

```text
tysel.dev/                 Product homepage
tysel.dev/docs/            Documentation home
tysel.dev/guides/          Task-oriented tutorials
tysel.dev/examples/        Runnable examples
tysel.dev/reference/       CLI and API reference
tysel.dev/benchmarks/      Reproducible performance evidence
tysel.dev/blog/            Releases and engineering notes
```

Recommended global navigation:

```text
Tysel | Docs | Guides | Examples | Benchmarks | Blog | GitHub
```

Global utilities:

- Search / command menu (`⌘K` or `Ctrl K`)
- Version selector
- Theme selector
- GitHub link and star count, when a public repository is ready
- Persistent primary CTA: **Get started**

Do not split marketing and documentation across separate subdomains at launch. A single
origin makes search, analytics, linking, and versioning easier and helps a small project
feel like one product.

## 4. Homepage content

### 4.1 Announcement bar

Reserve for the latest stable release or a major capability. Avoid a generic welcome
message.

Example:

> Tysel Production v1 — durable agents, bounded capabilities, and reproducible binaries.

### 4.2 Hero

Primary copy:

> **Write TypeScript. Ship a binary.**
>
> Tysel turns TypeScript services and AI agents into a single, capability-bounded
> executable—with durable work built in.

Primary CTA: **Get started**
Secondary CTA: **View on GitHub**

The interactive object should demonstrate the whole proposition in one compact flow:

1. A small `fetch` handler in `src/index.ts`
2. `tysel build`
3. A single output artifact

Implement this as a deterministic, user-controlled playback generated from captured
output of a named Tysel release. It should not execute a build in the visitor's browser.
Show the version and platform beside the result, and provide replay and step controls.

The hero must not imply ahead-of-time compilation to native machine code. Use “single
native executable” or “one executable,” matching the product contract.

The sentence about making TypeScript “directly shippable” can appear immediately below
the first demonstration, where there is enough room to explain what is embedded in the
artifact.

### 4.3 Immediate proof strip

Use short, factual statements:

- Web API first
- No Node.js, V8, or `node_modules` in production
- Explicit capabilities
- Durable task replay

Each statement should link to an explanatory documentation page.

Do not put unverified point measurements in this strip. Artifact size, cold-start time,
and idle memory must come from release evidence for a named version, platform, build
profile, workload, and commit.

### 4.4 Three product pillars

#### Ship one executable

Show `tysel dev`, `tysel check`, and `tysel build` as one short development-to-delivery
path. Link to the service quick start and build guide.

#### Bound every capability

Show a minimal `tysel.toml` permission block beside a service call. Explain deny-by-
default access, opaque secret handles, and the isolated profile without turning the
homepage into a threat-model document.

The public contrast is “trust boundaries, not application code.” Avoid saying competing
runtimes have no permissions or isolation. The defensible distinction is Tysel's layered
Linux isolated profile, its supervisor-held secrets, its restricted capability surface,
and its integration with the single-artifact delivery model.

#### Build agents that survive restarts

Show the durable-agent golden path: LLM call → persisted effect → human approval →
restart → resume exactly once. Link to a runnable example and the durable execution
concept page.

Describe durability as a **runtime primitive** or **programming-model capability**, not
a new TypeScript language feature. The developer experience may look like ordinary
`async`/`await`, while the documentation must still explain deterministic boundaries and
side-effect rules.

### 4.5 “Five minutes with Tysel”

Use a stepper with real commands:

1. Create a service: `tysel init my-service`
2. Develop with reload: `tysel dev`
3. Validate permissions and types: `tysel check`
4. Run tests: `tysel test`
5. Build one executable: `tysel build --release`

Each step should show expected output and link to a focused guide.

### 4.6 Workload selector

Use three tabs rather than a long feature grid:

- **HTTP service** — Request/Response, fetch, WebSocket, SQLite/Postgres
- **AI agent** — native LLM gateway, durable effects, signals, approvals
- **Task worker** — Cron, Queue, MCP tools, TaskRPC scheduling

This becomes the bridge from the homepage into the documentation.

### 4.7 Production evidence

Show four evidence categories with direct links:

- Reproducible release artifacts and SBOMs
- Offline signing and verification
- Security and isolation boundaries
- Production operations runbook

Benchmarks belong below product comprehension. Every chart must identify the version,
hardware, workload, sample method, and reproduction command.

### 4.8 Final CTA

> From TypeScript source to one production artifact.

Actions: **Build your first service** and **Run the durable agent example**.

## 5. Documentation information architecture

The left navigation should use progressive disclosure. Keep the first level short and
move exhaustive lists into reference pages.

### Get started

- Welcome to Tysel
- Install
- Quick start: HTTP service
- Quick start: durable agent
- How Tysel works
- Project structure

### Build services

- Request and Response handlers
- Outbound fetch
- WebSockets
- Timers, streams, encoding, and crypto
- SQLite
- Postgres
- Filesystem access

### Build agents and tasks

- Agent overview
- LLM generation gateway
- Durable execution
- Steps and effects
- Sleep, retry, and deterministic time
- Signals and human approval
- Cron handlers
- Queue handlers
- MCP tools
- Task lifecycle and cancellation

### Capabilities and security

- Capability model
- Manifest permissions
- Secrets
- Service and isolated profiles
- Linux isolation boundary
- Resource limits
- Threat model

### Develop and test

- `tysel.toml`
- Development server and reload
- Type checking and capability scanning
- Test runner
- npm compatibility
- Debugging
- Observability and local logs

### Build and ship

- Build a single executable
- Build targets and profiles
- Container images
- Release evidence and SBOMs
- Sign and verify artifacts
- Reproducible builds
- CI guide

### Operate

- Production checklist
- Configuration and secrets
- Durable Postgres
- Backup and restore
- Capacity planning
- OpenTelemetry
- Upgrade and rollback
- Monitoring and alerts
- Incident response

### Reference

- CLI
- Manifest schema
- Tysel runtime API
- Supported Web APIs
- Environment variables
- Capability matrix
- Compatibility catalog
- Limits
- Error codes
- Release compatibility guarantees

### Internals

- Architecture overview
- TAP format
- Capability ABI
- TaskRPC
- Durable event model
- Architecture decision records
- Contributing

## 6. Guide strategy

Reference documentation describes every surface. Guides should solve complete jobs.
Launch with these guides:

1. Build and package a JSON API
2. Connect a service to Postgres without exposing credentials to JavaScript
3. Build an approval-based durable AI agent
4. Expose a TypeScript function as an MCP tool
5. Process queue messages with retries and cancellation
6. Schedule a Cron task
7. Run untrusted plugin code with the isolated profile
8. Sign, verify, and ship a release artifact
9. Deploy a Tysel executable with systemd
10. Package a Tysel service as a container image

Every guide should contain:

- Outcome and prerequisites
- Complete runnable files
- Commands and expected output
- Permission explanation
- Failure and recovery path
- Link to the relevant API reference
- Link to the example directory and source revision

## 7. Page templates

### Concept page

Use: durable execution, capability security, TaskRPC.

1. One-sentence definition
2. Why it exists
3. Mental model
4. Smallest useful example
5. Guarantees
6. Limits and non-goals
7. Related guides and reference

### API/reference page

1. Signature or schema
2. Availability by profile
3. Required permission
4. Parameters and return value
5. Limits
6. Errors
7. Minimal example
8. Production example

### CLI page

1. Synopsis
2. Arguments and flags
3. Exit behavior
4. Machine-readable output
5. Examples
6. Related configuration

### Guide page

1. Visible finished outcome
2. Time estimate
3. Prerequisites
4. Sequential steps
5. Verification
6. Troubleshooting
7. Next step

## 8. Visual direction

Build on the existing Tysel brand instead of adopting Bun's mascot-led personality or
Deno's illustrated editorial style.

### Character

- Infrastructure-native, compact, and precise
- Monochrome-first with controlled flashes of Tysel Blue and Byte Lime
- Code and terminal output are the main visual material
- Motion should communicate source compression, isolation boundaries, and durable
  continuation

### Existing palette

| Role | Token | Value |
| --- | --- | --- |
| Primary dark | Binary Ink | `#111318` |
| Primary light | Runtime White | `#FFFFFF` |
| Brand accent | Tysel Blue | `#5B5CE2` |
| Success/output | Byte Lime | `#C9FF63` |
| Light surface | Runtime Mist | `#F1F3F7` |

### Layout principles

- Homepage: wide editorial sections with one strong demonstration per section
- Docs: three-column desktop layout — navigation, article, table of contents
- Reading width: approximately 70–78 characters for prose
- Code blocks: slightly wider than prose and never horizontally clipped on common laptop
  widths
- Borders before shadows; small radii; restrained layering
- Dark mode is a first-class theme, not a simple color inversion

### Graphic motif

Use a “source → boundary → binary” flow derived from the existing line-field brand
asset. The boundary can become a recurring visual language for permissions, isolates,
and durable checkpoints. Do not introduce an unrelated mascot.

## 9. Documentation UX

- Full-text search across docs, guides, examples, and CLI commands
- Copy button on every code block
- File-name labels on multi-file examples
- OS tabs only when commands actually differ
- Stable permalinks for headings
- Previous/next navigation based on learning order
- “Edit this page” and source revision links
- Version and stability badges: Stable, Experimental, Platform-specific
- Per-page profile badges: Service, Isolated, Linux production gate
- Copy page as Markdown
- `/llms.txt` plus a concise `/llms-small.txt`
- Machine-readable manifest and API schemas when available
- No chat widget in the MVP; high-quality searchable pages are the priority

## 10. Content source map

Existing repository material can seed the site:

| Website section | Current source |
| --- | --- |
| Product overview | `README.md` |
| Current docs entry and navigation | `docs/index.md`, `mkdocs.yml` |
| Current onboarding | `docs/getting-started.md` |
| Current CLI reference | `docs/cli.md` |
| Current runtime API reference | `docs/api/runtime.md` |
| Brand and visual system | `brand/README.md`, `brand/logo/` |
| Engineering design history (internal only) | `roadmap.md`, acceptance records |
| Capability model | `docs/capabilities/README.md` |
| npm compatibility | `docs/compatibility/README.md` |
| Architecture | `docs/architecture/`, `docs/adr/` |
| Production operations | `docs/operations/production.md` |
| Durable agent guide | `examples/durable-agent/README.md` |
| Benchmark evidence | `benchmarks/`, `docs/performance/README.md` |
| CLI reference | `crates/tysel-cli/src/main.rs` |
| Manifest reference | manifest crate types and example `tysel.toml` files |
| Runtime APIs | `packages/tysel-types/`, `runtime-js/`, `wit/` |

Generated reference pages should be produced from source types or schemas where
possible. Narrative guides should remain hand-authored.

## 11. Publication plan (internal)

This section plans the documentation project and must not be published as product
documentation. Public pages describe current released behavior and use stability labels
instead of project phases.

### Launch — credible evaluation

- Homepage
- Documentation shell and search
- Install and HTTP quick start
- Durable agent quick start
- Testing guide and `tysel test` reference
- Capability and security overview
- Build a single executable
- CLI and manifest reference
- Compatibility and limits pages
- Existing production runbook migrated into readable sections
- `llms.txt`, sitemap, metadata, and social cards

### Next — complete product learning paths

- All service, task, agent, and capability guides
- Runnable example gallery
- Versioned documentation
- Benchmark explorer with reproducible methodology
- Generated API and configuration reference

### Later — ecosystem and trust

- Release notes and engineering blog
- Public adoption stories
- Framework integration guides
- Deployment provider guides
- Community and contribution hub

## 12. Success criteria

- A new user can understand Tysel's unique value in under one minute.
- The HTTP quick start can be completed in under five minutes from a supported machine.
- The durable-agent guide demonstrates restart recovery and exactly-once saved output.
- Every public capability states its profile, required permission, limits, and failure
  behavior.
- Every homepage performance or security claim links to reproducible evidence or an
  explicit design contract.
- A developer can find any CLI command or manifest option in two navigation actions or
  one search.
- Documentation is useful to both humans and coding agents without maintaining separate
  prose sources.

## 13. Claim and evidence policy

The manifesto may be bold; product claims must be reproducible.

### Safe launch claims

- Produces one native executable.
- Production does not require Node.js, V8, or `node_modules`.
- Capabilities are deny-by-default and explicitly granted.
- The Linux isolated profile adds process separation, Landlock, seccomp, and best-effort
  cgroup memory enforcement.
- Durable tasks persist suspension, signals, retries, and replayable effects.

### Claims requiring release evidence

- Exact executable size such as `3.87 MB`
- Exact cold-start result such as `8 ms`
- Exact idle memory result such as `8 MB`
- Comparisons with Docker, Node.js, Bun, Deno, or another sandbox
- Scale claims such as “run 1,000 isolated functions”

Publish these only as a benchmark card with:

- Tysel version and source commit
- OS, architecture, CPU, and build profile
- Workload and artifact contents
- Metric definition, raw samples, and aggregation
- Competitor versions and equivalent configuration when applicable
- Reproduction command and evidence download

The current repository contract uses release gates of at most 20 MiB artifact size,
15 ms median cold start, and 32 MiB idle Linux PSS. These are admission thresholds, not
the measured `3.87 MB / 8 ms / 8 MB` results in the manifesto. Keep the sharper figures
out of production copy until matching evidence is checked in.

### Wording to avoid

- “The only runtime that…” — difficult to maintain and unnecessary.
- “Node/Bun/Deno have no isolation” — collapses different permission and sandbox models
  into an inaccurate absolute.
- “Process-level security” without platform scope — Linux is the production isolation
  gate; macOS is not.
- “Persistence is a language feature” — Tysel provides a runtime programming model, not
  a change to the TypeScript language.

### Recommended competitive frame

> Bun and Deno optimize the general JavaScript runtime and toolchain. Tysel is designed
> around a narrower production contract: small deployable artifacts, capability-bounded
> execution, and durable TypeScript services and agents.

## 14. Open decisions before visual implementation

1. Confirm the initial installation and binary distribution method.
2. Confirm the public release label and documentation versioning policy.
3. Confirm the public GitHub repository URL and release artifact locations.
4. Decide whether Chinese is launch scope. Recommendation: launch canonical English
   content first, then add `/zh/` without mixing languages inside a page.
5. Choose the first visual mock target: homepage desktop plus mobile, followed by the
   docs article template.

## References

- [Bun homepage](https://bun.sh/)
- [Bun documentation](https://bun.sh/docs)
- [Deno homepage](https://deno.com/)
- [Deno runtime documentation](https://docs.deno.com/runtime/)
- [Deno security and permissions](https://docs.deno.com/runtime/fundamentals/security/)
- [Node.js permission model](https://nodejs.org/api/permissions.html)
