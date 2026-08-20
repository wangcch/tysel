use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use http_body_util::{BodyExt, Empty};
use hyper::Request;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tysel_package::{PackageManifest, PackagedComponent, Tap, identity_source_map};

const HANDLER: &str = r#"
export default {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    return Response.json({
      message: "Hello from Tysel",
      path,
      packaged: true,
    });
  },
};
"#;

const TYPESCRIPT: &str = r#"
export default {
  async fetch(request: Request): Promise<Response> {
    const path = new URL(request.url).pathname;
    return Response.json({ message: "Hello from Tysel", path, packaged: true });
  },
};
"#;

#[tokio::test]
async fn unpackaged_stub_exits_without_payload() {
    let output = Command::new(stub_exe()).output().await.expect("run stub");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no tap payload"), "stderr was {stderr}");
}

#[tokio::test]
async fn packaged_stub_serves_embedded_bundle() {
    let packaged = package_stub();
    let mut child = Command::new(&packaged)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn packaged stub");
    let stdout = child.stdout.take().expect("stdout");
    let stderr = collect_stderr(child.stderr.take().expect("stderr"));
    let addr = tokio::time::timeout(Duration::from_secs(5), read_listen(stdout))
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for listen; stderr={}", stderr_text(&stderr)))
        .unwrap_or_else(|err| panic!("{err}; stderr={}", stderr_text(&stderr)));

    let (status, body) = request(addr, "/hello").await;
    assert_eq!(status, 200);
    assert!(body.contains("Hello from Tysel"));
    assert!(body.contains("\"path\":\"/hello\""));
    assert!(body.contains("\"packaged\":true"));
}

#[tokio::test]
async fn packaged_stub_invokes_embedded_component_over_stdio() {
    let packaged = package_component_stub();
    let mut child = Command::new(&packaged)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn packaged Component stub");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(br#"{"value":42}"#).await.unwrap();
    stdin.shutdown().await.unwrap();
    drop(stdin);
    let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
        .await
        .expect("Component execution timed out")
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "{\"value\":42}\n");
}

fn collect_stderr(mut stderr: tokio::process::ChildStderr) -> Arc<Mutex<Vec<u8>>> {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let out = buf.clone();
    tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        *out.lock().expect("stderr lock") = bytes;
    });
    buf
}

fn stderr_text(buf: &Arc<Mutex<Vec<u8>>>) -> String {
    String::from_utf8_lossy(&buf.lock().expect("stderr lock")).into_owned()
}

async fn read_listen(stdout: tokio::process::ChildStdout) -> std::io::Result<SocketAddr> {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Err(std::io::Error::other("stub exited before listening"));
        }
        if let Some(rest) = line.trim().strip_prefix("tysel listen ") {
            return rest
                .parse()
                .map_err(|err| std::io::Error::other(format!("invalid listen address: {err}")));
        }
    }
}

fn stub_exe() -> PathBuf {
    for key in ["CARGO_BIN_EXE_tysel_service", "CARGO_BIN_EXE_tysel-service"] {
        if let Some(path) = std::env::var_os(key) {
            return PathBuf::from(path);
        }
    }
    let test_exe = std::env::current_exe().expect("current_exe");
    let mut candidate = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("tysel-service"))
        .expect("target debug directory");
    if cfg!(windows) {
        candidate.set_extension("exe");
    }
    assert!(candidate.is_file(), "missing tysel-service at {}", candidate.display());
    candidate
}

fn package_stub() -> PathBuf {
    let stub = std::fs::read(stub_exe()).expect("read stub");
    let map = identity_source_map("src/index.ts", TYPESCRIPT).expect("source map");
    let tap = Tap::new(
        PackageManifest {
            format_version: 0,
            runtime_version: "0.0.1".into(),
            application_id: "hello-service".into(),
            entrypoint: "src/index.ts".into(),
            execution_profile: "service".into(),
            listen: "127.0.0.1:0".into(),
            memory_limit_bytes: 8 * 1024 * 1024,
            cpu_ms_per_turn: 200,
            request_timeout_ms: 2_000,
            bundle_hash: String::new(),
            max_request_bytes: 16 * 1024 * 1024,
            websocket: false,
            sqlite_path: String::new(),
            secret_names: Vec::new(),
            fetch_hosts: Vec::new(),
            postgres: Vec::new(),
            fs_read: Vec::new(),
            fs_write: Vec::new(),
            json_logs: true,
        },
        HANDLER.as_bytes().to_vec(),
        map,
    );
    let extracted_map = tap.parsed_source_map().expect("parse map");
    let origin = extracted_map.original_position(1, 1).expect("map origin");
    assert_eq!(origin.source, "src/index.ts");
    assert!(origin.content.unwrap().contains("Promise<Response>"));

    let dir = std::env::temp_dir().join(format!("tysel-packaged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("hello-service");
    let bytes = tap.embed_into(&stub).expect("embed");
    std::fs::write(&output, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&output).unwrap().permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        std::fs::set_permissions(&output, permissions).unwrap();
    }
    output
}

fn package_component_stub() -> PathBuf {
    let stub = std::fs::read(stub_exe()).expect("read stub");
    let source = wat::parse_str(
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
    .unwrap();
    let tap = Tap::new(
        PackageManifest {
            format_version: 0,
            runtime_version: "0.4.0".into(),
            application_id: "echo-component".into(),
            entrypoint: "echo.wasm".into(),
            execution_profile: "component".into(),
            listen: "127.0.0.1:0".into(),
            memory_limit_bytes: 8 * 1024 * 1024,
            cpu_ms_per_turn: 50,
            request_timeout_ms: 2_000,
            bundle_hash: String::new(),
            max_request_bytes: 1024 * 1024,
            websocket: false,
            sqlite_path: String::new(),
            secret_names: Vec::new(),
            fetch_hosts: Vec::new(),
            postgres: Vec::new(),
            fs_read: Vec::new(),
            fs_write: Vec::new(),
            json_logs: false,
        },
        Vec::new(),
        Vec::new(),
    )
    .with_components(vec![PackagedComponent {
        name: "echo-component".into(),
        abi_version: "0.4.0".into(),
        source,
        aot: Vec::new(),
    }]);
    let dir = std::env::temp_dir().join(format!("tysel-component-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("echo-component");
    std::fs::write(&output, tap.embed_into(&stub).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&output).unwrap().permissions();
        permissions.set_mode(permissions.mode() | 0o755);
        std::fs::set_permissions(&output, permissions).unwrap();
    }
    output
}

async fn request(addr: SocketAddr, path: &str) -> (u16, String) {
    let stream = TcpStream::connect(addr).await.unwrap();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
            .await
            .unwrap();
    tokio::spawn(conn);
    let request = Request::builder()
        .uri(path)
        .header(hyper::header::HOST, "localhost")
        .body(Empty::<Bytes>::new())
        .unwrap();
    let response = sender.send_request(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
