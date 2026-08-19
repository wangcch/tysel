use rquickjs::{Ctx, Function, Module, Object};
use tokio::sync::{mpsc, oneshot};
use tysel_engine::{EngineError, HttpHead, HttpRequest};

use crate::isolate::js_err;

const BOOTSTRAP: &str = r##"
(() => {
  function decodeURIComponentSafe(value) {
    try {
      return decodeURIComponent(String(value).replace(/\+/g, " "));
    } catch {
      return String(value);
    }
  }

  class URLSearchParams {
    constructor(init) {
      this._pairs = [];
      if (init == null) return;
      if (typeof init === "string") {
        const text = init.charAt(0) === "?" ? init.slice(1) : init;
        if (!text) return;
        for (const part of text.split("&")) {
          if (!part) continue;
          const eq = part.indexOf("=");
          if (eq === -1) this._pairs.push([decodeURIComponentSafe(part), ""]);
          else {
            this._pairs.push([
              decodeURIComponentSafe(part.slice(0, eq)),
              decodeURIComponentSafe(part.slice(eq + 1)),
            ]);
          }
        }
      } else if (Array.isArray(init)) {
        for (const pair of init) this.append(pair[0], pair[1]);
      } else {
        for (const key of Object.keys(init)) this.append(key, init[key]);
      }
    }
    append(name, value) {
      this._pairs.push([String(name), String(value)]);
    }
    set(name, value) {
      name = String(name);
      this._pairs = this._pairs.filter((pair) => pair[0] !== name);
      this.append(name, value);
    }
    get(name) {
      name = String(name);
      for (const pair of this._pairs) {
        if (pair[0] === name) return pair[1];
      }
      return null;
    }
    has(name) {
      name = String(name);
      return this._pairs.some((pair) => pair[0] === name);
    }
    toString() {
      return this._pairs
        .map((pair) => encodeURIComponent(pair[0]) + "=" + encodeURIComponent(pair[1]))
        .join("&");
    }
  }

  class Headers {
    constructor(init) {
      this._map = {};
      if (!init) return;
      if (typeof init.forEach === "function") {
        init.forEach((value, key) => this.append(key, value));
        return;
      }
      if (Array.isArray(init)) {
        for (const pair of init) this.append(pair[0], pair[1]);
      } else {
        for (const key of Object.keys(init)) this.append(key, init[key]);
      }
    }
    get(name) {
      const value = this._map[String(name).toLowerCase()];
      return value === undefined ? null : value;
    }
    set(name, value) {
      this._map[String(name).toLowerCase()] = String(value);
    }
    append(name, value) {
      const key = String(name).toLowerCase();
      const prev = this._map[key];
      this._map[key] = prev == null ? String(value) : prev + ", " + String(value);
    }
    has(name) {
      return Object.prototype.hasOwnProperty.call(this._map, String(name).toLowerCase());
    }
    delete(name) {
      delete this._map[String(name).toLowerCase()];
    }
    forEach(callback, thisArg) {
      for (const key of Object.keys(this._map)) {
        callback.call(thisArg, this._map[key], key, this);
      }
    }
    entries() {
      return Object.keys(this._map).map((key) => [key, this._map[key]])[Symbol.iterator]();
    }
    keys() {
      return Object.keys(this._map)[Symbol.iterator]();
    }
    values() {
      return Object.keys(this._map).map((key) => this._map[key])[Symbol.iterator]();
    }
    [Symbol.iterator]() {
      return this.entries();
    }
  }

  class Request {
    constructor(input, init) {
      init = init || {};
      if (typeof input === "string") {
        this.url = input;
        this.method = String(init.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers);
        this.body = init.body == null ? null : init.body;
      } else {
        this.url = input.url;
        this.method = String(init.method || input.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers || input.headers);
        this.body = init.body == null ? input.body : init.body;
      }
    }
    async text() {
      return this.body == null ? "" : String(this.body);
    }
    async json() {
      const text = await this.text();
      return text ? JSON.parse(text) : null;
    }
    clone() {
      return new Request(this.url, { method: this.method, headers: this.headers, body: this.body });
    }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      this.body = body == null ? null : body;
      this.status = init.status || 200;
      this.headers = new Headers(init.headers);
    }
    get ok() {
      return this.status >= 200 && this.status < 300;
    }
    static json(data, init) {
      init = init || {};
      const headers = new Headers(init.headers);
      if (!headers.get("content-type")) {
        headers.set("content-type", "application/json");
      }
      return new Response(JSON.stringify(data), { status: init.status || 200, headers });
    }
    async text() {
      return this.body == null ? "" : String(this.body);
    }
    async json() {
      const text = await this.text();
      return text ? JSON.parse(text) : null;
    }
  }

  class URL {
    constructor(url, base) {
      let href = String(url);
      if (base != null && !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(href)) {
        const baseHref = String(base);
        if (href.charAt(0) === "/") {
          const scheme = baseHref.indexOf("://");
          const hostEnd = scheme === -1 ? 0 : baseHref.indexOf("/", scheme + 3);
          href = (hostEnd === -1 ? baseHref : baseHref.slice(0, hostEnd)) + href;
        } else {
          href = baseHref.replace(/\/[^/]*$/, "/") + href;
        }
      }
      this.href = href;
      const hash = href.indexOf("#");
      const noHash = hash === -1 ? href : href.slice(0, hash);
      const query = noHash.indexOf("?");
      this.search = query === -1 ? "" : noHash.slice(query);
      this.hash = hash === -1 ? "" : href.slice(hash);
      const beforeQuery = query === -1 ? noHash : noHash.slice(0, query);
      const scheme = beforeQuery.indexOf("://");
      this.protocol = scheme === -1 ? "" : beforeQuery.slice(0, scheme + 1);
      const pathStart = scheme === -1 ? 0 : beforeQuery.indexOf("/", scheme + 3);
      this.origin = pathStart === -1 ? beforeQuery : beforeQuery.slice(0, pathStart);
      this.host = this.origin.slice(this.protocol.length + 2);
      this.hostname = this.host.split(":")[0];
      this.pathname = pathStart === -1 ? "/" : beforeQuery.slice(pathStart) || "/";
      this.searchParams = new URLSearchParams(this.search);
    }
    toString() {
      return this.href;
    }
  }

  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;
  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
})();
"##;

