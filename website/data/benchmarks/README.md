# Benchmark evidence snapshots

`admission-linux-x64.json` feeds `/benchmarks`.

- Checked-in default: `{ "status": "unpublished" }` — measured cells stay empty.
- Published shape: `{ "status": "published", "provenance": {...}, "evidence": <BenchmarkEvidence> }`.
- Local preview only: copy `admission-linux-x64.sample.json` over the default (keep `provenance.sample: true`).
- Production refresh: website CI / `pnpm sync:benchmarks` downloads a fresh artifact from the canonical successful **main-branch push** CI workflow. It binds the evidence commit to the workflow head SHA and validates all seven gates.
- Public raw envelope: `/benchmark-evidence/latest.json` is generated from the same build-time import as `/benchmarks`.

If fetch finds nothing, is older than 30 days, has mismatched provenance, or fails the complete seven-gate schema checks, the script writes `unpublished` and exits 0 — it never keeps a stale published snapshot from the checkout.

Do not hand-edit measured values. Numbers must come from a CI evidence artifact.
