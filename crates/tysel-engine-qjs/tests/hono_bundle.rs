use std::fs;
use std::path::Path;

use tysel_engine::{HttpRequest, IsolateConfig};
use tysel_engine_qjs::IsolatePool;

fn config() -> IsolateConfig {
    IsolateConfig {
        request_timeout_ms: 2_000,
        cpu_ms_per_turn: 50,
        memory_limit_bytes: 16 * 1024 * 1024,
    }
}

async fn body_text(mut body: tokio::sync::mpsc::Receiver<Vec<u8>>) -> String {
    let mut bytes = Vec::new();
    while let Some(chunk) = body.recv().await {
        bytes.extend(chunk);
    }
    String::from_utf8(bytes).expect("utf8 body")
}

#[tokio::test]
async fn bundled_hono_app_handles_json_routes() {
    let dir = std::env::temp_dir().join(format!("tysel-qjs-hono-{}", std::process::id()));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::create_dir_all(dir.join("node_modules/hono")).unwrap();
    fs::write(
        dir.join("node_modules/hono/package.json"),
        r#"{"name":"hono","type":"module","exports":{".":"./index.js"}}"#,
    )
    .unwrap();
    fs::write(
        dir.join("node_modules/hono/index.js"),
        r#"
export class Hono {
  constructor() {
    this.routes = [];
  }
  get(path, handler) {
    this.routes.push(["GET", path, handler]);
    return this;
  }
  fetch(request) {
    const url = new URL(request.url);
    for (const [method, path, handler] of this.routes) {
      if (method !== request.method) continue;
      const params = matchRoute(path, url.pathname);
      if (!params) continue;
      return handler({
        json: (data, status) => Response.json(data, { status: status || 200 }),
        req: { param: (key) => params[key] },
      });
    }
    return new Response("not found", { status: 404 });
  }
}
function matchRoute(pattern, pathname) {
  if (pattern === pathname) return {};
  const patternParts = pattern.split("/").filter(Boolean);
  const pathParts = pathname.split("/").filter(Boolean);
  if (patternParts.length !== pathParts.length) return null;
  const params = {};
  for (let i = 0; i < patternParts.length; i++) {
    if (patternParts[i].startsWith(":")) params[patternParts[i].slice(1)] = pathParts[i];
    else if (patternParts[i] !== pathParts[i]) return null;
  }
  return params;
}
"#,
    )
    .unwrap();
    let entry = dir.join("src/index.ts");
    fs::write(
        &entry,
        r#"
import { Hono } from "hono";

const app = new Hono();
app.get("/", (c) => c.json({ ok: true }));
app.get("/hello/:name", (c) => c.json({ hello: c.req.param("name") }));
export default app;
"#,
    )
    .unwrap();

    let (bundle, _) = tysel_build::read_bundle(&entry).expect("bundle hono fixture");
    let source = String::from_utf8(bundle).expect("utf8 bundle");
    let pool = IsolatePool::spawn(1, &source, config()).expect("spawn isolate");

    let (head, body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch /");
    assert_eq!(head.status, 200);
    assert!(body_text(body).await.contains("\"ok\":true"));

    let (head, body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/hello/tysel".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch /hello/:name");
    assert_eq!(head.status, 200);
    assert!(body_text(body).await.contains("\"hello\":\"tysel\""));
}

#[tokio::test]
async fn real_hono_example_handles_json_routes() {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hono-api");
    let entry = app.join("src/index.ts");
    if !app.join("node_modules/hono").is_dir() {
        eprintln!("skipping real Hono example; run pnpm install");
        return;
    }
    let (bundle, _) = tysel_build::read_bundle(&entry).expect("bundle examples/hono-api");
    let source = String::from_utf8(bundle).expect("utf8 bundle");
    assert!(source.contains("__tysel_require"), "expected a multi-file bundle");
    assert!(!source.contains("from \"hono\""));

    let pool = IsolatePool::spawn(
        1,
        &source,
        IsolateConfig { memory_limit_bytes: 32 * 1024 * 1024, ..config() },
    )
    .expect("spawn real Hono isolate");

    let (head, body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch /");
    assert_eq!(head.status, 200, "GET / status");
    let text = body_text(body).await;
    assert!(text.contains("\"ok\":true"), "GET / body was {text}");

    let (head, body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "http://tysel.local/hello/tysel".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .expect("dispatch /hello/:name");
    assert_eq!(head.status, 200, "GET /hello/:name status");
    let text = body_text(body).await;
    assert!(text.contains("\"hello\":\"tysel\""), "GET /hello/:name body was {text}");
}

#[tokio::test]
async fn real_hono_preserves_url_path_and_query_boundaries() {
    let app = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hono-api");
    assert!(app.join("node_modules/hono").is_dir(), "run pnpm install for real Hono acceptance");
    let dir = std::env::temp_dir().join(format!("tysel-hono-url-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let modules = dir.join("node_modules");
    if !modules.exists() {
        std::os::unix::fs::symlink(app.join("node_modules"), &modules).unwrap();
    }
    let entry = dir.join("index.ts");
    fs::write(
        &entry,
        r#"
      import { Hono } from 'hono';
      const app=new Hono();
      app.get('/a//b',c=>{
        const url=new URL(c.req.url);
        return c.json({path:url.pathname,port:url.port,q:url.searchParams.get('q')});
      });
      app.get('/a/b',c=>c.text('distinct route'));
      export default app;
    "#,
    )
    .unwrap();
    let (bundle, _) = tysel_build::read_bundle(&entry).unwrap();
    let pool = IsolatePool::spawn(1, &String::from_utf8(bundle).unwrap(), config()).unwrap();
    let (head, body) = pool
        .dispatch(HttpRequest {
            method: "GET".into(),
            url: "https://tysel.local:443/a//b?q=%E4%B8%AD+%FF".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        })
        .await
        .unwrap();
    assert_eq!(head.status, 200);
    let data: serde_json::Value = serde_json::from_str(&body_text(body).await).unwrap();
    assert_eq!(data, serde_json::json!({"path":"/a//b","port":"","q":"中 �"}));
    fs::remove_dir_all(dir).unwrap();
}
