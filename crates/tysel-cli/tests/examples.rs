#![cfg(unix)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

mod support;

use support::process::{ManagedChild, wait_listen};

const MCP_VERSION: &str = "2026-07-28";
const RAW_SECRET: &str = "sk-example-must-not-leak";

#[test]
fn isolated_plugin_enforces_profile_and_recovers() {
    let manifest = example_manifest("isolated-plugin");
    let mut child = ManagedChild::spawn(
        Command::new(cli_exe())
            .args(["run", "--manifest", manifest.to_str().unwrap()])
            .env("TYSEL_WORKER", ensure_worker())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        "isolated plugin example",
    );
    let (addr, log) = wait_listen(&mut child, Duration::from_secs(8));

    let (status, root) = http_json(&addr, "/");
    assert_eq!(status, 200);
    assert_eq!(root["isolated"], true);
    assert_eq!(root["plugin"], "echo");

    for (path, capability) in [("/probe/fetch", "fetch"), ("/probe/filesystem", "filesystem")] {
        let (status, result) = http_json(&addr, path);
        assert_eq!(status, 403, "{path}: {result}");
        assert_eq!(result["capability"], capability);
        assert_eq!(result["denied"], true);
        let error = result["error"].as_str().expect("denial error");
        assert!(
            error.contains("isolated profile") || error.contains("isolated worker"),
            "{path}: {error}"
        );
    }

    // A trusted caller holds the full record and projects only the fields the
    // plugin needs. No credential or authority crosses this request boundary.
    let record = serde_json::json!({
        "customerId": "customer-42", "name": "Acme", "openTickets": 3,
        "email": "private@example.test", "apiToken": RAW_SECRET,
    });
    let snapshot = serde_json::json!({
        "customerId": record["customerId"], "name": record["name"],
        "openTickets": record["openTickets"],
    })
    .to_string();
    assert!(!snapshot.contains(RAW_SECRET));
    assert!(!snapshot.contains("private@example.test"));
    let (status, summary) = http_request_json(&addr, "POST", "/summarize", &snapshot);
    assert_eq!(status, 200, "{summary}");
    assert_eq!(
        summary,
        serde_json::json!({
            "customerId": "customer-42",
            "summary": "Acme has 3 open support tickets.",
            "needsAttention": true,
        })
    );
    // An ID alone cannot cause the isolated plugin to look up host data.
    for invalid in [
        r#"{"customerId":"customer-42"}"#.to_owned(),
        "null".to_owned(),
        "{".to_owned(),
        r#"{"customerId":"c","name":"Acme","openTickets":-1}"#.to_owned(),
        r#"{"customerId":"c","name":"Acme","openTickets":1.5}"#.to_owned(),
        r#"{"customerId":"c","name":"Acme","openTickets":10001}"#.to_owned(),
        r#"{"customerId":"c","name":"Acme","openTickets":3,"extra":true}"#.to_owned(),
    ] {
        let (status, error) = http_request_json(&addr, "POST", "/summarize", &invalid);
        assert_eq!(status, 400, "{error}");
        assert!(!error.to_string().contains(RAW_SECRET));
    }
    let (status, _) = http_json(&addr, "/summarize");
    assert_eq!(status, 405);

    // The CLI owns one long-lived HTTP worker after task inspection has settled.
    let original = wait_for_worker(child.id(), None, Duration::from_secs(5));
    let status = Command::new("kill")
        .args(["-KILL", &original.to_string()])
        .status()
        .expect("kill isolated worker");
    assert!(status.success());
    // Signal delivery is asynchronous. A request racing worker exit can fail
    // with an uncertain outcome and must not be transparently replayed. This
    // test checks recovery after confirmed exit; supervisor tests cover the race.
    wait_for_worker_exit(original, Duration::from_secs(5));

    let (status, recovered) = http_json(&addr, "/");
    assert_eq!(
        status,
        200,
        "worker did not recover: {recovered}; logs: {}",
        log.lock().expect("captured service log")
    );
    assert_eq!(recovered["plugin"], "echo");
    let replacement = wait_for_worker(child.id(), Some(original), Duration::from_secs(5));
    assert_ne!(replacement, original);
    let (status, after_restart) = http_request_json(&addr, "POST", "/summarize", &snapshot);
    assert_eq!(status, 200);
    assert_eq!(after_restart, summary);
    let (status, denied) = http_json(&addr, "/probe/fetch");
    assert_eq!(status, 403);
    assert_eq!(denied["denied"], true);
}

// Trusted test caller: authorization comes from its own customer scope, never
// from a customer ID, URL, or grant supplied by the plugin.
struct TicketCaller {
    remaining: usize,
    reads: usize,
}