pub fn install_web_api(ctx: Ctx<'_>) -> Result<(), EngineError> {
    ctx.eval::<(), _>(BOOTSTRAP).map_err(js_err)
}

const BOOT_FETCH: &str = r#"
import handler from "app.js";
if (handler == null || typeof handler.fetch !== "function") {
  throw new TypeError("module must export default { fetch }");
}
globalThis.__tysel_fetch = handler.fetch.bind(handler);
"#;

pub fn load_fetch_handler(ctx: Ctx<'_>, source: &str) -> Result<(), EngineError> {
    Module::declare(ctx.clone(), "app.js", source).map_err(js_err)?;
    let promise = Module::evaluate(ctx.clone(), "tysel-boot.js", BOOT_FETCH).map_err(js_err)?;
    ctx.globals().set("__tysel_result", promise).map_err(js_err)?;
    Ok(())
}

pub fn begin_fetch(ctx: Ctx<'_>, request: &HttpRequest) -> Result<bool, EngineError> {
    let fetch: Function = ctx.globals().get("__tysel_fetch").map_err(js_err)?;
    let js_request = to_js_request(&ctx, request)?;
    let result: rquickjs::Value = fetch.call((js_request,)).map_err(js_err)?;
    if result.is_promise() {
        ctx.globals().set("__tysel_result", result).map_err(js_err)?;
        Ok(true)
    } else {
        ctx.globals().set("__tysel_response", result).map_err(js_err)?;
        Ok(false)
    }
}

pub fn take_response_into_globals(ctx: Ctx<'_>) -> Result<(), EngineError> {
    let promise: rquickjs::Promise = ctx.globals().get("__tysel_result").map_err(js_err)?;
    let value: rquickjs::Value = promise
        .result::<rquickjs::Value>()
        .ok_or_else(|| EngineError::Isolate("fetch promise still pending".into()))?
        .map_err(js_err)?;
    ctx.globals().set("__tysel_response", value).map_err(js_err)?;
    Ok(())
}

pub fn emit_response(
    ctx: Ctx<'_>,
    head_tx: oneshot::Sender<Result<HttpHead, EngineError>>,
    body_tx: mpsc::Sender<Vec<u8>>,
) -> Result<(), EngineError> {
    let response: Object = ctx.globals().get("__tysel_response").map_err(js_err)?;
    let status: i32 = response.get("status").unwrap_or(200);
    let headers = read_headers(&response)?;
    let _ = head_tx.send(Ok(HttpHead { status: status.max(0) as u16, headers }));
    let body: rquickjs::Value = response.get("body").map_err(js_err)?;
    send_body(body, &body_tx)?;
    Ok(())
}

fn read_headers(response: &Object<'_>) -> Result<Vec<(String, String)>, EngineError> {
    let headers_obj: Object = response.get("headers").map_err(js_err)?;
    let map: Object = headers_obj.get("_map").map_err(js_err)?;
    let mut headers = Vec::new();
    for entry in map.props::<String, String>() {
        let (key, value) = entry.map_err(js_err)?;
        headers.push((key, value));
    }
    Ok(headers)
}

fn send_body(
    body: rquickjs::Value<'_>,
    body_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<(), EngineError> {
    if body.is_null() || body.is_undefined() {
        return Ok(());
    }
    if let Some(array) = body.as_array() {
        for i in 0..array.len() {
            send_chunk(array.get::<rquickjs::Value>(i).map_err(js_err)?, body_tx)?;
        }
        return Ok(());
    }
    send_chunk(body, body_tx)
}

fn send_chunk(
    chunk: rquickjs::Value<'_>,
    body_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<(), EngineError> {
    let bytes = if let Some(text) = chunk.as_string() {
        text.to_string().map_err(js_err)?.into_bytes()
    } else {
        return Err(EngineError::Isolate("response chunk must be a string".into()));
    };
    let _ = body_tx.blocking_send(bytes);
    Ok(())
}

fn to_js_request<'js>(ctx: &Ctx<'js>, request: &HttpRequest) -> Result<Object<'js>, EngineError> {
    let factory: Function = ctx.eval("(url, init) => new Request(url, init)").map_err(js_err)?;
    let init = Object::new(ctx.clone()).map_err(js_err)?;
    init.set("method", request.method.as_str()).map_err(js_err)?;
    if !request.body.is_empty() {
        init.set("body", String::from_utf8_lossy(&request.body).into_owned()).map_err(js_err)?;
    }
    let headers = Object::new(ctx.clone()).map_err(js_err)?;
    for (key, value) in &request.headers {
        headers.set(key.as_str(), value.as_str()).map_err(js_err)?;
    }
    init.set("headers", headers).map_err(js_err)?;
    factory.call((request.url.as_str(), init)).map_err(js_err)
}
