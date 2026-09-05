# Real-example workflow acceptance

Run the repository's hello service, isolated plugin, and durable agent through
permission edits, invalid configuration, source errors, recovery, typecheck,
build, and execution from a deployment directory without source or npm packages.
The durable artifact is restarted twice to check one LLM effect and one save.

Prerequisites: Python 3, Node.js, the installed workspace TypeScript compiler,
built `@tysel/types` declarations, and current local native tools:

```sh
cargo build --offline -p tysel-cli -p tysel-runtime -p tysel-isolate \
  --bin tysel --bin tysel-service --bin tysel-worker
pnpm --filter @tysel/types build
python3 tests/workflows/run.py --output /tmp/tysel-workflow-verification.json
```

The runner creates temporary projects with `tysel init`, then uses the actual
example source and manifests. It keeps the generated standalone tsconfig and
links local type declarations and TypeScript instead of fetching packages. It
uses ephemeral local ports and a fake local LLM endpoint with a test credential;
no paid provider is contacted. The original example data remains untouched.
Each server is stopped when its case finishes or fails. The printed fixture
location and JSON report are retained for inspection.

The isolated deployment case first verifies that omitting `tysel-worker` fails
clearly, then copies the matching worker beside the executable and verifies
network and filesystem denial. This is not a single-file isolation claim.

The acceptance run uses debug artifacts on the current host. It does not certify
release signing, cross-platform packaging, Linux security enforcement, or live
LLM provider compatibility. The editor extension test fixture remains separate
in `tests/editor`; no production editor extension is installed or developed here.