impl TicketCaller {
    fn lookup(&mut self, request: &serde_json::Value) -> Result<serde_json::Value, &'static str> {
        let fields = request.as_object().ok_or("invalid operation")?;
        if fields.len() != 3 || request["kind"] != "lookup" || request["operation"] != "ticket.read"
        {
            return Err("invalid operation");
        }
        let id = request["ticketId"].as_str().ok_or("invalid ticket")?;
        // These two records are the complete authorized scope for customer-42.
        let subject = match id {
            "ticket-low" => "Update contact details",
            "ticket-urgent" => "Checkout unavailable",
            _ => return Err("ticket outside customer scope"),
        };
        if self.remaining == 0 {
            return Err("lookup budget exhausted");
        }
        self.remaining -= 1;
        self.reads += 1;
        // The real adapter would access its data source here. Only the selected
        // fields are returned; credentials and private notes stay with caller.
        Ok(serde_json::json!({"ticketId": id, "subject": subject}))
    }
}

#[test]
fn isolated_ticket_caller_rejects_untrusted_operations() {
    let mut caller = TicketCaller { remaining: 1, reads: 0 };
    for forged in [
        serde_json::json!({"kind":"lookup","operation":"ticket.delete","ticketId":"ticket-urgent"}),
        serde_json::json!({"kind":"lookup","operation":"ticket.read","ticketId":"other-customer-ticket"}),
        serde_json::json!({"kind":"lookup","operation":"ticket.read","ticketId":"ticket-urgent","url":"https://example.test"}),
        serde_json::json!({"kind":"lookup","operation":"ticket.read","ticketId":null}),
    ] {
        assert!(caller.lookup(&forged).is_err());
    }
    assert_eq!(caller.reads, 0, "rejection must precede data access");
    let valid =
        serde_json::json!({"kind":"lookup","operation":"ticket.read","ticketId":"ticket-urgent"});
    assert!(caller.lookup(&valid).is_ok());
    assert_eq!(caller.lookup(&valid), Err("lookup budget exhausted"));
    assert_eq!(caller.reads, 1);
}

#[test]
fn isolated_plugin_requests_scoped_data_across_worker_restart() {
    let manifest = example_manifest("isolated-plugin");
    let mut child = ManagedChild::spawn(
        Command::new(cli_exe())
            .args(["run", "--manifest", manifest.to_str().unwrap()])
            .env("TYSEL_WORKER", ensure_worker())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
        "isolated triage experiment",
    );
    let (addr, _log) = wait_listen(&mut child, Duration::from_secs(8));
    let mut snapshot = serde_json::json!({"customerId":"customer-42", "tickets":[
        {"id":"ticket-low","priority":0}, {"id":"ticket-urgent","priority":3}
    ]});
    let (status, operation) = http_request_json(&addr, "POST", "/triage", &snapshot.to_string());
    assert_eq!(status, 200);
    assert_eq!(
        operation,
        serde_json::json!({"kind":"lookup","operation":"ticket.read","ticketId":"ticket-urgent"})
    );
    let mut caller = TicketCaller { remaining: 1, reads: 0 };
    snapshot["detail"] = caller.lookup(&operation).unwrap();
    let original = wait_for_worker(child.id(), None, Duration::from_secs(5));
    assert!(
        Command::new("kill").args(["-KILL", &original.to_string()]).status().unwrap().success()
    );
    wait_for_worker_exit(original, Duration::from_secs(5));
    let (status, done) = http_request_json(&addr, "POST", "/triage", &snapshot.to_string());
    assert_eq!(status, 200);
    // The caller accepts only a final result at this stage, never another lookup.
    assert_eq!(
        done,
        serde_json::json!({"kind":"done","customerId":"customer-42", "summary":"Prioritize ticket-urgent: Checkout unavailable"})
    );
    assert_eq!(caller.reads, 1);
    assert_ne!(wait_for_worker(child.id(), Some(original), Duration::from_secs(5)), original);
    let (status, denied) = http_json(&addr, "/probe/fetch");
    assert_eq!(status, 403);
    assert_eq!(denied["denied"], true);

    snapshot["detail"]["ticketId"] = serde_json::json!("ticket-low");
    assert_eq!(http_request_json(&addr, "POST", "/triage", &snapshot.to_string()).0, 400);
    let empty = r#"{"customerId":"customer-42","tickets":[]}"#;
    let (status, done) = http_request_json(&addr, "POST", "/triage", empty);
    assert_eq!(status, 200);
    assert_eq!(done["kind"], "done");
}

