use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

#[test]
fn run_executes_a_component_over_stdio() {
    let dir = temp_app("run-component");
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "echo-component"
entry = "echo.wasm"
profile = "component"
"#,
    )
    .unwrap();
    fs::write(dir.join("echo.wasm"), echo_component()).unwrap();
    let mut child = Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Component");
    child.stdin.take().unwrap().write_all(br#"{"value":42}"#).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "{\"value\":42}\n");
}

#[cfg(unix)]
#[test]
fn mcp_stdio_discovers_lists_and_executes_a_tool() {
    let dir = temp_app("mcp-stdio");
    write_js_app(
        &dir,
        r#"export default {
  async fetch() { return new Response("ok"); },
  tasks: {
    analyze: {
      kind: "mcp",
      description: "Analyze a customer",
      input: { customerId: "string" },
      handler(input) { return { customer: input.customerId, risk: "low" }; },
    },
  },
};
"#,
    );
    let mut child = Command::new(cli_exe())
        .args(["mcp", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn tysel mcp");
    let meta = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#;
    let requests = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{{{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{{{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"analyze\",\"arguments\":{{\"customerId\":\"customer-1\"}},{meta}}}}}\n"
    );
    child.stdin.take().unwrap().write_all(requests.as_bytes()).unwrap();
    let output = child.wait_with_output().expect("wait for MCP server");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<serde_json::Value> =
        stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(responses.len(), 3, "{stdout}");
    assert_eq!(responses[0]["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(responses[1]["result"]["tools"][0]["name"], "analyze");
    assert_eq!(responses[2]["result"]["structuredContent"]["customer"], "customer-1");
}

#[cfg(unix)]
#[test]
fn run_starts_registered_cron_tasks() {
    let dir = temp_app("run-cron");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "cron-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"

[permissions]
fs_write = ["./data"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() { return new Response("ok"); },
  tasks: {
    heartbeat: {
      kind: "cron",
      expression: "* * * * *",
      async handler() { await tysel.fs.write("data/cron.txt", "ran"); },
    },
  },
};
"#,
    )
    .unwrap();
    let mut child = Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn tysel run");
    let stdout = child.stdout.take().expect("stdout");
    let _ = wait_listen(stdout, Duration::from_secs(8));
    let marker = dir.join("data/cron.txt");
    let started = std::time::Instant::now();
    while !marker.exists() && started.elapsed() < Duration::from_secs(3) {
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(fs::read_to_string(marker).unwrap(), "ran");
}

#[cfg(unix)]
#[test]
fn queue_command_submits_json_and_prints_the_handler_result() {
    let dir = temp_app("queue-command");
    write_js_app(
        &dir,
        r#"export default {
  async fetch() { return new Response("ok"); },
  tasks: {
    consume: {
      kind: "queue",
      name: "orders",
      handler(input, ctx) { return { order: input.order, requestId: ctx.requestId }; },
    },
  },
};
"#,
    );
    let output = Command::new(cli_exe())
        .args([
            "queue",
            "orders",
            "--input",
            r#"{"order":42}"#,
            "--message-id",
            "message-42",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
        ])
        .output()
        .expect("run tysel queue");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["order"].as_f64(), Some(42.0));
    assert!(result["requestId"].as_str().is_some_and(|id| id.len() == 32));
}

#[test]
fn check_reports_ok_for_a_javascript_service() {
    let dir = temp_app("check-ok");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"ok\"); } };\n");
    let output = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .expect("run tysel check");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Application: hello-service"));
    assert!(stdout.contains("manifest  ok"));
    assert!(stdout.contains("bundle    ok"));
}

#[test]
fn check_fails_when_the_entry_is_missing() {
    let dir = temp_app("check-missing");
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/missing.ts"
profile = "service"
"#,
    )
    .unwrap();
    let output = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .expect("run tysel check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("entry not found"), "{stderr}");
}

#[test]
fn check_rejects_a_postgres_url() {
    let dir = temp_app("check-pg-url");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[permissions]
postgres = ["postgres://user:pass@localhost/db"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"ok\"); } };\n",
    )
    .unwrap();
    let output = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .expect("run tysel check");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not a URL"), "{stderr}");
}

#[test]
fn init_writes_a_hello_service_skeleton() {
    let dir = temp_app("init-app");
    let _ = fs::remove_dir_all(&dir);
    let output = Command::new(cli_exe()).args(["init", dir.to_str().unwrap()]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(dir.join("src/index.ts").is_file());
    assert!(dir.join("tests/app.test.ts").is_file());
    assert!(!dir.join("tests/tysel-test.d.ts").exists());
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("package.json")).unwrap()).unwrap();
    assert_eq!(package["devDependencies"]["@tysel/test"], "0.0.1");
    assert!(dir.join(".gitignore").is_file());
    assert!(dir.join("tysel.toml").is_file());
    let check = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    let tests = Command::new(cli_exe())
        .args(["test", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(tests.status.success(), "{}", String::from_utf8_lossy(&tests.stderr));
    let test_stdout = String::from_utf8_lossy(&tests.stdout);
    assert!(test_stdout.contains("1 passed, 0 failed"), "{test_stdout}");
}

#[test]
fn init_preflights_conflicts_without_partial_writes() {
    let dir = temp_app("init-conflict");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("tysel.toml"), "keep me\n").unwrap();

    let output = Command::new(cli_exe()).args(["init", dir.to_str().unwrap()]).output().unwrap();
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(dir.join("tysel.toml")).unwrap(), "keep me\n");
    assert!(!dir.join("src").exists());
    assert!(!dir.join("package.json").exists());
}

#[test]
fn check_rejects_node_builtin_imports() {
    let dir = temp_app("check-node");
    write_js_app(
        &dir,
        "import fs from \"fs\";\nexport default { async fetch() { return new Response(String(fs)); } };\n",
    );
    let output = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(text.contains("fs") || text.contains("Node builtin"), "{text}");
}

#[test]
fn compat_classifies_project_dependencies() {
    let dir = temp_app("compat-deps");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"ok\"); } };\n");
    fs::write(
        dir.join("package.json"),
        r#"{
  "name": "compat-deps",
  "private": true,
  "dependencies": { "hono": "4.0.0", "sharp": "0.33.0", "buffer": "6.0.0" }
}"#,
    )
    .unwrap();
    let output = Command::new(cli_exe()).current_dir(&dir).arg("compat").output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hono"), "{stdout}");
    assert!(stdout.contains("sharp"), "{stdout}");
    assert!(stdout.contains("Node native addon"), "{stdout}");
    assert!(stdout.contains("Requires Shim\n  buffer"), "{stdout}");
}

#[test]
fn compat_json_is_machine_readable_and_strict_controls_exit_status() {
    let dir = temp_app("compat-json");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"ok\"); } };\n");
    fs::write(
        dir.join("package.json"),
        r#"{
  "name": "compat-json",
  "dependencies": { "sharp": "0.33.0", "mystery-package": "1.0.0" }
}"#,
    )
    .unwrap();

    let report =
        Command::new(cli_exe()).current_dir(&dir).args(["compat", "--json"]).output().unwrap();
    assert!(report.status.success(), "{}", String::from_utf8_lossy(&report.stderr));
    let value: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["summary"]["unsupported"], 1);
    assert_eq!(value["summary"]["unknown"], 1);

    let strict =
        Command::new(cli_exe()).current_dir(&dir).args(["compat", "--strict"]).output().unwrap();
    assert!(!strict.status.success());
}

