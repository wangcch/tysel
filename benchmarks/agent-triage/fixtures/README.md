# Agent benchmark regression fixtures

These 12 files are the minimal historical inputs used by the offline benchmark
regression tests. They are copied byte-for-byte from the original measurement
archives. CI does not require the full dated archive or any local build output.
Do not overwrite these inputs with a new benchmark run: add or intentionally
replace a fixture only when its regression contract changes.

## Coverage and provenance

All source paths below are relative to the original
`docs/agent-execution-evidence/` archive. They identify provenance; the complete
archive is kept outside the submitted test fixtures.

| Fixture | Original source | Regression purpose |
| --- | --- | --- |
| `cost/p5/{measurements.json.gz,summary.json,results.md}` | `2026-09-24-p5/` with the same filenames | Recompute all 132 numerical checks and reproduce the **14 real budget misses** on the original Podman host bind mount. |
| `cost/p51/{measurements.json.gz,summary.json,results.md}` | `2026-09-24-p51/` with the same filenames | Reproduce the **15 real budget misses** on the same bind-mount target after the experimental SQL reduction. That candidate was subsequently withdrawn. |
| `cost/p52/{measurements.json.gz,summary.json,results.md}` | `2026-09-24-p52/` with the same filenames | Reproduce the **132/132 passing checks** for the explicitly different Podman local named-volume target; provide valid input for corruption rejection tests. |
| `dispatch/o3-measurements.json.gz` | `2026-09-28-o3/measurements.json.gz` | Supply historical audit/read shapes for synthetic dispatcher A/B evidence in `test_dispatch_ab.py`. |
| `dispatch/o4-measurements.json.gz` | `2026-09-28-o4/measurements.json.gz` | Supply historical lookup shapes for synthetic capacity/CPU-accounting evidence in `test_capacity.py`. |
| `dispatch/o4-predeclared.json` | `2026-09-28-o4/predeclared.json` | Freeze the dispatcher experiment schedule and candidate-selection boundaries. |

`test_report.py` recomputes the cost summaries and checks their exact rendered
JSON/Markdown bytes. A `budget_miss` fixture is a valid, complete historical
measurement, not a passing cost result and not an incomplete driver failure.
The P5/P5.1 failures remain failures; the P5.2 named-volume result does not erase
or relabel them. Storage scope is part of each result.

Dispatcher and capacity tests deliberately replace performance values and
identities with synthetic values. Their passing results validate the checkers,
not new throughput, latency, CPU or production-capacity claims. These fixtures
alone are not a complete performance archive or release-admission record.

The frozen O4 plan's `volumeProtocol` field records the historical location.
The live dispatcher driver loads `../protocol-volume.json` relative to this
fixture directory and verifies the same `volumeProtocolSha256`; the plan bytes
and selection criteria remain unchanged.

## File integrity

The SHA-256 values below hash the exact stored files, including the compressed
bytes for `.gz` files (not the decompressed JSON). The original JSON retains its
historical source, package and protocol identities; moving the files does not
retarget those identities to the current checkout.

```text
dddb5da73ea2ea382dba0fda4a2d982ad1a31192b00dcd2fd4d0d80195ddd324  cost/p5/measurements.json.gz
81332576c037e8e53e47f19098f0cb118abe55cf0d30151edae0d3907954d5f8  cost/p5/results.md
b26a3dbb528614cc39b4ed0a99dc9991745e74f64f2d84119d9ab5ddbfee7cfc  cost/p5/summary.json
f232ea74e447ad1f8df0f3e899e6af13820632030c788c837202c93a877db0d2  cost/p51/measurements.json.gz
a834330ddfd5a6d000c9d2d37c4240b12ac7d0be25f2fbec0218d89efeccab34  cost/p51/results.md
7885e24231d478a9e9b7515a9e3743859f946db2c0b397f9495431e2052ad178  cost/p51/summary.json
72644fbfcf650f6060016c16f92a1aa7d3dc53c0d6dfbc00590d9346c80f9f26  cost/p52/measurements.json.gz
01784861ecd2bb4359723581fc91e0d5839925405425ef3fbc3dd8cef0bd9e61  cost/p52/results.md
1e3cb1f8a046763ba9201a48336f313cb863df6b7668c13aeefdea3cb839a37d  cost/p52/summary.json
585382b9c9d0f8d9f48d60878017b826339c49ac83c02942914c7115b22728ad  dispatch/o3-measurements.json.gz
b7e943f35f33f2a33d6268fa6b8ca75e7360fa45fe15f335f22a74386cd8e931  dispatch/o4-measurements.json.gz
3cbebc7ab7985f5c4e4a61c547b82eb50fff9e7a5dac9f971cd2e9291fdb38b8  dispatch/o4-predeclared.json
```

Run the consumers from the repository root:

```sh
python3 benchmarks/agent-triage/test_report.py
python3 benchmarks/agent-triage/test_dispatch_ab.py
python3 benchmarks/agent-triage/test_capacity.py
```

The same offline checks also run with `PYTHONOPTIMIZE=1`; they use explicit
validation and unittest assertions. This does not authorize running measurement
drivers with Python optimization enabled.
