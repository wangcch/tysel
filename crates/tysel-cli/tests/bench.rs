use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn bench_without_suite_fails() {
    let output = Command::new(cli_exe()).arg("bench").output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn isolate_suite_is_unavailable_in_human_and_json() {
    let human = Command::new(cli_exe()).args(["bench", "isolate"]).output().unwrap();
    assert_eq!(human.status.code(), Some(1), "{}", String::from_utf8_lossy(&human.stderr));
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("isolate"));
    assert!(stdout.contains("unavailable"));
    assert!(!stdout.contains("12.34"));
    assert!(String::from_utf8_lossy(&human.stderr).contains("unavailable"));

    let json =
        Command::new(cli_exe()).args(["bench", "isolate", "--format", "json"]).output().unwrap();
    assert_eq!(json.status.code(), Some(1));
    let value: serde_json::Value =
        serde_json::from_str(&String::from_utf8(json.stdout).unwrap()).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["suites"][0]["name"], "isolate");
    assert_eq!(value["suites"][0]["status"], "unavailable");
    assert_eq!(value["suites"][0]["reason"], "harness is not implemented yet");
    assert!(value["suites"][0].get("measured").is_none());
    assert!(value["suites"][0].get("samples").is_none());
}

#[test]
fn task_and_durable_suites_are_unavailable() {
    for suite in ["task", "durable"] {
        let output =
            Command::new(cli_exe()).args(["bench", suite, "--format", "json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(1), "{suite}");
        let value: serde_json::Value =
            serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();
        assert_eq!(value["suites"][0]["name"], suite);
        assert_eq!(value["suites"][0]["status"], "unavailable");
        assert!(value["suites"][0].get("measured").is_none());
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
fn evidence_requires_all_and_does_not_write_on_unavailable() {
    let dir = std::env::temp_dir().join(format!("tysel-bench-cli-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let evidence = dir.join("bench.json");
    let output = Command::new(cli_exe())
        .args([
            "bench",
            "isolate",
            "--evidence",
            evidence.to_str().unwrap(),
            "--source-commit",
            &"ab".repeat(20),
            "--command",
            "tysel bench isolate",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("tysel bench all"), "{stderr}");
    assert!(!evidence.exists());
}

#[test]
fn missing_stub_fails_available_suites() {
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
            || stderr.contains("No such"),
        "{stderr}"
    );
}

#[test]
fn available_suites_report_json_when_stub_is_present() {
    let stub = ensure_stub();
    let output = Command::new(cli_exe())
        .args(["bench", "all", "--format", "json"])
        .env("TYSEL_STUB", &stub)
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!output.status.success());
    assert!(
        stderr.contains("benchmark suites are unavailable") || stderr.contains("§30 gates failed"),
        "status={} stderr={stderr} stdout={stdout}",
        output.status
    );
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    let names: Vec<&str> = value["suites"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["startup", "memory", "binary-size", "isolate", "task", "durable"]);
    for name in ["startup", "memory", "binary-size"] {
        let suite =
            value["suites"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
        assert!(matches!(suite["status"].as_str(), Some("pass" | "fail")), "{suite}");
        assert!(suite["measured"].is_number(), "{suite}");
        assert!(suite["limit"].is_number(), "{suite}");
    }
    for name in ["isolate", "task", "durable"] {
        let suite =
            value["suites"].as_array().unwrap().iter().find(|row| row["name"] == name).unwrap();
        assert_eq!(suite["status"], "unavailable");
        assert!(suite.get("measured").is_none(), "{suite}");
    }
}

#[test]
fn evidence_rejects_incomplete_all() {
    let dir = std::env::temp_dir().join(format!("tysel-bench-evidence-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let evidence = dir.join("bench.json");
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let output = Command::new(cli_exe())
        .args([
            "bench",
            "all",
            "--format",
            "json",
            "--evidence",
            evidence.to_str().unwrap(),
            "--source-commit",
            commit,
            "--command",
            "tysel bench all --evidence dist/bench.json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("every requested suite"), "{stderr}");
    assert!(!evidence.exists(), "incomplete suites must not produce evidence");
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