#[test]
fn test_command_reports_async_failures_as_json() {
    let dir = temp_app("test-json");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"ok\"); } };\n");
    fs::create_dir_all(dir.join("tests")).unwrap();
    fs::write(
        dir.join("tests/example.test.ts"),
        r#"test("passes asynchronously", async () => {
  await new Promise((resolve) => setTimeout(resolve, 1));
  assert.equal(2 + 2, 4);
});

test("reports a failure", () => {
  assert.deepEqual({ value: 1 }, { value: 2 });
});

test("interrupts a synchronous loop", () => {
  while (true) {}
});

test("continues after a timeout", () => {
  assert(true);
});
"#,
    )
    .unwrap();

    let output = Command::new(cli_exe())
        .args([
            "test",
            "--json",
            "--timeout-ms",
            "100",
            "--manifest",
            dir.join("tysel.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], 2);
    assert_eq!(report["failed"], 2);
    assert_eq!(report["files"][0]["tests"][1]["status"], "failed");
    assert!(
        report["files"][0]["tests"][1]["error"]
            .as_str()
            .is_some_and(|error| error.contains("example.test.ts:")),
        "{report}"
    );
    assert!(
        report["files"][0]["tests"][2]["error"]
            .as_str()
            .is_some_and(|error| error.contains("timed out after 100ms")),
        "{report}"
    );
    assert_eq!(report["files"][0]["tests"][3]["status"], "passed");
}

#[test]
fn test_command_enforces_manifest_capabilities() {
    let dir = temp_app("test-capabilities");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"ok\"); } };\n");
    fs::create_dir_all(dir.join("tests")).unwrap();
    fs::write(
        dir.join("tests/capability.test.ts"),
        r#"test("fetch is denied", async () => {
  await fetch("http://192.0.2.1/");
});
"#,
    )
    .unwrap();

    let output = Command::new(cli_exe())
        .args(["test", "--json", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["failed"], 1);
    assert!(
        report["files"][0]["tests"][0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("not permitted"))
    );
}

