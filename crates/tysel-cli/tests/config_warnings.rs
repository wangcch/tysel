use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod support;
use support::process::{ManagedChild, wait_listen};

struct Fixture {
    root: PathBuf,
    manifest: PathBuf,
}

impl Fixture {
    fn new(name: &str, json: bool) -> Self {
        let root =
            std::env::temp_dir().join(format!("tysel-config-{name}-{json}-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let manifest = root.join(if json { "tysel.json" } else { "tysel.toml" });
        let raw = if json {
            r#"{"app":{"name":"test","entry":"王😀-index.js"},"server":{"listen":"127.0.0.1:0"},"durable":{"store":"SQLite"},"observability":{"logs":"jsno","traces":"https://user:manifest-secret@collector","metrics":""}}"#
        } else {
            "[app]\nname = 'test'\nentry = '王😀-index.js'\n[server]\nlisten = '127.0.0.1:0'\n[durable]\nstore = 'SQLite'\n[observability]\nlogs = 'jsno'\ntraces = 'https://user:manifest-secret@collector'\nmetrics = ''\n"
        };
        fs::write(&manifest, raw).unwrap();
        fs::write(
            root.join("王😀-index.js"),
            "export default {fetch() {return new Response('config-ok')}};\n",
        )
        .unwrap();
        Self { root, manifest }
    }

    fn command(&self) -> Command {
        self.command_with_format("json")
    }

    fn command_with_format(&self, format: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tysel"));
        for (key, _) in std::env::vars() {
            if key.starts_with("OTEL_") || key.starts_with("TYSEL_DURABLE_") {
                command.env_remove(key);
            }
        }
        command.current_dir(&self.root).args(["--error-format", format]);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assert_warnings(stderr: &[u8], fixture: &Fixture) {
    let event: serde_json::Value = serde_json::from_slice(stderr).unwrap();
    assert_eq!(event["event"], "diagnostics");
    assert!(event.get("error").is_none());
    assert!(event.get("generation").is_none());
    let diagnostics = event["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 4);
    let raw = fs::read_to_string(&fixture.manifest).unwrap();
    for (diagnostic, (code, value)) in diagnostics.iter().zip([
        ("TYSEL_CONFIG_UNSUPPORTED_STORE", "SQLite"),
        ("TYSEL_CONFIG_JSON_LOGS_DISABLED", "jsno"),
        ("TYSEL_CONFIG_IGNORED_TRACES", "https://user:manifest-secret@collector"),
        ("TYSEL_CONFIG_IGNORED_METRICS", ""),
    ]) {
        assert_eq!(diagnostic["code"], code);
        assert_eq!(diagnostic["severity"], "warning");
        assert_eq!(diagnostic["phase"], "manifest");
        assert_eq!(
            fs::canonicalize(diagnostic["file"].as_str().unwrap()).unwrap(),
            fs::canonicalize(&fixture.manifest).unwrap()
        );
        let start = diagnostic["start"]["byteOffset"].as_u64().unwrap() as usize;
        let end = diagnostic["end"]["byteOffset"].as_u64().unwrap() as usize;
        assert_eq!(raw[start + 1..end - 1], *value);
    }
    let output = String::from_utf8_lossy(stderr);
    assert!(!output.contains("manifest-secret"));
    assert!(!output.contains("environment-secret"));
}

#[test]
fn configuration_warnings_are_nonfatal_located_and_do_not_change_serialized_values() {
    for json in [false, true] {
        let fixture = Fixture::new("formats", json);
        // Environment endpoints never make manifest fields active, even at build time.
        for disabled in ["true", "false"] {
            let output = fixture
                .command()
                .args(["config", "validate"])
                .env("OTEL_EXPORTER_OTLP_ENDPOINT", "https://environment-secret.example")
                .env("OTEL_SDK_DISABLED", disabled)
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert_warnings(&output.stderr, &fixture);
        }
        let output =
            fixture.command().args(["config", "show", "--format", "json"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_warnings(&output.stderr, &fixture);
        let shown: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(shown["observability"]["logs"], "jsno");
        assert_eq!(shown["observability"]["traces"], "https://user:manifest-secret@collector");
        let human = Command::new(env!("CARGO_BIN_EXE_tysel"))
            .args(["config", "validate", "--manifest", fixture.manifest.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(human.status.success());
        let stderr = String::from_utf8_lossy(&human.stderr);
        assert!(stderr.contains("warning[TYSEL_CONFIG_IGNORED_TRACES]"));
        assert!(!stderr.contains("manifest-secret"));
    }
}

#[test]
fn configuration_warnings_do_not_mask_a_later_build_failure() {
    let fixture = Fixture::new("failure", true);
    fs::write(fixture.root.join("王😀-index.js"), "export default {\n").unwrap();
    let output = fixture.command().arg("check").output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let events = stderr
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2);
    assert_warnings(stderr.lines().next().unwrap().as_bytes(), &fixture);
    assert_eq!(events[1]["error"]["code"], "TYSEL_CLI_ERROR");
    assert_eq!(events[1]["diagnostics"][0]["severity"], "error");
    assert_eq!(events[1]["diagnostics"][0]["phase"], "parse");
}

fn task_fixture(name: &str, tasks: serde_json::Value) -> Fixture {
    let fixture = Fixture::new(name, true);
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.manifest).unwrap()).unwrap();
    manifest["tasks"] = tasks;
    fs::write(&fixture.manifest, serde_json::to_vec(&manifest).unwrap()).unwrap();
    fixture
}

#[test]
fn native_tasks_inherit_diagnostic_format_in_dependencies_and_steps() {
    let fixture = task_fixture(
        "task-formats",
        serde_json::json!({
            "base": {"steps": [["inspect"]]},
            "verify": {"depends": ["base"], "steps": [["inspect"], ["inspect"]]}
        }),
    );
    for format in ["json", "human"] {
        let output = fixture.command_with_format(format).args(["task", "verify"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        if format == "json" {
            // One warning event from the parent and one from each of its three steps.
            assert_eq!(stderr.lines().count(), 4, "{stderr}");
            for line in stderr.lines() {
                assert_warnings(line.as_bytes(), &fixture);
            }
        } else {
            assert_eq!(stderr.lines().count(), 16, "{stderr}");
            assert!(stderr.lines().all(|line| line.starts_with("warning[TYSEL_CONFIG_")));
        }
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("task base [1/1] tysel inspect"), "{stdout}");
        assert!(stdout.contains("task verify [2/2] tysel inspect"), "{stdout}");
        assert!(stdout.contains("task verify completed"), "{stdout}");
    }
}

#[test]
fn native_tasks_preserve_json_diagnostics_and_stop_on_dependency_failure() {
    let fixture = task_fixture(
        "task-failure",
        serde_json::json!({
            "base": {"steps": [["check"], ["inspect"]]},
            "verify": {"depends": ["base"], "steps": [["inspect"]]}
        }),
    );
    fs::write(fixture.root.join("王😀-index.js"), "export default {\n").unwrap();
    let output = fixture.command().args(["task", "verify"]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let lines = stderr.lines().collect::<Vec<_>>();
    let events = lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 4, "{stderr}");
    for line in &lines[..2] {
        assert_warnings(line.as_bytes(), &fixture);
    }
    assert_eq!(events[2]["diagnostics"][0]["severity"], "error");
    assert_eq!(events[2]["diagnostics"][0]["phase"], "parse");
    assert_eq!(events[3]["error"]["code"], "TYSEL_CLI_ERROR");
    assert!(events[3]["error"]["message"].as_str().unwrap().contains("failed at step 1"));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("tysel inspect"), "{stdout}");
    assert!(!stdout.contains("completed"), "{stdout}");
}

#[test]
fn native_tasks_respect_explicit_step_diagnostic_formats() {
    for format in ["json", "human"] {
        let fixture = task_fixture(
            "task-explicit-format",
            serde_json::json!({"verify": {"steps": [
                ["inspect", "--error-format", format],
                ["inspect", format!("--error-format={format}")]
            ]}}),
        );
        let output = fixture.command().args(["task", "verify"]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        let lines = stderr.lines().collect::<Vec<_>>();
        assert_warnings(lines[0].as_bytes(), &fixture);
        if format == "json" {
            assert_eq!(lines.len(), 3, "{stderr}");
            for line in &lines[1..] {
                assert_warnings(line.as_bytes(), &fixture);
            }
        } else {
            assert_eq!(lines.len(), 9, "{stderr}");
            assert!(lines[1..].iter().all(|line| line.starts_with("warning[TYSEL_CONFIG_")));
        }
    }
}

#[test]
fn build_warns_before_packaging_and_preserves_legacy_runtime_flags() {
    for json in [false, true] {
        let fixture = Fixture::new("build", json);
        let stub = fixture.root.join("stub");
        let artifact = fixture.root.join("app");
        fs::write(&stub, "stub-runtime").unwrap();
        let output = fixture
            .command()
            .args([
                "build",
                "--stub",
                stub.to_str().unwrap(),
                "--output",
                artifact.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_warnings(&output.stderr, &fixture);
        let tap = tysel_package::Tap::from_path(&artifact).unwrap();
        assert!(!tap.manifest.json_logs);
        assert!(tap.manifest.sqlite_path.is_empty());
        assert!(
            !String::from_utf8_lossy(&fs::read(&artifact).unwrap()).contains("manifest-secret")
        );
    }
}

#[test]
fn run_warns_without_enabling_ignored_exporters_or_breaking_http() {
    for json in [false, true] {
        let fixture = Fixture::new("run", json);
        let mut child = ManagedChild::spawn(
            fixture
                .command()
                .arg("run")
                // Invalid endpoint proves local run does not initialize the packaged exporter.
                .env("OTEL_EXPORTER_OTLP_ENDPOINT", "invalid-environment-secret")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
            "config run",
        );
        let (addr, log) = wait_listen(&mut child, Duration::from_secs(10));
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.contains("config-ok"), "{response}");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let captured = log.lock().unwrap().clone();
            if captured.ends_with('\n') {
                assert_warnings(captured.as_bytes(), &fixture);
                break;
            }
            assert!(Instant::now() < deadline, "missing warnings: {captured}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
