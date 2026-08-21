# npm compatibility

Tysel is Web-API-first and does not attempt general Node.js compatibility. `tysel compat` scans direct package dependencies plus Node builtin imports visible in the entry.

| Status | Meaning |
| --- | --- |
| `compatible` | In the reviewed compatibility catalog. |
| `shim` | Requires an explicit Web/Tysel shim. |
| `unsupported` | Requires an unavailable builtin, native addon, or runtime model. |
| `unknown` | Not reviewed; never reported as compatible. |

`--strict` rejects unsupported dependencies. `--deny-unknown` extends the policy to unknown packages. `--json` emits schema version 1 with counts and reasons. Package classification is only an early warning; use `tysel check` and tests as the acceptance gate.
