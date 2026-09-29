# P5 cost results

Status: **budget_miss**. Linux ARM64 release on the named local VM.

These are descriptive local measurements, not production SLOs or a cross-runtime ranking.

| Variant / size / clients / adapter delay | p50 ms | p95 ms | Jobs/s, range of 3 rounds | Sampled peak PSS MiB | Budget |
| --- | ---: | ---: | ---: | ---: | --- |
| direct/small/c1/delay0 | 0.73 | 0.94 | 1164.62–1532.34 | 9.32 | pass |
| direct/small/c4/delay0 | 1.32 | 2.21 | 2190.49–2876.14 | 9.50 | pass |
| direct/bounded/c1/delay0 | 0.72 | 0.93 | 1127.58–1499.87 | 9.33 | pass |
| direct/bounded/c4/delay0 | 1.40 | 3.36 | 1528.43–2662.36 | 9.51 | pass |
| snapshot/small/c1/delay0 | 0.83 | 2.12 | 590.09–1204.88 | 12.26 | pass |
| snapshot/small/c4/delay0 | 1.85 | 3.33 | 1739.93–2158.81 | 12.57 | pass |
| snapshot/bounded/c1/delay0 | 0.87 | 0.99 | 1057.08–1152.59 | 12.30 | pass |
| snapshot/bounded/c4/delay0 | 2.03 | 3.48 | 1459.15–1889.63 | 12.61 | pass |
| lookup/small/c1/delay0 | 270.97 | 484.47 | 3.16–3.41 | 41.77 | pass |
| lookup/small/c1/delay20 | 284.54 | 547.56 | 2.69–3.10 | 41.94 | pass |
| lookup/small/c4/delay0 | 721.12 | 998.98 | 5.07–5.41 | 51.18 | MISS |
| lookup/small/c4/delay20 | 783.61 | 974.88 | 4.79–5.21 | 49.84 | MISS |
| lookup/bounded/c1/delay0 | 265.16 | 474.75 | 3.27–3.39 | 42.20 | pass |
| lookup/bounded/c1/delay20 | 363.24 | 578.31 | 2.35–2.66 | 42.18 | pass |
| lookup/bounded/c4/delay0 | 708.48 | 929.59 | 5.38–5.54 | 50.26 | MISS |
| lookup/bounded/c4/delay20 | 736.18 | 951.37 | 5.09–5.37 | 50.40 | MISS |

Cold process-to-first-result (20 samples each, warm filesystem caches):

| Variant | p50 ms | p95 ms |
| --- | ---: | ---: |
| direct | 26.25 | 36.27 |
| snapshot | 50.55 | 54.70 |
| lookup | 705.96 | 768.44 |

Lookup stages below are **p50 / p95 milliseconds** from transactional audit timestamps,
with millisecond resolution. Stage percentiles do not sum to end-to-end percentiles.

| Cell | Queue → select | Select | Read | Summarize | Between stages | Terminal commit | Actual adapter sleep | E2E excluding sleep |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| lookup/small/c1/delay0 | 88.00 / 299.00 | 18.00 / 28.00 | 14.00 / 23.00 | 14.00 / 20.00 | 81.00 / 103.00 | 25.00 / 33.00 | 0.00 / 0.00 | 270.97 / 484.47 |
| lookup/small/c1/delay20 | 80.00 / 308.00 | 18.00 / 25.00 | 35.00 / 41.00 | 15.00 / 18.00 | 79.00 / 114.00 | 26.00 / 37.00 | 20.18 / 20.31 | 264.36 / 527.47 |
| lookup/small/c4/delay0 | 259.00 / 471.00 | 42.00 / 61.00 | 34.00 / 56.00 | 29.00 / 58.00 | 172.00 / 304.00 | 51.00 / 117.00 | 0.00 / 0.00 | 721.12 / 998.98 |
| lookup/small/c4/delay20 | 342.00 / 479.00 | 41.00 / 67.00 | 54.00 / 91.00 | 32.00 / 55.00 | 187.00 / 276.00 | 52.00 / 122.00 | 20.14 / 20.35 | 763.23 / 954.74 |
| lookup/bounded/c1/delay0 | 81.00 / 291.00 | 18.00 / 25.00 | 15.00 / 22.00 | 15.00 / 23.00 | 82.00 / 122.00 | 26.00 / 36.00 | 0.00 / 0.00 | 265.16 / 474.75 |
| lookup/bounded/c1/delay20 | 186.00 / 332.00 | 18.00 / 27.00 | 35.00 / 45.00 | 15.00 / 21.00 | 83.00 / 114.00 | 26.00 / 39.00 | 20.18 / 20.32 | 343.13 / 558.10 |
| lookup/bounded/c4/delay0 | 280.00 / 457.00 | 41.00 / 61.00 | 32.00 / 62.00 | 33.00 / 48.00 | 179.00 / 263.00 | 49.00 / 121.00 | 0.00 / 0.00 | 708.48 / 929.59 |
| lookup/bounded/c4/delay20 | 283.00 / 449.00 | 42.00 / 63.00 | 58.00 / 81.00 | 33.00 / 59.00 | 181.00 / 272.00 | 49.00 / 109.00 | 20.16 / 20.31 | 716.05 / 931.23 |

Default-lease recovery samples (ms): 45455.15, 45400.53, 45381.11.
Three samples support the maximum-budget check, not a p95 estimate.

Measured warm jobs: 1536; warmups: 384; cold samples: 60.
Warm adapter reads including warmups: 960. Two expected capability denials; no adapter access from probes.
Numerical checks: 132; misses: 14; process cleanup checks: 115.

Budget misses:

```json
[
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round0.p95Ms",
    "actual": 1053.7584729900118,
    "limit": 1000,
    "relation": "<=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round0.jobsPerSec",
    "actual": 5.07131899067472,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round1.jobsPerSec",
    "actual": 5.410485196621163,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round2.jobsPerSec",
    "actual": 5.291156004960141,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round0.p95Ms",
    "actual": 1020.83818797837,
    "limit": 1000,
    "relation": "<=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round0.jobsPerSec",
    "actual": 4.794835448457946,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round1.jobsPerSec",
    "actual": 5.171304076646268,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round2.jobsPerSec",
    "actual": 5.214866980250659,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round0.jobsPerSec",
    "actual": 5.375118331469005,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round1.jobsPerSec",
    "actual": 5.465904095490852,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round2.jobsPerSec",
    "actual": 5.538272318167844,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round0.jobsPerSec",
    "actual": 5.093928237747746,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round1.jobsPerSec",
    "actual": 5.3660330344690275,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round2.jobsPerSec",
    "actual": 5.08513995959796,
    "limit": 6,
    "relation": ">=",
    "passed": false
  }
]
```