#[test]
fn fatal_cli_errors_support_json_output() {
    let dir = temp_app("json-error");
    let output = Command::new(cli_exe())
        .args([
            "--error-format",
            "json",
            "check",
            "--manifest",
            dir.join("missing.toml").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "TYSEL_CLI_ERROR");
    assert!(
        error["error"]["message"].as_str().is_some_and(|message| message.contains("missing.toml"))
    );
}

#[test]
fn check_fails_on_invalid_typescript() {
    let dir = temp_app("check-syntax");
    write_js_app(&dir, "export default {\n");
    fs::write(dir.join("src/index.ts"), "export default {\n").unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.ts"
profile = "service"
"#,
    )
    .unwrap();
    let output = Command::new(cli_exe())
        .args(["check", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .output()
        .expect("run tysel check");
    assert!(!output.status.success());
}

#[test]
fn dev_serves_hello_until_killed() {
    let dir = temp_app("dev-hello");
    write_js_app(
        &dir,
        r#"export default {
  async fetch() {
    return Response.json({ ok: true });
  },
};
"#,
    );
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let mut stream = TcpStream::connect(&addr).expect("connect");
    stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut body = String::new();
    stream.read_to_string(&mut body).unwrap();
    let _ = child.kill();
    let _ = child.wait();
    assert!(body.contains("200"), "{body}");
    assert!(body.contains("\"ok\":true") || body.contains("\"ok\": true"), "{body}");
}

#[test]
fn run_serves_hello_until_killed() {
    let dir = temp_app("run-hello");
    write_js_app(
        &dir,
        r#"export default {
  async fetch() {
    return Response.json({ ok: true });
  },
};
"#,
    );

    let mut child = Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn tysel run");
    let stdout = child.stdout.take().expect("stdout");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let body = http_get(&addr);
    let _ = child.kill();
    let _ = child.wait();
    assert!(body.contains("200"), "{body}");
    assert!(body.contains("\"ok\":true") || body.contains("\"ok\": true"), "{body}");
}

