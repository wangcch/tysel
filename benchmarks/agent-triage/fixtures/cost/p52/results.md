# P5 cost results

Status: **passed**. Linux ARM64 release on the named local VM.

These are descriptive local measurements, not production SLOs or a cross-runtime ranking.

| Variant / size / clients / adapter delay | p50 ms | p95 ms | Jobs/s, range of 3 rounds | Sampled peak PSS MiB | Budget |
| --- | ---: | ---: | ---: | ---: | --- |
| direct/small/c1/delay0 | 0.71 | 0.95 | 1167.04–1514.39 | 16.76 | pass |
| direct/small/c4/delay0 | 1.65 | 2.80 | 1640.58–2680.68 | 9.91 | pass |
| direct/bounded/c1/delay0 | 0.74 | 0.95 | 1092.26–1498.30 | 16.76 | pass |
| direct/bounded/c4/delay0 | 1.65 | 4.88 | 984.63–2493.99 | 9.95 | pass |
| snapshot/small/c1/delay0 | 2.01 | 3.97 | 375.44–995.33 | 12.89 | pass |
| snapshot/small/c4/delay0 | 1.83 | 3.26 | 1730.02–1957.81 | 13.16 | pass |
| snapshot/bounded/c1/delay0 | 0.95 | 2.42 | 535.02–1076.80 | 12.95 | pass |
| snapshot/bounded/c4/delay0 | 2.04 | 3.65 | 1526.06–1947.57 | 13.21 | pass |
| lookup/small/c1/delay0 | 254.93 | 305.27 | 3.88–3.94 | 43.11 | pass |
| lookup/small/c1/delay20 | 252.47 | 301.87 | 3.90–3.96 | 42.61 | pass |
| lookup/small/c4/delay0 | 250.54 | 421.63 | 12.68–15.13 | 59.50 | pass |
| lookup/small/c4/delay20 | 255.69 | 339.30 | 13.70–15.48 | 94.70 | pass |
| lookup/bounded/c1/delay0 | 253.26 | 288.16 | 3.92–3.94 | 42.02 | pass |
| lookup/bounded/c1/delay20 | 254.30 | 299.20 | 3.91–3.95 | 43.84 | pass |
| lookup/bounded/c4/delay0 | 266.75 | 403.58 | 13.80–14.82 | 58.47 | pass |
| lookup/bounded/c4/delay20 | 271.49 | 383.27 | 11.05–15.22 | 94.73 | pass |

Cold process-to-first-result (20 samples each, warm filesystem caches):

| Variant | p50 ms | p95 ms |
| --- | ---: | ---: |
| direct | 14.14 | 19.48 |
| snapshot | 24.63 | 30.87 |
| lookup | 399.26 | 450.60 |

Lookup stages below are **p50 / p95 milliseconds** from transactional audit timestamps,
with millisecond resolution. Stage percentiles do not sum to end-to-end percentiles.

| Cell | Queue → select | Select | Read | Summarize | Between stages | Terminal commit | Actual adapter sleep | E2E excluding sleep |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| lookup/small/c1/delay0 | 207.00 / 225.00 | 8.00 / 13.00 | 5.00 / 8.00 | 5.00 / 8.00 | 17.00 / 31.00 | 5.00 / 8.00 | 0.00 / 0.00 | 254.93 / 305.27 |
| lookup/small/c1/delay20 | 185.00 / 198.00 | 8.00 / 12.00 | 25.00 / 28.00 | 5.00 / 7.00 | 18.00 / 39.00 | 5.00 / 11.00 | 20.17 / 20.35 | 232.15 / 281.53 |
| lookup/small/c4/delay0 | 136.00 / 244.00 | 15.00 / 23.00 | 11.00 / 20.00 | 11.00 / 22.00 | 52.00 / 94.00 | 15.00 / 37.00 | 0.00 / 0.00 | 250.54 / 421.63 |
| lookup/small/c4/delay20 | 145.00 / 203.00 | 13.00 / 20.00 | 26.00 / 31.00 | 8.00 / 13.00 | 38.00 / 72.00 | 10.00 / 22.00 | 20.15 / 20.39 | 235.44 / 319.16 |
| lookup/bounded/c1/delay0 | 208.00 / 220.00 | 8.00 / 12.00 | 5.00 / 8.00 | 5.00 / 10.00 | 16.00 / 37.00 | 5.00 / 13.00 | 0.00 / 0.00 | 253.26 / 288.16 |
| lookup/bounded/c1/delay20 | 187.00 / 196.00 | 8.00 / 12.00 | 25.00 / 27.00 | 5.00 / 9.00 | 17.00 / 30.00 | 4.00 / 7.00 | 20.19 / 20.34 | 234.13 / 279.04 |
| lookup/bounded/c4/delay0 | 150.00 / 234.00 | 13.00 / 28.00 | 11.00 / 23.00 | 9.00 / 15.00 | 50.00 / 94.00 | 12.00 / 26.00 | 0.00 / 0.00 | 266.75 / 403.58 |
| lookup/bounded/c4/delay20 | 150.00 / 212.00 | 13.00 / 27.00 | 27.00 / 36.00 | 9.00 / 21.00 | 41.00 / 108.00 | 12.00 / 27.00 | 20.14 / 20.39 | 250.87 / 363.13 |

Default-lease recovery samples (ms): 45445.74, 45427.82, 45407.17.
Three samples support the maximum-budget check, not a p95 estimate.

Measured warm jobs: 1536; warmups: 384; cold samples: 60.
Warm adapter reads including warmups: 960. Two expected capability denials; no adapter access from probes.
Numerical checks: 132; misses: 0; process cleanup checks: 115.
