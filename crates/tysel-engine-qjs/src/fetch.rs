use rquickjs::{Ctx, Function, Module, Object};
use tokio::sync::{mpsc, oneshot};
use tysel_engine::{EngineError, HttpHead, HttpRequest};

use crate::isolate::{js_err, js_err_ctx};

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
      // Arrays have forEach, so the sequence form must be checked first.
      if (Array.isArray(init)) {
        for (const pair of init) this.append(pair[0], pair[1]);
      } else if (typeof init.forEach === "function") {
        init.forEach((value, key) => this.append(key, value));
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
        this._stream = init.bodyStream === true;
        this._text = null;
      } else {
        this.url = input.url;
        this.method = String(init.method || input.method || "GET").toUpperCase();
        this.headers = new Headers(init.headers || input.headers);
        this.body = init.body == null ? input.body : init.body;
        this._stream = init.bodyStream === true || (init.body == null && input._stream === true);
        this._text = input._text || null;
      }
    }
    async text() {
      if (this._text != null) return this._text;
      if (this._stream) {
        const chunks = [];
        for (;;) {
          const chunk = await tysel.readBody();
          if (chunk == null) break;
          chunks.push(chunk);
        }
        this._text = chunks.join("");
        this._stream = false;
        return this._text;
      }
      this._text = this.body == null ? "" : String(this.body);
      return this._text;
    }
    async json() {
      const text = await this.text();
      return text ? JSON.parse(text) : null;
    }
    clone() {
      if (this._stream) {
        throw new TypeError("cannot clone a streaming request");
      }
      return new Request(this.url, { method: this.method, headers: this.headers, body: this.body });
    }
  }

  class Response {
    constructor(body, init) {
      init = init || {};
      this.body = body == null ? null : body;
      this.status = init.status || 200;
      this.headers = new Headers(init.headers);
      this._stream = false;
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
      if (this._stream) {
        const chunks = [];
        for (;;) {
          const chunk = await tysel._httpRead();
          if (chunk == null) break;
          chunks.push(chunk);
        }
        this.body = chunks.join("");
        this._stream = false;
        return this.body;
      }
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

  class TextEncoder {
    constructor() {
      this.encoding = "utf-8";
    }
    encode(input) {
      return tysel._utf8Encode(input == null ? "" : String(input));
    }
  }

  class TextDecoder {
    constructor(label, options) {
      const encoding = String(label == null ? "utf-8" : label)
        .trim()
        .toLowerCase()
        .replace(/[_-]/g, "");
      if (encoding !== "utf8") {
        throw new RangeError("TextDecoder only supports utf-8");
      }
      this.encoding = "utf-8";
      this.fatal = Boolean(options && options.fatal);
      this.ignoreBOM = Boolean(options && options.ignoreBOM);
    }
    decode(input) {
      if (input == null) return "";
      let view;
      if (input instanceof ArrayBuffer) view = new Uint8Array(input);
      else if (ArrayBuffer.isView(input)) {
        view = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      } else {
        throw new TypeError("expected BufferSource");
      }
      let text = tysel._utf8Decode(view, this.fatal);
      if (this.ignoreBOM && text.charCodeAt(0) === 0xfeff) text = text.slice(1);
      return text;
    }
  }

  const timers = new Map();
  let nextTimerId = 1;
  let timerGeneration = 0;

  function scheduleTimer(fn, ms, interval, args) {
    if (typeof fn !== "function") {
      throw new TypeError("timer callback must be a function");
    }
    const id = nextTimerId++;
    const delay = Math.max(0, Number(ms) || 0);
    const generation = timerGeneration;
    let cleared = false;
    timers.set(id, () => {
      cleared = true;
    });
    const tick = async () => {
      try {
        await tysel.sleep(delay);
      } catch {
        timers.delete(id);
        return;
      }
      if (cleared || generation !== timerGeneration) {
        timers.delete(id);
        return;
      }
      if (interval) tick();
      else timers.delete(id);
      fn.apply(undefined, args);
    };
    tick();
    return id;
  }

  globalThis.setTimeout = function (fn, ms) {
    return scheduleTimer(fn, ms, false, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.setInterval = function (fn, ms) {
    return scheduleTimer(fn, ms, true, Array.prototype.slice.call(arguments, 2));
  };
  globalThis.clearTimeout = function (id) {
    const clear = timers.get(id);
    if (clear) {
      clear();
      timers.delete(id);
    }
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.__tysel_resetTimers = function () {
    timerGeneration++;
    timers.forEach((clear) => clear());
    timers.clear();
  };

  class CryptoKey {
    constructor(token, type, extractable, algorithm, usages) {
      if (token !== cryptoKeyToken) throw new TypeError("Illegal constructor");
      Object.defineProperties(this, {
        type: { value: type, enumerable: true },
        extractable: { value: extractable, enumerable: true },
        algorithm: { value: Object.freeze(algorithm), enumerable: true },
        usages: { value: Object.freeze(usages), enumerable: true },
      });
      Object.freeze(this);
    }
  }
  globalThis.CryptoKey = CryptoKey;
  globalThis.crypto = {
    getRandomValues(typedArray) {
      if (typedArray == null || typedArray.buffer == null) {
        throw new TypeError("expected TypedArray");
      }
      const view = new Uint8Array(
        typedArray.buffer,
        typedArray.byteOffset,
        typedArray.byteLength,
      );
      if (view.byteLength > 65536) {
        throw new Error("QuotaExceededError");
      }
      view.set(tysel._randomBytes(view.byteLength));
      return typedArray;
    },
    subtle: {
      async digest(algorithm, data) {
        const name = normalizeCryptoHash(algorithm);
        return tysel._digest(name, toCryptoBytes(data)).buffer;
      },
      async importKey(format, keyData, algorithm, extractable, keyUsages) {
        if (format !== "raw") throw new DOMException("only raw CryptoKeys are supported", "NotSupportedError");
        const algo = typeof algorithm === "string" ? { name: algorithm } : algorithm || {};
        if (String(algo.name).toUpperCase() !== "HMAC") throw new DOMException("only HMAC keys are supported", "NotSupportedError");
        const hash = normalizeCryptoHash(algo.hash || "SHA-256");
        const usages = Array.from(keyUsages || [], String);
        if (usages.length === 0) {
          throw new DOMException("secret keys require at least one usage", "SyntaxError");
        }
        if (usages.some((usage) => usage !== "sign" && usage !== "verify")) {
          throw new DOMException("HMAC keys only support sign and verify usages", "SyntaxError");
        }
        const bytes = new Uint8Array(toCryptoBytes(keyData));
        const sourceLength = bytes.byteLength * 8;
        if (sourceLength === 0) throw new DOMException("HMAC key data cannot be empty", "DataError");
        let length = sourceLength;
        if (algo.length !== undefined) {
          length = Number(algo.length);
          if (!Number.isInteger(length) || length < 0 || length > 0xffffffff) {
            throw new TypeError("HMAC key length must be an unsigned integer");
          }
          if (length > sourceLength || length <= sourceLength - 8) {
            throw new DOMException("HMAC key length is inconsistent with key data", "DataError");
          }
          if (length % 8 !== 0) {
            bytes[bytes.length - 1] &= (0xff << (8 - (length % 8))) & 0xff;
          }
        }
        // Ask the native implementation to validate the hash before returning a key.
        tysel._digest(hash, new Uint8Array(0));
        const key = new CryptoKey(
          cryptoKeyToken,
          "secret",
          Boolean(extractable),
          { name: "HMAC", hash: Object.freeze({ name: hash }), length },
          usages,
        );
        cryptoKeys.set(key, { hash, bytes });
        return key;
      },
      async sign(algorithm, key, data) {
        const rec = cryptoKeys.get(key);
        if (!rec) throw new TypeError("unknown CryptoKey");
        const name = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
        if (name.toUpperCase() !== "HMAC") throw new DOMException("only HMAC signing is supported", "NotSupportedError");
        if (!key.usages.includes("sign")) throw new DOMException("key does not allow signing", "InvalidAccessError");
        return tysel._hmac(rec.hash, rec.bytes, toCryptoBytes(data)).buffer;
      },
      async verify(algorithm, key, signature, data) {
        const rec = cryptoKeys.get(key);
        if (!rec) throw new TypeError("unknown CryptoKey");
        const name = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
        if (name.toUpperCase() !== "HMAC") throw new DOMException("only HMAC verification is supported", "NotSupportedError");
        if (!key.usages.includes("verify")) throw new DOMException("key does not allow verification", "InvalidAccessError");
        return tysel._hmacVerify(rec.hash, rec.bytes, toCryptoBytes(signature), toCryptoBytes(data));
      },
    },
  };
  const cryptoKeyToken = Object.freeze({});
  const cryptoKeys = new WeakMap();
  function normalizeCryptoHash(algorithm) {
    const raw = typeof algorithm === "string" ? algorithm : String(algorithm && algorithm.name || "");
    const compact = raw.trim().toUpperCase().replace("-", "");
    if (compact === "SHA256") return "SHA-256";
    if (compact === "SHA384") return "SHA-384";
    if (compact === "SHA512") return "SHA-512";
    throw new DOMException(`unsupported digest algorithm ${raw}`, "NotSupportedError");
  }
  function toCryptoBytes(data) {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) {
      return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    }
    throw new TypeError("expected BufferSource");
  }

  globalThis.Headers = Headers;
  globalThis.Request = Request;
  globalThis.Response = Response;
  globalThis.URL = URL;
  globalThis.URLSearchParams = URLSearchParams;
  globalThis.TextEncoder = TextEncoder;
  globalThis.TextDecoder = TextDecoder;
})();
"##;

pub fn install_web_api(ctx: Ctx<'_>) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(BOOTSTRAP)
}

const BOOT_FETCH: &str = r#"
import handler from "app.js";
if (handler == null || typeof handler.fetch !== "function") {
  throw new TypeError("module must export default { fetch }");
}
globalThis.__tysel_fetch = handler.fetch.bind(handler);
"#;

pub fn load_fetch_handler(ctx: Ctx<'_>, source: &str) -> Result<(), EngineError> {
    Module::declare(ctx.clone(), "app.js", source).map_err(|err| js_err_ctx(&ctx, err))?;
    let promise = Module::evaluate(ctx.clone(), "tysel-boot.js", BOOT_FETCH)
        .map_err(|err| js_err_ctx(&ctx, err))?;
    ctx.globals().set("__tysel_result", promise).map_err(js_err)?;
    Ok(())
}

pub fn begin_fetch(ctx: Ctx<'_>, request: &HttpRequest) -> Result<bool, EngineError> {
    let fetch: Function = ctx.globals().get("__tysel_fetch").map_err(js_err)?;
    let js_request = to_js_request(&ctx, request)?;
    let result: rquickjs::Value = fetch.call((js_request,)).map_err(|err| js_err_ctx(&ctx, err))?;
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
        .map_err(|err| js_err_ctx(&ctx, err))?;
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
    let _ = head_tx.send(Ok(HttpHead {
        status: status.max(0) as u16,
        headers,
        websocket: ctx.globals().get::<_, bool>("__tysel_ws_accepted").unwrap_or(false),
    }));
    let body: rquickjs::Value = response.get("body").map_err(js_err)?;
    send_body(body, &body_tx)?;
    Ok(())
}

pub fn arm_websocket(ctx: Ctx<'_>) -> Result<bool, EngineError> {
    let Ok(promise) = ctx.globals().get::<_, rquickjs::Promise>("__tysel_ws_done") else {
        return Ok(false);
    };
    ctx.globals().set("__tysel_result", promise).map_err(js_err)?;
    Ok(true)
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
    init.set("bodyStream", true).map_err(js_err)?;
    let headers = Object::new(ctx.clone()).map_err(js_err)?;
    for (key, value) in &request.headers {
        headers.set(key.as_str(), value.as_str()).map_err(js_err)?;
    }
    init.set("headers", headers).map_err(js_err)?;
    factory.call((request.url.as_str(), init)).map_err(js_err)
}