#[test]
fn durable_agent_resumes_after_restart_without_repeating_effects() {
    let (llm_addr, llm_calls, llm_stop) = spawn_fake_llm();
    let dir = temp_app("durable-agent");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "durable-agent"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:0"

[permissions]
secrets = ["OPENAI_API_KEY"]

[limits]
request_timeout_ms = 5000

[durable]
store = "sqlite"
path = "./data/tysel.db"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.ts"),
        include_str!("../../../examples/durable-agent/src/index.ts"),
    )
    .unwrap();
    fs::write(dir.join(".env"), "OPENAI_API_KEY=test-key\n").unwrap();

    let mut first = spawn_durable_agent(&dir, &llm_addr);
    let first_addr = wait_listen(first.stdout.take().expect("stdout"), Duration::from_secs(8));
    let started = http_json(
        &first_addr,
        "POST",
        "/runs",
        Some(r#"{"customerId":"customer-1","prompt":"Summarize this account"}"#),
    );
    let run_id = started["runId"].as_str().expect("runId").to_owned();
    assert_eq!(started["status"], "awaiting_approval");
    assert_eq!(llm_calls.load(Ordering::SeqCst), 1);
    first.kill().unwrap();
    first.wait().unwrap();

    let mut second = spawn_durable_agent(&dir, &llm_addr);
    let second_addr = wait_listen(second.stdout.take().expect("stdout"), Duration::from_secs(8));
    let waiting = http_json(&second_addr, "GET", &format!("/runs/{run_id}"), None);
    assert_eq!(waiting["status"], "awaiting_approval");
    assert_eq!(llm_calls.load(Ordering::SeqCst), 1, "LLM effect replayed after restart");
    let queued = http_json(
        &second_addr,
        "POST",
        &format!("/runs/{run_id}/approval"),
        Some(r#"{"approved":true}"#),
    );
    assert_eq!(queued["status"], "approval_queued");

    let started_wait = std::time::Instant::now();
    let completed = loop {
        let run = http_json(&second_addr, "GET", &format!("/runs/{run_id}"), None);
        if run["status"] == "completed" {
            break run;
        }
        assert!(started_wait.elapsed() < Duration::from_secs(5), "run did not complete: {run}");
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(completed["result"]["approved"], true);
    assert_eq!(completed["saveCount"], 1);
    assert_eq!(llm_calls.load(Ordering::SeqCst), 1, "LLM effect ran more than once");
    second.kill().unwrap();
    second.wait().unwrap();

    let mut third = spawn_durable_agent(&dir, &llm_addr);
    let third_addr = wait_listen(third.stdout.take().expect("stdout"), Duration::from_secs(8));
    let replayed = http_json(&third_addr, "GET", &format!("/runs/{run_id}"), None);
    assert_eq!(replayed["status"], "completed");
    assert_eq!(replayed["saveCount"], 1, "save-result effect replayed after restart");
    assert_eq!(llm_calls.load(Ordering::SeqCst), 1);
    third.kill().unwrap();
    third.wait().unwrap();
    llm_stop.store(true, Ordering::SeqCst);
}

#[test]
fn run_does_not_reload_when_source_changes() {
    let dir = temp_app("run-no-reload");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"v1\"); } };\n");

    let mut child = Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel run");
    let stdout = child.stdout.take().expect("stdout");
    let log = capture_output(child.stderr.take().expect("stderr"));
    let addr = wait_listen(stdout, Duration::from_secs(8));
    assert!(http_get(&addr).contains("v1"), "initial body");

    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"v2\"); } };\n",
    )
    .unwrap();
    thread::sleep(Duration::from_millis(500));
    assert!(
        !log.lock().expect("log").contains("tysel reload"),
        "tysel run reloaded on source change: {}",
        log.lock().expect("log")
    );
    assert!(http_get(&addr).contains("v1"), "tysel run served reloaded source");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_reloads_source_but_ignores_node_modules() {
    let dir = temp_app("dev-reload");
    write_js_app(&dir, "export default { async fetch() { return new Response(\"v1\"); } };\n");
    fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
    fs::write(dir.join("node_modules/pkg/index.js"), "export const x = 1;\n").unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let log = capture_output(stderr);
    assert!(http_get(&addr).contains("v1"), "initial body");

    thread::sleep(Duration::from_millis(300));
    log.lock().expect("log").clear();
    fs::write(dir.join("node_modules/pkg/index.js"), "export const x = 2;\n").unwrap();
    thread::sleep(Duration::from_millis(400));
    assert!(
        !log.lock().expect("log").contains("tysel reload"),
        "node_modules change triggered reload: {}",
        log.lock().expect("log")
    );

    fs::write(
        dir.join("src/index.js"),
        "export default { async fetch() { return new Response(\"v2\"); } };\n",
    )
    .unwrap();
    wait_log(&log, "tysel reload", Duration::from_secs(5));
    assert!(http_get(&addr).contains("v2"), "reloaded body");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_reloads_secrets_when_dotenv_changes() {
    let dir = temp_app("dev-secrets");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"

[permissions]
secrets = ["API_KEY"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() {
    try {
      return new Response(await tysel.secrets.ref("API_KEY"));
    } catch (err) {
      return new Response(String(err), { status: 500 });
    }
  },
};
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let log = capture_output(stderr);
    let missing = http_get(&addr);
    assert!(missing.contains("500"), "{missing}");
    assert!(missing.contains("unknown secret API_KEY"), "{missing}");

    log.lock().expect("log").clear();
    fs::write(dir.join(".env"), "API_KEY=sk-reload-test\n").unwrap();
    wait_log(&log, "tysel reload", Duration::from_secs(5));
    let loaded = http_get(&addr);
    assert!(loaded.contains("200"), "{loaded}");
    assert!(loaded.contains("secret:API_KEY"), "{loaded}");
    assert!(!loaded.contains("sk-reload-test"), "{loaded}");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_fetch_allowlist_denies_unlisted_hosts() {
    let dir = temp_app("dev-fetch-deny");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"

[limits]
request_timeout_ms = 2000

[permissions]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() {
    try {
      await fetch("http://192.0.2.1/");
      return new Response("allowed");
    } catch (err) {
      return new Response(String(err), { status: 403 });
    }
  },
};
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let started = std::time::Instant::now();
    let body = http_get(&addr);
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(2), "deny took {elapsed:?}");
    assert!(body.contains("403"), "{body}");
    assert!(body.contains("192.0.2.1"), "{body}");
    assert!(body.contains("not permitted"), "{body}");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn runtime_failures_use_the_structured_json_envelope() {
    let dir = temp_app("runtime-json-error");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "source-map-error"
entry = "src/index.ts"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.ts"),
        r#"type Failure = { message: string };
const failure: Failure = { message: "intentional failure" };
export default {
  async fetch() {
    throw new Error(failure.message);
  },
};
"#,
    )
    .unwrap();
    let mut child = Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel run for source-map error");
    let addr = wait_listen(child.stdout.take().expect("stdout"), Duration::from_secs(8));
    let response = http_get(&addr);
    let _ = child.kill();
    let _ = child.wait();

    assert!(response.contains("500 Internal Server Error"), "{response}");
    assert!(response.contains("content-type: application/json"), "{response}");
    let (head, encoded) = response.split_once("\r\n\r\n").expect("response body");
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        decode_chunked(encoded)
    } else {
        encoded.to_owned()
    };
    let error: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(error["error"]["code"], "RUNTIME_ERROR");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("intentional failure"), "{message}");
    assert!(message.contains("src/index.ts:5"), "{message}");
    assert!(!message.contains("app.js:"), "{message}");
    assert!(error["error"]["requestId"].as_str().is_some_and(|id| id.len() == 16));
}

