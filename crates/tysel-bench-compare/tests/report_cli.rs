use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use tysel_bench_compare::{
    BenchmarkSystem, COMPARISON_SCHEMA_VERSION, ComparisonEvidence, ComparisonSummary, HttpRound,
    HttpWorkloadEvidence, MemoryMeasurement, RuntimeEvidence, ToolchainEvidence, distribution,
};

#[test]
fn report_cli_aggregates_four_rotated_evidence_files() {
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let directory = std::env::temp_dir()
        .join(format!("tysel-bench-report-test-{}-{unique}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let mut input_paths = Vec::new();
    for seed in 1..=4 {
        let path = directory.join(format!("seed-{seed}.json"));
        fs::write(&path, serde_json::to_vec(&evidence(seed)).unwrap()).unwrap();
        input_paths.push(path);
    }
    let output = directory.join("summary.json");
    let mut command = Command::new(env!("CARGO_BIN_EXE_tysel-bench-report"));
    command.arg("--input");
    command.args(&input_paths);
    let result = command.arg("--output").arg(&output).output().unwrap();
    assert!(
        result.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let summary: ComparisonSummary = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    assert_eq!(summary.order_seeds, [1, 2, 3, 4]);
    assert_eq!(summary.inputs.len(), 4);
    assert_eq!(summary.runtimes.len(), 4);
    assert!(output.with_extension("md").is_file());
    fs::remove_dir_all(directory).unwrap();
}

fn evidence(seed: u64) -> ComparisonEvidence {
    let mut ids = vec!["tysel", "node", "bun", "deno"];
    let length = ids.len();
    ids.rotate_left(seed as usize % length);
    ComparisonEvidence {
        schema_version: COMPARISON_SCHEMA_VERSION,
        run_id: format!("run-{seed}"),
        generated_at_unix_ms: seed as u128,
        source_commit: "a".repeat(40),
        workspace_dirty: false,
        command: "compare".into(),
        matrix: "benchmarks/comparison/matrix.toml".into(),
        runtime_lock: "benchmarks/comparison/runtimes.lock.json".into(),
        quick: false,
        order_seed: seed,
        system: BenchmarkSystem {
            os: "linux".into(),
            arch: "x86_64".into(),
            os_version: "test kernel".into(),
            cpu_model: "test cpu".into(),
        },
        toolchains: vec![ToolchainEvidence {
            id: "typescript".into(),
            expected_version: "7.0.2".into(),
            actual_version: "Version 7.0.2".into(),
            executable: "/opt/tsc".into(),
            executable_sha256: "e".repeat(64),
        }],
        runtimes: ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| runtime(id, index + 1, seed))
            .collect(),
    }
}

fn runtime(id: &str, execution_order: usize, seed: u64) -> RuntimeEvidence {
    let hash_character = match id {
        "tysel" => 'a',
        "node" => 'b',
        "bun" => 'c',
        "deno" => 'd',
        _ => unreachable!(),
    };
    RuntimeEvidence {
        id: id.into(),
        expected_version: "1.0.0".into(),
        actual_version: Some(format!("{id} 1.0.0")),
        build_mode: "runtime-source".into(),
        executable: Some(format!("/opt/{id}")),
        executable_sha256: Some(hash_character.to_string().repeat(64)),
        status: "measured".into(),
        reason: None,
        execution_order,
        startup_ms: Some(distribution(vec![10.0; 5], seed)),
        idle_memory: Some(MemoryMeasurement {
            value_kb: 1024,
            kind: "pss".into(),
            process_count: 1,
        }),
        workloads: vec![HttpWorkloadEvidence {
            id: "health".into(),
            path: "/health".into(),
            concurrency: 1,
            round_duration_ms: 500,
            rounds: vec![HttpRound {
                duration_ms: 100.0,
                requests: 100,
                requests_per_second: 1000.0,
                latency_ms: vec![1.0; 100],
                errors: 0,
                server_cpu_core_pct: Some(75.0),
                client_cpu_core_pct: Some(25.0),
                peak_memory_kb: Some(2048),
                memory_kind: Some("pss".into()),
            }],
            requests_per_second: distribution(vec![1000.0], seed),
            latency_ms: distribution(vec![1.0; 100], seed),
            errors: 0,
        }],
    }
}
