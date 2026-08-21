use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn bench_without_suite_fails() {
    let output = Command::new(cli_exe()).arg("bench").output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn advanced_suites_emit_real_metrics() {
    for (suite, required_metric) in [
        ("isolate", "warm_create_ms"),
        ("task", "enqueue_100_ms"),
        ("durable", "resume_ms"),
        ("http", "http1_keepalive_ms"),
    ] {
        let output = quick_bench(&["bench", suite, "--format", "json"]);
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
                panic!("{suite}: {error}; stderr={}", String::from_utf8_lossy(&output.stderr))
            });
        assert_eq!(value["schemaVersion"], 2, "{suite}");
        assert_eq!(value["suites"][0]["name"], suite, "{suite}");
        assert!(matches!(value["suites"][0]["status"].as_str(), Some("pass" | "fail")));
        let metrics = value["suites"][0]["metrics"].as_array().unwrap();
        let metric = metrics
            .iter()
            .find(|metric| metric["name"] == required_metric)
            .unwrap_or_else(|| panic!("missing {required_metric} in {suite}"));
        assert!(metric["samples"].as_array().is_some_and(|samples| !samples.is_empty()));
        assert!(metric["p50"].is_number());
    }
}

#[test]
fn allow_unavailable_is_restricted_to_all() {
    let output =
        Command::new(cli_exe()).args(["bench", "isolate", "--allow-unavailable"]).output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("only valid with `tysel bench all`"));
}

#[test]
fn evidence_requires_all() {
    let evidence = std::env::temp_dir().join(format!("tysel-bench-{}.json", std::process::id()));
    let output = Command::new(cli_exe())
        .args([
            "bench",
            "isolate",
            "--evidence",
            evidence.to_str().unwrap(),
            "--source-commit",
            &"ab".repeat(20),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("tysel bench all"));
    assert!(!evidence.exists());
}

#[test]
fn debug_cli_cannot_emit_release_evidence() {
    let evidence = std::env::temp_dir()
        .join(format!("tysel-debug-bench-evidence-{}.json", std::process::id()));
    let output = Command::new(cli_exe())
        .args([
            "bench",
            "all",
            "--evidence",
            evidence.to_str().unwrap(),
            "--source-commit",
            &"ab".repeat(20),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a release build"));
    assert!(!evidence.exists());
}

#[test]
fn missing_stub_fails_baseline_suites() {
    let output = Command::new(cli_exe())
        .args(["bench", "startup", "--format", "json"])
        .env("TYSEL_STUB", "/no/such/tysel-service")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("tysel-service")
            || stderr.contains("measure")
            || stderr.contains("No such")
    );
}

#[test]
fn all_contains_the_complete_matrix() {
    let stub = ensure_stub();
    let output = Command::new(cli_exe())
        .args(["bench", "all", "--format", "json"])
        .env("TYSEL_BENCH_QUICK", "1")
        .env("TYSEL_STUB", &stub)
        .output()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("{error}; stderr={}", String::from_utf8_lossy(&output.stderr))
    });
    let names: Vec<&str> = value["suites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["startup", "memory", "binary-size", "isolate", "task", "durable", "http"]);
    for suite in value["suites"].as_array().unwrap() {
        assert_ne!(suite["status"], "unavailable", "{suite}");
        assert!(suite["metrics"].as_array().is_some_and(|metrics| !metrics.is_empty()));
    }
}

fn quick_bench(args: &[&str]) -> Output {
    Command::new(cli_exe()).args(args).env("TYSEL_BENCH_QUICK", "1").output().unwrap()
}

fn ensure_stub() -> PathBuf {
    if let Some(path) = locate_stub() {
        return path;
    }
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "tysel-runtime", "--bin", "tysel-service"])
        .status()
        .expect("build tysel-service");
    assert!(status.success(), "failed to build tysel-service stub");
    locate_stub().expect("tysel-service stub after build")
}

fn locate_stub() -> Option<PathBuf> {
    let mut next_to_cli = cli_exe();
    next_to_cli.set_file_name("tysel-service");
    if cfg!(windows) {
        next_to_cli.set_extension("exe");
    }
    if next_to_cli.is_file() {
        return Some(next_to_cli);
    }
    tysel_testkit::find_stub().ok().filter(|path| path.is_file())
}

fn cli_exe() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_tysel") {
        return PathBuf::from(path);
    }
    let test_exe = std::env::current_exe().expect("current_exe");
    let mut candidate = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("tysel"))
        .expect("target debug directory");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    assert!(candidate.is_file(), "missing tysel at {}", candidate.display());
    candidate
}
