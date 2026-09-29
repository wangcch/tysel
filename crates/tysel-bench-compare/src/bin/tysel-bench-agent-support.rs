//! Small IPC bridge so the Python application fixture reuses benchmark metrics.
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};

use anyhow::{Result, ensure};
use serde::Deserialize;
use tysel_bench_compare::{benchmark_system, distribution, process_memory_kb, process_tree_pids};

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
enum Request {
    Memory { roots: Vec<u32> },
    Stats { series: BTreeMap<String, Vec<f64>> },
    System,
}

fn main() -> Result<()> {
    // One request/reply per line, allowing a persistent low-overhead sampler.
    for line in std::io::stdin().lock().lines() {
        let reply = match serde_json::from_str::<Request>(&line?)? {
            Request::System => serde_json::to_value(benchmark_system())?,
            Request::Stats { series } => {
                let mut out = BTreeMap::new();
                for (name, values) in series {
                    ensure!(
                        !values.is_empty() && values.iter().all(|v| v.is_finite()),
                        "invalid samples"
                    );
                    let mut result = distribution(values, 1);
                    // Samples in an application round are correlated. No IID confidence claim.
                    result.p50_ci95 = None;
                    out.insert(name, result);
                }
                serde_json::to_value(out)?
            }
            Request::Memory { roots } => {
                ensure!(!roots.is_empty(), "memory roots required");
                let mut pids = BTreeSet::new();
                for root in roots {
                    pids.extend(process_tree_pids(root)?);
                }
                let mut total = 0;
                let mut kind = None;
                // Fail rather than silently undercounting a still-live application tree.
                for pid in &pids {
                    let (value, measured_kind) = process_memory_kb(*pid)?;
                    ensure!(kind.is_none_or(|k| k == measured_kind), "mixed memory kinds");
                    kind = Some(measured_kind);
                    total += value;
                }
                serde_json::json!({"valueKiB": total, "kind": kind, "pids": pids})
            }
        };
        println!("{}", serde_json::to_string(&reply)?);
        std::io::stdout().flush()?;
    }
    Ok(())
}
