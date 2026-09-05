# Editor feedback iteration

Status: Implemented and verified (2026-09-05)

## Objective

Keep generated runtime types synchronized during development and make Tysel's
own static errors consumable by editors. Reuse TypeScript language services;
an independent LSP is outside this iteration.

## Delivery order

1. Automatically synchronize `tysel-env.d.ts` at dev startup and reload after
   successful manifest validation. Do not rewrite identical output, overwrite
   user-owned files, follow output symlinks, or trigger a reload from generated
   declaration changes. Invalid manifests preserve the last valid declaration.
   `tysel types --check` remains read-only for CI.
2. Reuse the structured diagnostic envelope for manifest and unsupported import
   errors. Preserve stable codes and original-source locations when available;
   do not invent a location for an absent field. Keep CLI and dev consistent.
3. Diagnose unsupported runtime imports throughout the reachable module graph
   using the build resolver, including imports in non-entry files. Preserve
   TypeScript source positions and exclude erased type-only imports.
4. Associate new TOML manifests with the existing JSON schema and document editor
   setup and its limits. Verify schema/tool compatibility before recommending it.

## Verification

- Targeted type-generation tests cover changed/unchanged output, invalid
  manifests, user files, and symlinks; watcher tests cover generated output.
- Diagnostic tests cover stable codes, nested imports, type-only imports,
  missing fields, and Unicode source positions.
- Exercise CLI JSON failures and a live dev edit/error/recovery cycle.
- Run formatting and relevant crate tests; document any unavailable checks.

## Verification results

- Relevant unit suites: 147 passed (manifest 24, build 39, supply-chain tool 6,
  CLI 78). CLI development integration suite: 66 passed.
- `cargo clippy --offline -p tysel-manifest -p tysel-build -p tysel-cli --lib
  --bins -- -D warnings`, formatting, and diff whitespace checks passed.
- Strict MkDocs build passed with output outside the repository.
- Taplo CLI 0.9.0 accepted a valid manifest through the local schema directive
  and rejected invalid workers, an unknown profile, and a Component/entry
  mismatch. This validates schema consumption, not a visual editor UI audit.
- Runtime inventory regenerated and verified (497 production components) after
  the direct TOML span dependency changed Cargo.lock. Regeneration also included
  existing interactive-init dependencies missing from the previous inventory.
- Socket-based tests passed outside the restricted sandbox. The initial failures
  were denied TCP/Unix socket operations; the new recovery test also had one
  expected-line assertion corrected before the final successful run.

## Delivered behavior and limits

- Dev synchronizes the default output after manifest validation, before source
  bundling. Source errors can therefore coexist with newly updated types; an
  invalid manifest preserves the previous types. Custom output paths stay manual.
- Manifest semantic diagnostics locate TOML and JSON values from the original
  source snapshot. Parse errors keep the parser's position. Absent values have
  no invented range. Validation still reports the first failure.
- The existing bundler already traversed the module graph. This iteration adds
  structured resolution failures and original-source ranges, removes redundant
  entry scanning in check, and excludes erased type-only requests.
- New TOML projects carry a local schema with no online dependency. Existing
  projects and JSON editor associations have documented setup instructions.

## Follow-up decision

Keep the lightweight editor adapter as a test fixture. Productization is paused
until real usage demonstrates a need. Consider a thin LSP only when unsaved-buffer
analysis, cross-file configuration navigation, or multiple editor clients justify it. TypeScript inference and unrestricted
permission-changing quick fixes remain outside the scope.


## Editor-host follow-up (2026-09-05)

The real VS Code extension-host smoke test passed all seven scenarios on VS Code
1.136.1, its bundled TypeScript 6.0.3 provider, and Even Better TOML 0.21.2. The
final run recorded no TypeScript server errors. The reproducible runner is
`tests/editor/run.py`; evidence is saved in
`tests/editor/evidence/vscode-2026-09-05.json`.