#[test]
fn dev_isolated_profile_denies_fetch_even_when_hosts_are_listed() {
    let dir = temp_app("dev-isolated-fetch");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "isolated"

[server]
listen = "127.0.0.1:0"

[limits]
request_timeout_ms = 2000

[permissions]
fetch = ["192.0.2.1"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() {
    try {
      await fetch("http://192.0.2.1/");
      return new Response("allowed");
    } catch (err) {
      return new Response(String(err), { status: 403 });
    }
  },
};
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .env("TYSEL_WORKER", ensure_worker())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let log = capture_output(child.stderr.take().expect("stderr"));
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let started = std::time::Instant::now();
    let body = http_get(&addr);
    let elapsed = started.elapsed();
    wait_log(&log, "\"capability\":\"fetch\"", Duration::from_secs(2));
    wait_log(&log, "\"method\":\"GET\"", Duration::from_secs(2));
    assert!(elapsed < Duration::from_secs(2), "deny took {elapsed:?}");
    assert!(body.contains("403"), "{body}");
    assert!(body.contains("isolated profile"), "{body}");
    assert!(!body.contains("not permitted"), "{body}");
    let captured = log.lock().expect("log").clone();
    assert!(captured.contains("\"capability\":\"fetch\""), "{captured}");
    assert!(captured.contains("\"operation\":\"request\""), "{captured}");
    assert!(captured.contains("\"result\":\"denied\""), "{captured}");
    assert_matching_rid(&captured, "fetch");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_isolated_profile_does_not_inherit_supervisor_env() {
    let dir = temp_app("dev-isolated-env");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "isolated"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() {
    return new Response("ENV:" + tysel.envKeys() + ":END");
  },
};
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .env("TYSEL_WORKER", ensure_worker())
        .env("TYSEL_TEST_SECRET", "should-not-leak")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let body = http_get(&addr);
    let start = body.find("ENV:").expect("env marker");
    let rest = &body[start + 4..];
    let keys = rest.split(":END").next().unwrap_or(rest);
    for leaked in ["HOME", "USER", "PATH", "TYSEL_TEST_SECRET"] {
        assert!(!keys.split(',').any(|key| key == leaked), "worker inherited {leaked}: {keys}");
    }

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_fetch_expands_secret_handles_on_the_host() {
    let (origin, seen) = spawn_header_echo();
    let dir = temp_app("dev-fetch-secret");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"

[permissions]
fetch = ["127.0.0.1"]
secrets = ["API_KEY"]
"#,
    )
    .unwrap();
    fs::write(dir.join(".env"), "API_KEY=sk-host-only\n").unwrap();
    fs::write(
        dir.join("src/index.js"),
        format!(
            r#"export default {{
  async fetch() {{
    const token = await tysel.secrets.ref("API_KEY");
    const res = await fetch("http://{origin}/", {{
      headers: {{ Authorization: "Bearer " + token }},
    }});
    return new Response(await res.text());
  }},
}};
"#
        ),
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let body = http_get(&addr);
    assert!(body.contains("200"), "{body}");
    assert!(body.contains("ok"), "{body}");
    assert!(!body.contains("sk-host-only"), "{body}");
    assert_eq!(seen.lock().expect("seen").as_str(), "Bearer sk-host-only");

    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn dev_fs_read_and_write_stay_inside_allowlist() {
    let dir = temp_app("dev-fs");
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("data")).unwrap();
    fs::write(dir.join("data/hello.txt"), "hi").unwrap();
    fs::write(dir.join("secret.txt"), "no").unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"

