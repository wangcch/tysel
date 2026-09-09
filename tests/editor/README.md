# Editor feedback smoke test

This is a test-only VS Code extension host fixture, not an installable Tysel
extension or LSP. It validates real editor providers against a generated project:

- Local TOML schema completion and validation, with online catalogs disabled.
- TypeScript capability completion and navigation to generated declarations.
- Live type updates when saved manifest permissions change.
- Invalid-manifest preservation and recovery.
- Mapping dev JSON snapshots into VS Code diagnostics, including Unicode ranges,
  canonical-path mapping for symlinked workspaces, replacement across files, stale
  generations, and clearing after repair.
- The generated schema is visible to Git while runtime state stays ignored.

## Run

Build the CLI and declarations first:

```sh
cargo build --offline -p tysel-cli
pnpm --filter @tysel/types build
```

Supply a VS Code executable and a locally installed Even Better TOML extension:

```sh
python3 tests/editor/run.py \
  --code '/Applications/Visual Studio Code.app/Contents/MacOS/Code' \
  --toml-extension "$HOME/.vscode/extensions/tamasfe.even-better-toml-0.21.2" \
  --output /tmp/tysel-editor-verification.json
```

The runner creates a temporary project, profile, and extension directory, then
launches a separate test window. It copies only the selected TOML extension and
uses VS Code's bundled TypeScript provider. It does not install dependencies,
change the user's editor settings, or modify another project. GUI and local
TCP/Unix socket access must be available. A 180-second timeout bounds the run.
Reports and editor logs stay in the printed temporary directory for inspection.
The JSON report also retains any TypeScript server errors seen in the editor log;
passing the assertions alone does not imply the editor log was error-free.

## Interpretation

The diagnostic adapter lives only in `test.cjs`. It proves the saved-file event
contract can feed the editor API, including error navigation; it is not a shipped
Problems integration. It serializes event application and clears previous-file
errors when a newer snapshot arrives. Runtime occurrence events are separate.

The suite intentionally uses VS Code's bundled TypeScript provider and symlinks
the built local `@tysel/types` declarations. It does not prove npm publication,
TypeScript 7 native-extension support, unsaved-buffer Tysel analysis, multi-root
workspace routing, process restart handling, or every JSON Schema keyword. A
production adapter still needs those lifecycle and UX decisions, including
how to avoid duplicate TOML/Tysel manifest errors and how to label stale errors
when unsaved contents diverge from the snapshot on disk.
