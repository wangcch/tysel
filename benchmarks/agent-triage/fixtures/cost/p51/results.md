# P5 cost results

Status: **budget_miss**. Linux ARM64 release on the named local VM.

These are descriptive local measurements, not production SLOs or a cross-runtime ranking.

| Variant / size / clients / adapter delay | p50 ms | p95 ms | Jobs/s, range of 3 rounds | Sampled peak PSS MiB | Budget |
| --- | ---: | ---: | ---: | ---: | --- |
| direct/small/c1/delay0 | 0.68 | 0.99 | 1244.26–1484.93 | 9.33 | pass |
| direct/small/c4/delay0 | 1.43 | 2.19 | 2038.78–2883.18 | 9.52 | pass |
| direct/bounded/c1/delay0 | 0.69 | 0.83 | 1295.88–1455.15 | 9.32 | pass |
| direct/bounded/c4/delay0 | 2.01 | 5.51 | 917.30–2713.68 | 9.49 | pass |
| snapshot/small/c1/delay0 | 1.00 | 2.05 | 625.15–1191.94 | 12.29 | pass |
| snapshot/small/c4/delay0 | 2.06 | 3.83 | 1350.58–2060.99 | 12.57 | pass |
| snapshot/bounded/c1/delay0 | 1.02 | 1.92 | 686.52–1148.79 | 12.33 | pass |
| snapshot/bounded/c4/delay0 | 2.27 | 3.84 | 1237.38–1881.88 | 12.62 | pass |
| lookup/small/c1/delay0 | 272.16 | 586.49 | 2.64–3.62 | 41.19 | pass |
| lookup/small/c1/delay20 | 399.61 | 645.74 | 2.11–2.86 | 42.33 | pass |
| lookup/small/c4/delay0 | 862.07 | 1360.49 | 3.81–4.96 | 54.64 | MISS |
| lookup/small/c4/delay20 | 787.93 | 976.06 | 4.72–5.34 | 52.63 | MISS |
| lookup/bounded/c1/delay0 | 307.06 | 559.38 | 2.25–3.39 | 41.64 | pass |
| lookup/bounded/c1/delay20 | 305.28 | 567.73 | 2.50–3.15 | 42.66 | pass |
| lookup/bounded/c4/delay0 | 746.21 | 921.16 | 5.22–5.44 | 51.29 | MISS |
| lookup/bounded/c4/delay20 | 733.63 | 922.06 | 5.28–5.46 | 49.98 | MISS |

Cold process-to-first-result (20 samples each, warm filesystem caches):

| Variant | p50 ms | p95 ms |
| --- | ---: | ---: |
| direct | 27.15 | 42.31 |
| snapshot | 43.69 | 58.41 |
| lookup | 763.79 | 954.55 |

Lookup stages below are **p50 / p95 milliseconds** from transactional audit timestamps,
with millisecond resolution. Stage percentiles do not sum to end-to-end percentiles.

| Cell | Queue → select | Select | Read | Summarize | Between stages | Terminal commit | Actual adapter sleep | E2E excluding sleep |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| lookup/small/c1/delay0 | 89.00 / 304.00 | 20.00 / 36.00 | 15.00 / 27.00 | 16.00 / 26.00 | 88.00 / 149.00 | 27.00 / 50.00 | 0.00 / 0.00 | 272.16 / 586.49 |
| lookup/small/c1/delay20 | 179.00 / 326.00 | 20.00 / 37.00 | 36.00 / 53.00 | 16.00 / 29.00 | 88.00 / 157.00 | 27.00 / 53.00 | 20.16 / 20.34 | 379.51 / 624.20 |
| lookup/small/c4/delay0 | 299.00 / 501.00 | 50.00 / 109.00 | 48.00 / 94.00 | 41.00 / 84.00 | 269.00 / 495.00 | 75.00 / 213.00 | 0.00 / 0.00 | 862.07 / 1360.49 |
| lookup/small/c4/delay20 | 301.00 / 426.00 | 47.00 / 71.00 | 61.00 / 97.00 | 39.00 / 65.00 | 231.00 / 330.00 | 65.00 / 128.00 | 20.16 / 20.33 | 767.74 / 955.90 |
| lookup/bounded/c1/delay0 | 98.00 / 309.00 | 19.00 / 28.00 | 15.00 / 26.00 | 16.00 / 31.00 | 85.00 / 140.00 | 27.00 / 54.00 | 0.00 / 0.00 | 307.06 / 559.38 |
| lookup/bounded/c1/delay20 | 88.00 / 309.00 | 19.00 / 30.00 | 35.00 / 47.00 | 15.00 / 22.00 | 86.00 / 124.00 | 26.00 / 46.00 | 20.17 / 20.33 | 285.09 / 547.53 |
| lookup/bounded/c4/delay0 | 265.00 / 416.00 | 48.00 / 74.00 | 39.00 / 65.00 | 34.00 / 64.00 | 219.00 / 329.00 | 67.00 / 114.00 | 0.00 / 0.00 | 746.21 / 921.16 |
| lookup/bounded/c4/delay20 | 283.00 / 388.00 | 43.00 / 66.00 | 60.00 / 93.00 | 36.00 / 57.00 | 212.00 / 280.00 | 56.00 / 141.00 | 20.16 / 20.38 | 712.72 / 901.90 |

Default-lease recovery samples (ms): 45647.12, 45548.33, 45671.18.
Three samples support the maximum-budget check, not a p95 estimate.

Measured warm jobs: 1536; warmups: 384; cold samples: 60.
Warm adapter reads including warmups: 960. Two expected capability denials; no adapter access from probes.
Numerical checks: 132; misses: 15; process cleanup checks: 115.

Budget misses:

```json
[
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "warmP95Ms",
    "actual": 1360.489893995691,
    "limit": 1000,
    "relation": "<=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round0.p95Ms",
    "actual": 1205.1947030122392,
    "limit": 1000,
    "relation": "<=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round0.jobsPerSec",
    "actual": 4.55818434421273,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round1.p95Ms",
    "actual": 1536.8081500055268,
    "limit": 1000,
    "relation": "<=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round1.jobsPerSec",
    "actual": 3.8095693792126006,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay0",
    "metric": "round2.jobsPerSec",
    "actual": 4.963491551275829,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round0.jobsPerSec",
    "actual": 5.335947604381662,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round1.jobsPerSec",
    "actual": 4.717593377483912,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/small/c4/delay20",
    "metric": "round2.jobsPerSec",
    "actual": 4.918617817675886,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round0.jobsPerSec",
    "actual": 5.22900871205218,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round1.jobsPerSec",
    "actual": 5.216195210955965,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay0",
    "metric": "round2.jobsPerSec",
    "actual": 5.438544210885507,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round0.jobsPerSec",
    "actual": 5.4648556452669865,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round1.jobsPerSec",
    "actual": 5.276442934018905,
    "limit": 6,
    "relation": ">=",
    "passed": false
  },
  {
    "cell": "lookup/bounded/c4/delay20",
    "metric": "round2.jobsPerSec",
    "actual": 5.275675101152556,
    "limit": 6,
    "relation": ">=",
    "passed": false
  }
]
```