[permissions]
fs_read = ["./data"]
fs_write = ["./data"]
"#,
    )
    .unwrap();
    fs::write(
        dir.join("src/index.js"),
        r#"export default {
  async fetch() {
    try {
      await tysel.fs.write("data/out.txt", "ok");
      const hello = await tysel.fs.read("data/hello.txt");
      try {
        await tysel.fs.read("secret.txt");
        return new Response("escaped", { status: 500 });
      } catch (err) {
        return Response.json({ hello, denied: String(err) });
      }
    } catch (err) {
      return new Response(String(err), { status: 500 });
    }
  },
};
"#,
    )
    .unwrap();

    let mut child = Command::new(cli_exe())
        .args(["dev", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn tysel dev");
    let stdout = child.stdout.take().expect("stdout");
    let log = capture_output(child.stderr.take().expect("stderr"));
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let body = http_get(&addr);
    wait_log(&log, "\"capability\":\"fs\"", Duration::from_secs(2));
    let _ = child.kill();
    let _ = child.wait();
    assert!(body.contains("200"), "{body}");
    assert!(body.contains("\"hello\":\"hi\"") || body.contains("\"hello\": \"hi\""), "{body}");
    assert!(body.contains("not permitted"), "{body}");
    assert_eq!(fs::read_to_string(dir.join("data/out.txt")).unwrap(), "ok");
    assert_eq!(fs::read_to_string(dir.join("secret.txt")).unwrap(), "no");
    let captured = log.lock().expect("log").clone();
    assert!(captured.contains("\"capability\":\"fs\""), "{captured}");
    assert!(captured.contains("\"operation\":\"read\""), "{captured}");
    assert!(captured.contains("\"operation\":\"write\""), "{captured}");
    assert!(captured.contains("\"result\":\"ok\""), "{captured}");
    assert!(captured.contains("\"result\":\"error\""), "{captured}");
    assert!(!captured.contains("secret.txt"), "{captured}");
    assert_matching_rid(&captured, "fs");
}

fn spawn_header_echo() -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind origin");
    let addr = listener.local_addr().expect("local addr");
    let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured = seen.clone();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            if let Some(line) =
                req.lines().find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
            {
                *captured.lock().expect("seen") = line
                    .split_once(':')
                    .map(|(_, value)| value.trim().to_owned())
                    .unwrap_or_default();
            }
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
        }
    });
    (format!("127.0.0.1:{}", addr.port()), seen)
}