#[test]
fn mcp_tool_covers_stdio_contract_and_opaque_secrets() {
    let manifest = example_manifest("mcp-tool");
    let mut child = Command::new(cli_exe())
        .args(["mcp", "--manifest", manifest.to_str().unwrap()])
        .env("TYSEL_WORKER", ensure_worker())
        .env("OPENAI_API_KEY", RAW_SECRET)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn MCP example");

    let meta = format!(r#""_meta":{{"io.modelcontextprotocol/protocolVersion":"{MCP_VERSION}"}}"#);
    let requests = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"server/discover\",\"params\":{{{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{{{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"lookup\",\"arguments\":{{\"customerId\":\"customer-42\"}},{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{{\"name\":\"lookup\",\"arguments\":{{\"customerId\":42}},{meta}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{{\"name\":\"missing\",\"arguments\":{{}},{meta}}}}}\n"
    );
    child.stdin.take().unwrap().write_all(requests.as_bytes()).unwrap();
    let output = child.wait_with_output().expect("wait for MCP example");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains(RAW_SECRET), "raw secret leaked: {stdout}");
    assert!(!stderr.contains(RAW_SECRET), "raw secret leaked to stderr: {stderr}");
    let responses: Vec<serde_json::Value> =
        stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert_eq!(responses.len(), 5, "{stdout}");

    assert_eq!(responses[0]["result"]["supportedVersions"][0], MCP_VERSION);
    let tool = &responses[1]["result"]["tools"][0];
    assert_eq!(tool["name"], "lookup");
    assert_eq!(tool["inputSchema"]["required"][0], "customerId");
    assert_eq!(tool["inputSchema"]["properties"]["customerId"]["type"], "string");

    let result = &responses[2]["result"]["structuredContent"];
    assert_eq!(result["customerId"], "customer-42");
    assert_eq!(result["isolated"], true);
    assert_eq!(result["secret"], "secret:OPENAI_API_KEY");

    assert_eq!(responses[3]["result"]["isError"], true);
    assert!(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .is_some_and(|text| text.contains("customerId") && text.contains("string"))
    );
    assert_eq!(responses[4]["error"]["code"], -32602);
    assert!(
        responses[4]["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("Unknown tool"))
    );
}

fn example_manifest(name: &str) -> PathBuf {
    workspace_root().join("examples").join(name).join("tysel.toml")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

fn http_json(addr: &str, path: &str) -> (u16, serde_json::Value) {
    http_request_json(addr, "GET", path, "")
}

fn http_request_json(addr: &str, method: &str, path: &str, body: &str) -> (u16, serde_json::Value) {
    let mut stream = TcpStream::connect(addr).expect("connect to example");
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, encoded) = response.split_once("\r\n\r\n").expect("HTTP response");
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .expect("HTTP status");
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") {
        decode_chunked(encoded)
    } else {
        encoded.to_owned()
    };
    let value = serde_json::from_str(&body)
        .unwrap_or_else(|error| panic!("invalid JSON response {error}: {response}"));
    (status, value)
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

fn wait_for_worker(parent: u32, exclude: Option<u32>, timeout: Duration) -> u32 {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if let Some(pid) = worker_children(parent).into_iter().find(|pid| Some(*pid) != exclude) {
            return pid;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for tysel-worker child of {parent}");
}

fn wait_for_worker_exit(pid: u32, timeout: Duration) {
    let started = Instant::now();
    loop {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "stat="])
            .output()
            .expect("inspect killed worker");
        assert!(
            output.status.success() || output.status.code() == Some(1),
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let state = String::from_utf8_lossy(&output.stdout);
        // A zombie has exited and can be reaped by the supervisor's try_wait.
        if state.trim().is_empty() || state.trim_start().starts_with('Z') {
            return;
        }
        assert!(started.elapsed() < timeout, "worker {pid} did not exit: {state}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn worker_children(parent: u32) -> Vec<u32> {
    let output =
        Command::new("ps").args(["-axo", "pid=,ppid=,comm="]).output().expect("list processes");
    assert!(output.status.success(), "ps failed");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let ppid: u32 = fields.next()?.parse().ok()?;
            let command = fields.next()?;
            (ppid == parent && command.contains("tysel-worker")).then_some(pid)
        })
        .collect()
}

fn cli_exe() -> PathBuf {
    executable("CARGO_BIN_EXE_tysel", "tysel")
}

fn ensure_worker() -> PathBuf {
    if let Some(path) = executable_if_present("CARGO_BIN_EXE_tysel_worker", "tysel-worker") {
        return path;
    }
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "tysel-isolate", "--bin", "tysel-worker"])
        .status()
        .expect("build tysel-worker");
    assert!(status.success(), "failed to build tysel-worker");
    executable("CARGO_BIN_EXE_tysel_worker", "tysel-worker")
}

fn executable(key: &str, name: &str) -> PathBuf {
    executable_if_present(key, name).unwrap_or_else(|| panic!("missing {name} executable"))
}

fn executable_if_present(key: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(key) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    let test_exe = std::env::current_exe().ok()?;
    let mut candidate = test_exe.parent()?.parent()?.join(name);
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    candidate.is_file().then_some(candidate)
}