Verified through editor APIs: offline schema completion, generated-type
navigation, live capability completion changes, invalid-manifest preservation,
repair and revocation, Unicode error navigation, cross-file diagnostic
replacement, stale-generation rejection, and final clearing. The adapter is a
test fixture, not a shipped extension. It maps canonical diagnostic paths back
to workspace URIs, including macOS /var versus /private/var aliases.

This validation found and fixed a delivery bug: the generated `.tysel/` ignore
rule hid the shared schema from Git. Init now adds an ordered exception for
`.tysel/manifest.schema.json`, preserving ignores for other runtime state. The
merge logic keeps the exception effective even with existing, conflicting ignore
rules. A Git-backed regression test covers new and adopted projects. All 21 init
integration tests and the gitignore merge unit test passed, as did CLI Clippy,
formatting, whitespace checks, and the strict documentation build.

An earlier run exposed a TypeScript 6.0.3 refactor request error while rapidly
removing the currently displayed module from the import graph. Returning to the
entry editor before changing imports produced a clean final run; this does not
claim to fix the TypeScript server edge case. The runner records such errors in
its evidence rather than hiding them behind passing assertions.

Decision: the current APIs are sufficient for a saved-file editor adapter. Before
shipping it, specify process restart/reset behavior, dirty-buffer diagnostics,
multi-root routing, and deduplication with TOML validation. TypeScript 7 native
extension support and npm publication remain unverified by this smoke test.
An independent LSP is still unnecessary for the validated scenarios.


## Real-project acceptance and review (2026-09-05)

All three repository examples completed the development-to-artifact workflow:

| Example | Verified behavior |
| --- | --- |
| `hello-service` | Permission/type update, invalid manifest and import recovery, pinned TS 7.0.2 check, native build, HTTP response without source or npm dependencies. |
| `isolated-plugin` | The same development checks, plus denied fetch/filesystem probes in dev and in the packaged deployment with the matching worker. |
| `durable-agent` | The same development checks, plus two native-artifact restarts, retained approval state, exactly one local LLM effect, and one final save. |

The runner initializes a disposable project, adopts the actual example manifest
and source, keeps the standalone generated tsconfig, and links local workspace
types and the pinned compiler. Deployment directories contain no source,
manifest, or node_modules. These are macOS arm64 debug-artifact checks with a
local fake LLM, not a release, external-provider, or Linux isolation certification.
See `tests/workflows/README.md` and the evidence in
`tests/workflows/evidence/macos-2026-09-05.json`.

Review covered generated-file ownership and symlinks, unchanged-file writes,
invalid-manifest preservation, type-only imports, original-source ranges,
JSON/TOML field location, diagnostic envelope compatibility, Git rule ordering,
and whether the test-only editor adapter is kept separate from runtime code.
No blocking regression was found in the tested routes. Final relevant tests:
147 unit tests plus 67 CLI integration tests passed (214 total); Clippy,
formatting, diff whitespace, strict documentation build, and the 497-component
runtime inventory check passed.

### Findings ordered by practical impact

1. **P1 — isolated deployment requires a companion worker.** Confirmed by running
   the packaged app without one: startup fails with `worker binary not found`.
   Copying the matching worker restores operation and preserves capability denial.
   Build output, README, example instructions, and deployment documentation now
   state the requirement. Automatic worker packaging/signing remains a separate
   product decision; this iteration does not promise a single-file isolated app.
2. **P2 — generated schema was ignored by Git.** Fixed and covered by Git-backed
   tests for new projects and conflicting existing rules. Other runtime state
   stays ignored.
3. **P2 — canonical paths differ from editor workspace URIs.** The test adapter
   now maps them; preserve this behavior if a production adapter is later built.

There is no planned extension product work in this iteration. TS 7 native editor
support, dirty-buffer analysis, multi-root behavior, and release/Linux gates are
explicit verification gaps, not failures inferred from the passing local tests.

### Change organization

- Development feedback foundation: type synchronization, static diagnostics,
  schema generation and Git handling, plus focused Rust regressions.
- Acceptance fixtures: separate editor-host and real-project workflow tests with
  reproducible commands and bounded evidence.
- Delivery documentation: accurate isolated-worker requirements, validation
  conclusions, and the decision to pause editor productization.