fn spawn_fake_llm() -> (String, Arc<AtomicUsize>, Arc<AtomicBool>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind fake LLM");
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let server_calls = calls.clone();
    let server_stop = stop.clone();
    thread::spawn(move || {
        while !server_stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let mut request = [0u8; 8192];
                    let _ = stream.read(&mut request);
                    server_calls.fetch_add(1, Ordering::SeqCst);
                    let body = r#"{"id":"demo-llm-1","output_text":"Account looks healthy","usage":{"input_tokens":4,"output_tokens":3}}"#;
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes());
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    (format!("127.0.0.1:{}", addr.port()), calls, stop)
}

fn spawn_durable_agent(dir: &std::path::Path, llm_addr: &str) -> std::process::Child {
    Command::new(cli_exe())
        .args(["run", "--manifest", dir.join("tysel.toml").to_str().unwrap()])
        .env("TYSEL_LLM_ENDPOINT", format!("http://{llm_addr}/v1/responses"))
        .env("TYSEL_LLM_MODEL", "demo-model")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn durable agent")
}

fn http_json(addr: &str, method: &str, path: &str, body: Option<&str>) -> serde_json::Value {
    let body = body.unwrap_or("");
    let mut stream = TcpStream::connect(addr).expect("connect");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").expect("HTTP response");
    assert!(head.contains(" 200 ") || head.contains(" 202 "), "{response}");
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        decode_chunked(body)
    } else {
        body.to_owned()
    };
    serde_json::from_str(&body).unwrap_or_else(|error| panic!("invalid JSON {error}: {response}"))
}

fn decode_chunked(mut encoded: &str) -> String {
    let mut decoded = Vec::new();
    loop {
        let (size, rest) = encoded.split_once("\r\n").expect("chunk size");
        let size = usize::from_str_radix(size.trim(), 16).expect("hex chunk size");
        if size == 0 {
            break;
        }
        let bytes = rest.as_bytes();
        assert!(bytes.len() >= size + 2, "truncated HTTP chunk");
        decoded.extend_from_slice(&bytes[..size]);
        encoded = std::str::from_utf8(&bytes[size + 2..]).expect("chunked UTF-8");
    }
    String::from_utf8(decoded).expect("HTTP JSON UTF-8")
}

fn http_get(addr: &str) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect");
    stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut body = String::new();
    stream.read_to_string(&mut body).unwrap();
    body
}

fn capture_output(
    mut reader: impl Read + Send + 'static,
) -> std::sync::Arc<std::sync::Mutex<String>> {
    let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let captured = log.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            let n = reader.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            captured.lock().expect("log").push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    });
    log
}

