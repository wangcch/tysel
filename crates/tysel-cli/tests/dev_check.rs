use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

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
    let started = std::time::Instant::now();
    let addr = wait_listen(stdout, Duration::from_secs(8));
    let body = http_get(&addr);
    assert!(started.elapsed() < Duration::from_secs(2), "deny took {:?}", started.elapsed());
    assert!(body.contains("403"), "{body}");
    assert!(body.contains("192.0.2.1"), "{body}");
    assert!(body.contains("not permitted"), "{body}");

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