fn wait_log(log: &std::sync::Arc<std::sync::Mutex<String>>, needle: &str, timeout: Duration) {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if log.lock().expect("log").contains(needle) {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {needle:?}: {}", log.lock().expect("log"));
}

fn json_rid(line: &str) -> Option<u64> {
    let idx = line.find("\"rid\":")?;
    let rest = line[idx + 6..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn assert_matching_rid(captured: &str, capability: &str) {
    let cap_needle = format!("\"capability\":\"{capability}\"");
    let mut http_rids = Vec::new();
    let mut cap_rids = Vec::new();
    for line in captured.lines() {
        if line.contains("\"method\"") && line.contains("\"path\"") {
            http_rids.push(json_rid(line));
        }
        if line.contains(&cap_needle) {
            cap_rids.push(json_rid(line));
        }
    }
    assert_eq!(http_rids.len(), 1, "expected one HTTP log line: {captured}");
    let rid = http_rids[0].expect("HTTP rid");
    assert!(rid > 0, "HTTP rid should be nonzero: {captured}");
    assert!(!cap_rids.is_empty(), "expected {capability} log lines: {captured}");
    assert!(
        cap_rids.iter().all(|value| *value == Some(rid)),
        "{capability} rids {cap_rids:?} did not match HTTP rid {rid}: {captured}"
    );
}

fn wait_listen(stdout: impl Read + Send + 'static, timeout: Duration) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut reader = stdout;
        let mut chunk = [0u8; 256];
        loop {
            let n = reader.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                let _ = tx.send(Err("eof"));
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(line) = std::str::from_utf8(&buf)
                .ok()
                .and_then(|text| text.lines().find_map(|line| line.strip_prefix("tysel listen ")))
            {
                let _ = tx.send(Ok(line.trim().to_owned()));
                return;
            }
        }
    });
    rx.recv_timeout(timeout).expect("listen").expect("addr")
}

fn write_js_app(dir: &std::path::Path, source: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("tysel.toml"),
        r#"
[app]
name = "hello-service"
entry = "src/index.js"
profile = "service"

[server]
listen = "127.0.0.1:0"
"#,
    )
    .unwrap();
    fs::write(dir.join("src/index.js"), source).unwrap();
}

fn temp_app(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tysel-cli-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn echo_component() -> Vec<u8> {
    wat::parse_str(
        r#"
(component
  (core module $module
    (memory (export "memory") 1)
    (global $heap (mut i32) (i32.const 16))
    (func (export "realloc")
      (param i32 i32 i32) (param $new-len i32) (result i32)
      (local $ptr i32)
      global.get $heap
      local.tee $ptr
      local.get $new-len
      i32.add
      global.set $heap
      local.get $ptr)
    (func (export "run") (param $ptr i32) (param $len i32) (result i32)
      i32.const 0
      i32.const 0
      i32.store
      i32.const 4
      local.get $ptr
      i32.store
      i32.const 8
      local.get $len
      i32.store
      i32.const 0))
  (core instance $instance (instantiate $module))
  (alias core export $instance "memory" (core memory $memory))
  (alias core export $instance "realloc" (core func $realloc))
  (alias core export $instance "run" (core func $run-core))
  (type $run-type
    (func (param "input" string) (result (result string (error string)))))
  (func $run (type $run-type)
    (canon lift (core func $run-core) (memory $memory) (realloc $realloc)))
  (export "run" (func $run)))
"#,
    )
    .unwrap()
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

fn ensure_worker() -> PathBuf {
    let worker = worker_exe_candidate();
    if worker.is_file() {
        return worker;
    }
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "tysel-isolate", "--bin", "tysel-worker", "--quiet"])
        .status()
        .expect("cargo build tysel-worker");
    assert!(status.success(), "failed to build tysel-worker");
    assert!(worker.is_file(), "missing tysel-worker at {}", worker.display());
    worker
}

fn worker_exe_candidate() -> PathBuf {
    for key in ["CARGO_BIN_EXE_tysel_worker", "CARGO_BIN_EXE_tysel-worker"] {
        if let Some(path) = std::env::var_os(key) {
            return PathBuf::from(path);
        }
    }
    let test_exe = std::env::current_exe().expect("current_exe");
    let mut candidate = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("tysel-worker"))
        .expect("target debug directory");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    candidate
}
