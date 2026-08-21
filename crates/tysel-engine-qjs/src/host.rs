use hmac::{Hmac, Mac};
use rquickjs::{Ctx, Exception, Function, IntoJs, Object, Promise, TypedArray};
use sha2::{Digest, Sha256, Sha384, Sha512};
use tysel_engine::{InterruptReason, Value};

use crate::DurableSession;
use crate::queue::{IoHandle, IoRequest, OpId};

const PENDING: &str = "__tysel_pending";

pub fn install(ctx: Ctx<'_>, io: IoHandle, isolate_id: u32) -> rquickjs::Result<()> {
    install_inner(ctx, io, isolate_id, None)
}

pub(crate) fn install_durable(
    ctx: Ctx<'_>,
    io: IoHandle,
    isolate_id: u32,
    durable: DurableSession,
) -> rquickjs::Result<()> {
    install_inner(ctx, io, isolate_id, Some(durable))
}

fn install_inner(
    ctx: Ctx<'_>,
    io: IoHandle,
    isolate_id: u32,
    durable: Option<DurableSession>,
) -> rquickjs::Result<()> {
    crate::fetch::install_web_api(ctx.clone())?;
    ctx.globals().set(PENDING, Object::new(ctx.clone())?)?;

    let tysel = Object::new(ctx.clone())?;
    tysel.set("isolateId", isolate_id)?;
    let io_sleep = io.clone();
    let io_echo = io.clone();
    let io_secret = io.clone();
    let io_body = io.clone();
    let io_http_start = io.clone();
    let io_http_read = io.clone();
    let io_ws_read = io.clone();
    let io_ws_send = io.clone();
    let io_ws_close = io.clone();
    let io_ws_connect = io.clone();
    let io_ws_client_read = io.clone();
    let io_ws_client_send = io.clone();
    let io_ws_client_close = io.clone();
    let io_sqlite_exec = io.clone();
    let io_sqlite_query = io.clone();
    let io_pg_exec = io.clone();
    let io_pg_query = io.clone();
    let io_fs_read = io.clone();
    let io_fs_write = io.clone();
    let io_llm = io.clone();
    tysel.set(
        "sleep",
        Function::new(ctx.clone(), move |ctx, millis: f64| {
            submit(ctx, &io_sleep, |id| IoRequest::Sleep { id, millis: millis.max(0.0) as u64 })
        })?,
    )?;
    tysel.set(
        "echo",
        Function::new(ctx.clone(), move |ctx, value: String| {
            submit(ctx, &io_echo, |id| IoRequest::Echo { id, value })
        })?,
    )?;
    tysel.set(
        "secretRef",
        Function::new(ctx.clone(), move |ctx, name: String| {
            submit(ctx, &io_secret, |id| IoRequest::SecretRef { id, name })
        })?,
    )?;
    tysel.set(
        "readBody",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_body, |id| IoRequest::ReadBody { id })
        })?,
    )?;
    tysel.set(
        "_httpStart",
        Function::new(
            ctx.clone(),
            move |ctx, url: String, method: String, headers_json: String, body: String| {
                submit(ctx, &io_http_start, |id| IoRequest::HttpGet {
                    id,
                    url,
                    method,
                    headers_json,
                    body,
                })
            },
        )?,
    )?;
    tysel.set(
        "_httpRead",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_http_read, |id| IoRequest::HttpRead { id })
        })?,
    )?;
    tysel.set(
        "envKeys",
        Function::new(ctx.clone(), || {
            std::env::vars().map(|(key, _)| key).collect::<Vec<_>>().join(",")
        })?,
    )?;
    tysel.set(
        "_utf8Encode",
        Function::new(ctx.clone(), |ctx, text: String| {
            TypedArray::<u8>::new(ctx, text.into_bytes())
        })?,
    )?;
    tysel.set(
        "_utf8Decode",
        Function::new(ctx.clone(), |ctx, bytes: TypedArray<u8>, fatal: bool| {
            let Some(raw) = bytes.as_bytes() else {
                return Ok(String::new());
            };
            if fatal {
                String::from_utf8(raw.to_vec())
                    .map_err(|_| Exception::throw_type(&ctx, "UTF-8 decode failed"))
            } else {
                Ok(String::from_utf8_lossy(raw).into_owned())
            }
        })?,
    )?;
    tysel.set(
        "_randomBytes",
        Function::new(ctx.clone(), |ctx, len: f64| {
            if !len.is_finite() || len < 0.0 {
                return Err(Exception::throw_type(&ctx, "byte length must be a number"));
            }
            let n = len.min(65536.0) as usize;
            let mut buf = vec![0u8; n];
            getrandom::fill(&mut buf)
                .map_err(|err| Exception::throw_type(&ctx, &err.to_string()))?;
            TypedArray::<u8>::new(ctx, buf)
        })?,
    )?;
    tysel.set(
        "_digest",
        Function::new(ctx.clone(), |ctx, algorithm: String, data: TypedArray<u8>| {
            let out = digest_bytes(&algorithm, data.as_ref())
                .map_err(|err| Exception::throw_type(&ctx, &err))?;
            TypedArray::<u8>::new(ctx, out)
        })?,
    )?;
    tysel.set(
        "_hmac",
        Function::new(
            ctx.clone(),
            |ctx, algorithm: String, key: TypedArray<u8>, data: TypedArray<u8>| {
                let out = hmac_bytes(&algorithm, key.as_ref(), data.as_ref())
                    .map_err(|err| Exception::throw_type(&ctx, &err))?;
                TypedArray::<u8>::new(ctx, out)
            },
        )?,
    )?;
    tysel.set(
        "_hmacVerify",
        Function::new(
            ctx.clone(),
            |ctx,
             algorithm: String,
             key: TypedArray<u8>,
             signature: TypedArray<u8>,
             data: TypedArray<u8>| {
                hmac_verify(&algorithm, key.as_ref(), signature.as_ref(), data.as_ref())
                    .map_err(|err| Exception::throw_type(&ctx, &err))
            },
        )?,
    )?;
    tysel.set(
        "_durableStart",
        Function::new(ctx.clone(), |ctx, name: String, input_json: String| {
            crate::control::start_named(&name, &input_json)
                .map_err(|err| Exception::throw_type(&ctx, &err))
        })?,
    )?;
    tysel.set(
        "_durableSendSignal",
        Function::new(ctx.clone(), |ctx, task_id: String, name: String, payload_json: String| {
            crate::control::send_signal(&task_id, &name, &payload_json)
                .map_err(|err| Exception::throw_type(&ctx, &err))
        })?,
    )?;
    tysel.set(
        "_wsRead",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_ws_read, |id| IoRequest::WsRead { id })
        })?,
    )?;
    tysel.set(
        "_wsSend",
        Function::new(ctx.clone(), move |ctx, data: String| {
            submit(ctx, &io_ws_send, |id| IoRequest::WsSend { id, data })
        })?,
    )?;
    tysel.set(
        "_wsClose",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_ws_close, |id| IoRequest::WsClose { id })
        })?,
    )?;
    tysel.set(
        "_wsConnect",
        Function::new(ctx.clone(), move |ctx, url: String| {
            submit(ctx, &io_ws_connect, |id| IoRequest::WsConnect { id, url })
        })?,
    )?;
    tysel.set(
        "_wsClientRead",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_ws_client_read, |id| IoRequest::WsClientRead { id })
        })?,
    )?;
    tysel.set(
        "_wsClientSend",
        Function::new(ctx.clone(), move |ctx, data: String| {
            submit(ctx, &io_ws_client_send, |id| IoRequest::WsClientSend { id, data })
        })?,
    )?;
    tysel.set(
        "_wsClientClose",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_ws_client_close, |id| IoRequest::WsClientClose { id })
        })?,
    )?;
    tysel.set(
        "_sqliteExec",
        Function::new(ctx.clone(), move |ctx, sql: String, params_json: String| {
            submit(ctx, &io_sqlite_exec, |id| IoRequest::SqliteExec { id, sql, params_json })
        })?,
    )?;
    tysel.set(
        "_sqliteQuery",
        Function::new(ctx.clone(), move |ctx, sql: String, params_json: String| {
            submit(ctx, &io_sqlite_query, |id| IoRequest::SqliteQuery { id, sql, params_json })
        })?,
    )?;
    tysel.set(
        "_pgExec",
        Function::new(ctx.clone(), move |ctx, sql: String, params_json: String| {
            submit(ctx, &io_pg_exec, |id| IoRequest::PostgresExec { id, sql, params_json })
        })?,
    )?;
    tysel.set(
        "_pgQuery",
        Function::new(ctx.clone(), move |ctx, sql: String, params_json: String| {
            submit(ctx, &io_pg_query, |id| IoRequest::PostgresQuery { id, sql, params_json })
        })?,
    )?;
    tysel.set(
        "_fsRead",
        Function::new(ctx.clone(), move |ctx, path: String| {
            submit(ctx, &io_fs_read, |id| IoRequest::FsRead { id, path })
        })?,
    )?;
    tysel.set(
        "_fsWrite",
        Function::new(ctx.clone(), move |ctx, path: String, data: String| {
            submit(ctx, &io_fs_write, |id| IoRequest::FsWrite { id, path, data })
        })?,
    )?;
    tysel.set(
        "_llmGenerate",
        Function::new(ctx.clone(), move |ctx, request_json: String| {
            submit(ctx, &io_llm, |id| IoRequest::LlmGenerate { id, request_json })
        })?,
    )?;
    let durable_enabled = durable.is_some();
    if let Some(durable) = durable {
        let lookup = durable.clone();
        let retry_outcome = durable.clone();
        let record = durable.clone();
        let record_sleep = durable.clone();
        let poll_signal = durable.clone();
        tysel.set(
            "_durableLookup",
            Function::new(ctx.clone(), move |ctx, kind: String, key: String| {
                lookup.lookup_json(&kind, &key).map_err(|err| Exception::throw_type(&ctx, &err))
            })?,
        )?;
        tysel.set(
            "_durableFindRetryOutcome",
            Function::new(ctx.clone(), move |ctx, key: String| {
                retry_outcome
                    .find_retry_outcome_json(&key)
                    .map_err(|err| Exception::throw_type(&ctx, &err))
            })?,
        )?;
        tysel.set(
            "_durableRecord",
            Function::new(
                ctx.clone(),
                move |ctx, kind: String, key: String, payload_json: String, recorded_at_ms: f64| {
                    record
                        .record(
                            &kind,
                            key,
                            &payload_json,
                            durable_millis(&ctx, recorded_at_ms, "recorded time")?,
                        )
                        .map_err(|err| Exception::throw_type(&ctx, &err))
                },
            )?,
        )?;
        tysel.set(
            "_durableRecordSleep",
            Function::new(
                ctx.clone(),
                move |ctx,
                      key: String,
                      payload_json: String,
                      recorded_at_ms: f64,
                      wake_at_ms: f64| {
                    record_sleep
                        .record_sleep(
                            key,
                            &payload_json,
                            durable_millis(&ctx, recorded_at_ms, "recorded time")?,
                            durable_millis(&ctx, wake_at_ms, "wakeup time")?,
                        )
                        .map_err(|err| Exception::throw_type(&ctx, &err))
                },
            )?,
        )?;
        tysel.set(
            "_durableCompleteSleep",
            Function::new(ctx.clone(), move |ctx| {
                durable.complete_sleep().map_err(|err| Exception::throw_type(&ctx, &err))
            })?,
        )?;
        tysel.set(
            "_durablePollSignal",
            Function::new(ctx.clone(), move |ctx, signal_name: String| {
                poll_signal
                    .poll_signal_json(&signal_name)
                    .map_err(|err| Exception::throw_type(&ctx, &err))
            })?,
        )?;
    }
    ctx.globals().set("tysel", tysel)?;
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = async function(input, init) {
          init = init || {};
          const url = typeof input === "string" ? input : input.url;
          const method = String(init.method || (input && input.method) || "GET").toUpperCase();
          const headers = new Headers(init.headers || (input && input.headers));
          const pairs = [];
          headers.forEach((value, key) => pairs.push([key, value]));
          let body = "";
          if (init.body != null) body = String(init.body);
          else if (input && typeof input !== "string" && input.body != null) body = String(input.body);
          const started = await tysel._httpStart(String(url), method, JSON.stringify(pairs), body);
          let headerPairs = [];
          try { headerPairs = JSON.parse(started.headers || "[]"); } catch (_) {}
          const response = new Response(null, { status: started.status, headers: headerPairs });
          response._stream = true;
          return response;
        };
        globalThis.tysel.httpGet = function(url) {
          return fetch(url);
        };
        globalThis.tysel.acceptWebSocket = function() {
          if (globalThis.__tysel_ws_accepted) {
            throw new Error("websocket already accepted");
          }
          globalThis.__tysel_ws_accepted = true;
          const listeners = { message: [], close: [], error: [] };
          const socket = {
            readyState: 1,
            send(data) {
              return tysel._wsSend(data == null ? "" : String(data));
            },
            close() {
              this.readyState = 2;
              return tysel._wsClose();
            },
            addEventListener(type, fn) {
              if (listeners[type]) listeners[type].push(fn);
            },
          };
          globalThis.__tysel_ws_done = (async () => {
            try {
              for (;;) {
                const chunk = await tysel._wsRead();
                if (chunk == null) break;
                const event = { type: "message", data: chunk };
                for (const fn of listeners.message) fn(event);
              }
            } finally {
              socket.readyState = 3;
              for (const fn of listeners.close) fn({ type: "close" });
            }
          })();
          return socket;
        };
        if (!Number.isInteger(globalThis.__tysel_request_generation)) {
          globalThis.__tysel_request_generation = 0;
        }
        class WebSocket {
          static CONNECTING = 0;
          static OPEN = 1;
          static CLOSING = 2;
          static CLOSED = 3;
          constructor(url) {
            this.url = String(url);
            this._generation = globalThis.__tysel_request_generation;
            this.readyState = WebSocket.CONNECTING;
            this.binaryType = "arraybuffer";
            this.onopen = null;
            this.onmessage = null;
            this.onerror = null;
            this.onclose = null;
            this._listeners = { open: [], message: [], error: [], close: [] };
            this.opened = tysel._wsConnect(this.url).then(() => {
              if (this._generation !== globalThis.__tysel_request_generation) return this;
              this.readyState = WebSocket.OPEN;
              this._dispatch("open", { type: "open", target: this });
              this._readLoop();
              return this;
            }, (error) => {
              if (this._generation !== globalThis.__tysel_request_generation) return this;
              this.readyState = WebSocket.CLOSED;
              this._dispatch("error", { type: "error", error, target: this });
              this._dispatch("close", { type: "close", code: 1006, reason: String(error), wasClean: false, target: this });
              throw error;
            });
          }
          addEventListener(type, listener) {
            if (this._listeners[type] && typeof listener === "function") this._listeners[type].push(listener);
          }
          removeEventListener(type, listener) {
            if (!this._listeners[type]) return;
            this._listeners[type] = this._listeners[type].filter((item) => item !== listener);
          }
          send(data) {
            if (this.readyState !== WebSocket.OPEN) throw new DOMException("WebSocket is not open", "InvalidStateError");
            return tysel._wsClientSend(data == null ? "" : String(data));
          }
          async close() {
            if (this.readyState === WebSocket.CLOSED || this.readyState === WebSocket.CLOSING) return;
            this.readyState = WebSocket.CLOSING;
            await tysel._wsClientClose();
          }
          _dispatch(type, event) {
            if (this._generation !== globalThis.__tysel_request_generation) return;
            const handler = this[`on${type}`];
            if (typeof handler === "function") handler.call(this, event);
            for (const listener of this._listeners[type]) listener.call(this, event);
          }
          async _readLoop() {
            try {
              while (this.readyState === WebSocket.OPEN) {
                const frame = await tysel._wsClientRead();
                if (this._generation !== globalThis.__tysel_request_generation) return;
                if (frame.type === "close") {
                  this.readyState = WebSocket.CLOSED;
                  this._dispatch("close", {
                    type: "close",
                    code: frame.code,
                    reason: frame.reason,
                    wasClean: frame.wasClean,
                    target: this,
                  });
                  return;
                }
                let data = frame.data;
                if (Array.isArray(data)) data = Uint8Array.from(data).buffer;
                this._dispatch("message", { type: "message", data, target: this });
              }
              this.readyState = WebSocket.CLOSED;
              this._dispatch("close", { type: "close", code: 1000, reason: "", wasClean: true, target: this });
            } catch (error) {
              if (this._generation !== globalThis.__tysel_request_generation) return;
              this.readyState = WebSocket.CLOSED;
              this._dispatch("error", { type: "error", error, target: this });
              this._dispatch("close", { type: "close", code: 1006, reason: String(error), wasClean: false, target: this });
            }
          }
        }
        globalThis.WebSocket = WebSocket;
        globalThis.tysel.sqlite = {
          exec(sql, params) {
            return tysel._sqliteExec(String(sql), JSON.stringify(params == null ? [] : params));
          },
          query(sql, params) {
            return tysel._sqliteQuery(String(sql), JSON.stringify(params == null ? [] : params));
          },
        };
        globalThis.tysel.postgres = {
          exec(sql, params) {
            return tysel._pgExec(String(sql), JSON.stringify(params == null ? [] : params));
          },
          query(sql, params) {
            return tysel._pgQuery(String(sql), JSON.stringify(params == null ? [] : params));
          },
        };
        globalThis.tysel.fs = {
          read(path) {
            return tysel._fsRead(String(path));
          },
          write(path, data) {
            return tysel._fsWrite(String(path), data == null ? "" : String(data));
          },
        };
        globalThis.tysel.secrets = {
          ref(name) {
            return tysel.secretRef(String(name));
          },
        };
        globalThis.tysel.llm = {
          generate(options) {
            if (options === null || typeof options !== "object" || Array.isArray(options)) {
              throw new TypeError("llm.generate options must be an object");
            }
            return tysel._llmGenerate(JSON.stringify(options));
          },
        };
        "#,
    )?;
    if durable_enabled {
        ctx.eval::<(), _>(DURABLE_API)?;
    }
    ctx.eval::<(), _>(
        r#"
        if (!globalThis.tysel.durable) globalThis.tysel.durable = {};
        globalThis.tysel.durable.start = function(name, input) {
          return JSON.parse(tysel._durableStart(String(name), JSON.stringify(input === undefined ? null : input)));
        };
        globalThis.tysel.durable.sendSignal = function(taskId, name, payload) {
          tysel._durableSendSignal(String(taskId), String(name), JSON.stringify(payload === undefined ? null : payload));
        };
        "#,
    )?;
    Ok(())
}

const DURABLE_API: &str = r#"
(() => {
  let active = false;
  let retryIndex = 0;

  function lookup(kind, key) {
    return JSON.parse(tysel._durableLookup(kind, key));
  }

  function encode(value) {
    const encoded = JSON.stringify(value);
    if (encoded === undefined) {
      throw new TypeError("durable values must be JSON serializable");
    }
    return encoded;
  }

  function enter() {
    if (active) {
      throw new Error("durable boundaries must be awaited sequentially");
    }
    active = true;
  }

  function durationMs(value) {
    if (typeof value === "number") {
      if (!Number.isFinite(value) || value < 0) throw new TypeError("invalid durable duration");
      return Math.floor(value);
    }
    const match = /^\s*(\d+(?:\.\d+)?)\s*(ms|s|m|h|d)\s*$/.exec(String(value));
    if (!match) throw new TypeError("invalid durable duration");
    const scales = { ms: 1, s: 1000, m: 60000, h: 3600000, d: 86400000 };
    const millis = Number(match[1]) * scales[match[2]];
    if (!Number.isSafeInteger(Math.floor(millis))) throw new TypeError("durable duration is too large");
    return Math.floor(millis);
  }

  function retryPolicy(value) {
    if (value === null || typeof value !== "object" || Array.isArray(value)) {
      throw new TypeError("durable retry policy must be an object");
    }
    const maxAttempts = value.maxAttempts === undefined ? 3 : Number(value.maxAttempts);
    if (!Number.isInteger(maxAttempts) || maxAttempts < 1 || maxAttempts > 100) {
      throw new TypeError("durable retry maxAttempts must be an integer from 1 to 100");
    }
    const delayMs = value.delay === undefined ? 0 : durationMs(value.delay);
    const factor = value.factor === undefined ? 2 : Number(value.factor);
    if (!Number.isFinite(factor) || factor < 1 || factor > 100) {
      throw new TypeError("durable retry factor must be from 1 to 100");
    }
    const maxDelayMs = value.maxDelay === undefined ? null : durationMs(value.maxDelay);
    return { maxAttempts, delayMs, factor, maxDelayMs };
  }

  function retryDelayMs(policy, attempt) {
    const scaled = Math.floor(policy.delayMs * Math.pow(policy.factor, attempt - 1));
    const millis = policy.maxDelayMs === null ? scaled : Math.min(scaled, policy.maxDelayMs);
    if (!Number.isSafeInteger(millis)) {
      throw new TypeError("durable retry delay is too large");
    }
    return millis;
  }

  function retryFailure(error) {
    let name = "Error";
    let message = "retry callback failed";
    try {
      if (error && typeof error.name === "string") name = error.name;
      message = error && typeof error.message === "string" ? error.message : String(error);
    } catch (_) {}
    return { name: name.slice(0, 256), message: message.slice(0, 4096) };
  }

  function throwRetryFailure(failure) {
    if (!failure || typeof failure.name !== "string" || typeof failure.message !== "string") {
      throw new Error("invalid durable retry history");
    }
    const error = new Error(failure.message);
    error.name = failure.name;
    throw error;
  }

  function retryLookupOrRecord(key, payload) {
    enter();
    try {
      const replay = lookup("retry", key);
      if (replay.found) return replay.payload;
      if (payload !== undefined) {
        tysel._durableRecord("retry", key, encode(payload), Date.now());
      }
      return undefined;
    } finally {
      active = false;
    }
  }

  function findRetryOutcome(key) {
    enter();
    try {
      return JSON.parse(tysel._durableFindRetryOutcome(key));
    } finally {
      active = false;
    }
  }

  function applyRetryOutcome(outcome) {
    if (!outcome || typeof outcome.ok !== "boolean") {
      throw new Error("invalid durable retry outcome");
    }
    if (outcome.ok) return outcome.value;
    throwRetryFailure(outcome.failure);
  }

  async function boundary(kind, name, fn) {
    if (typeof fn !== "function") throw new TypeError("durable boundary requires a function");
    enter();
    try {
      const replay = lookup(kind, String(name));
      if (replay.found) return replay.payload;
      const value = await fn();
      tysel._durableRecord(kind, String(name), encode(value), Date.now());
      return value;
    } finally {
      active = false;
    }
  }

  const durable = {
    step(name, fn) {
      return boundary("step", name, fn);
    },
    effect(name, fn) {
      return boundary("effect", name, fn);
    },
    now() {
      enter();
      try {
        const replay = lookup("now", "now");
        if (replay.found) return new Date(replay.payload);
        const value = Date.now();
        tysel._durableRecord("now", "now", encode(value), value);
        return new Date(value);
      } finally {
        active = false;
      }
    },
    random() {
      enter();
      try {
        const replay = lookup("random", "random");
        if (replay.found) return replay.payload;
        const value = Math.random();
        tysel._durableRecord("random", "random", encode(value), Date.now());
        return value;
      } finally {
        active = false;
      }
    },
    async sleep(duration) {
      const millis = durationMs(duration);
      const key = "sleep:" + millis;
      enter();
      try {
        const replay = lookup("sleep", key);
        if (replay.found) {
          tysel._durableCompleteSleep();
          return;
        }
        const now = Date.now();
        const wakeAt = now + millis;
        if (!Number.isSafeInteger(wakeAt)) throw new TypeError("durable wakeup is too large");
        tysel._durableRecordSleep(key, encode({ durationMs: millis }), now, wakeAt);
        await tysel.sleep(millis);
        tysel._durableCompleteSleep();
      } finally {
        active = false;
      }
    },
    async waitForSignal(name) {
      const key = String(name);
      if (!key) throw new TypeError("durable signal name cannot be empty");
      enter();
      try {
        const replay = lookup("signal", key);
        if (replay.found) return replay.payload;
        const signal = JSON.parse(tysel._durablePollSignal(key));
        if (signal.found) return signal.payload;
        await new Promise(() => {});
      } finally {
        active = false;
      }
    },
    async retry(policyValue, fn) {
      if (typeof fn !== "function") throw new TypeError("durable retry requires a function");
      const policy = retryPolicy(policyValue);
      const retryId = retryIndex++;
      const scope = [
        "retry",
        retryId,
        policy.maxAttempts,
        policy.delayMs,
        policy.factor,
        policy.maxDelayMs === null ? "none" : policy.maxDelayMs,
      ].join(":");
      for (let attempt = 1; attempt <= policy.maxAttempts; attempt++) {
        retryLookupOrRecord(scope + ":start:" + attempt, { attempt });
        const outcomeKey = scope + ":outcome:" + attempt;
        const replayedOutcome = findRetryOutcome(outcomeKey);
        if (replayedOutcome.found) {
          const outcome = replayedOutcome.payload;
          if (outcome && outcome.ok === true) return applyRetryOutcome(outcome);
          if (attempt === policy.maxAttempts) return applyRetryOutcome(outcome);
          const delayMs = retryDelayMs(policy, attempt);
          if (delayMs > 0) await durable.sleep(delayMs);
          continue;
        }
        let failed = false;
        let failure;
        let value;
        try {
          value = await fn(attempt);
        } catch (error) {
          failed = true;
          failure = retryFailure(error);
        }
        const outcome = failed ? { ok: false, failure } : { ok: true, value };
        retryLookupOrRecord(outcomeKey, outcome);
        if (!failed) return applyRetryOutcome(outcome);
        if (attempt === policy.maxAttempts) throwRetryFailure(failure);
        const delayMs = retryDelayMs(policy, attempt);
        if (delayMs > 0) await durable.sleep(delayMs);
      }
      throw new Error("durable retry exhausted unexpectedly");
    },
  };
  globalThis.tysel.durable = durable;
})();
"#;

fn durable_millis(ctx: &Ctx<'_>, value: f64, label: &str) -> rquickjs::Result<u64> {
    if !value.is_finite() || value < 0.0 || value > i64::MAX as f64 {
        return Err(Exception::throw_type(ctx, &format!("invalid durable {label}")));
    }
    Ok(value as u64)
}

fn submit<'js>(
    ctx: Ctx<'js>,
    io: &IoHandle,
    request: impl FnOnce(OpId) -> IoRequest,
) -> rquickjs::Result<Promise<'js>> {
    let (promise, resolve, reject) = Promise::new(&ctx)?;
    let id = io.submit(request);
    let entry = Object::new(ctx.clone())?;
    entry.set("resolve", resolve)?;
    entry.set("reject", reject)?;
    pending(&ctx)?.set(id.0.to_string(), entry)?;
    Ok(promise)
}

pub fn settle(ctx: &Ctx<'_>, id: OpId, result: Result<Value, String>) -> rquickjs::Result<()> {
    let pending = pending(ctx)?;
    let key = id.0.to_string();
    let Some(entry): Option<Object> = pending.get(&key)? else {
        return Ok(());
    };
    pending.remove(&key)?;
    match result {
        Ok(value) => {
            let resolve: Function = entry.get("resolve")?;
            resolve.call::<(rquickjs::Value<'_>,), ()>((value_to_js(ctx, value)?,))?;
        }
        Err(error) => {
            if let Ok(reject) = entry.get::<_, Function>("reject") {
                let error_ctor: Function = ctx.globals().get("Error")?;
                if let Ok(exception) =
                    error_ctor.call::<(String,), rquickjs::Value>((error.clone(),))
                {
                    let _ = reject.call::<(rquickjs::Value<'_>,), ()>((exception,));
                } else {
                    let _ = reject.call::<(String,), ()>((error,));
                }
            }
        }
    }
    Ok(())
}

pub fn reject_all(ctx: &Ctx<'_>, reason: InterruptReason) -> rquickjs::Result<()> {
    let pending = pending(ctx)?;
    let keys: Vec<String> = pending.keys().collect::<rquickjs::Result<Vec<_>>>()?;
    for key in keys {
        let Some(entry): Option<Object> = pending.get(&key)? else {
            continue;
        };
        pending.remove(&key)?;
        if let Ok(reject) = entry.get::<_, Function>("reject") {
            let _ = reject.call::<(String,), ()>((format!("{reason:?}"),));
        }
    }
    Ok(())
}

pub fn drop_host(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let _ = reset_timers(ctx);
    let globals = ctx.globals();
    globals.remove(PENDING)?;
    globals.remove("tysel")?;
    Ok(())
}

pub fn reset_timers(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(
        "if (typeof globalThis.__tysel_resetTimers === 'function') globalThis.__tysel_resetTimers();",
    )
}

fn digest_bytes(algorithm: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match normalize_hash_name(algorithm) {
        "SHA-256" => Ok(Sha256::digest(data).to_vec()),
        "SHA-384" => Ok(Sha384::digest(data).to_vec()),
        "SHA-512" => Ok(Sha512::digest(data).to_vec()),
        _ => Err(format!("unsupported digest algorithm {algorithm}")),
    }
}

fn hmac_bytes(algorithm: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    match normalize_hash_name(algorithm) {
        "SHA-256" => finish_hmac(Hmac::<Sha256>::new_from_slice(key), data),
        "SHA-384" => finish_hmac(Hmac::<Sha384>::new_from_slice(key), data),
        "SHA-512" => finish_hmac(Hmac::<Sha512>::new_from_slice(key), data),
        _ => Err(format!("unsupported HMAC algorithm {algorithm}")),
    }
}

fn finish_hmac<M: Mac>(
    mac: Result<M, hmac::digest::InvalidLength>,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let mut mac = mac.map_err(|err| err.to_string())?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn hmac_verify(algorithm: &str, key: &[u8], signature: &[u8], data: &[u8]) -> Result<bool, String> {
    match normalize_hash_name(algorithm) {
        "SHA-256" => finish_hmac_verify(Hmac::<Sha256>::new_from_slice(key), signature, data),
        "SHA-384" => finish_hmac_verify(Hmac::<Sha384>::new_from_slice(key), signature, data),
        "SHA-512" => finish_hmac_verify(Hmac::<Sha512>::new_from_slice(key), signature, data),
        _ => Err(format!("unsupported HMAC algorithm {algorithm}")),
    }
}

fn finish_hmac_verify<M: Mac>(
    mac: Result<M, hmac::digest::InvalidLength>,
    signature: &[u8],
    data: &[u8],
) -> Result<bool, String> {
    let mut mac = mac.map_err(|err| err.to_string())?;
    mac.update(data);
    Ok(mac.verify_slice(signature).is_ok())
}

fn normalize_hash_name(algorithm: &str) -> &str {
    match algorithm.trim() {
        "SHA-256" | "SHA256" | "sha-256" | "sha256" => "SHA-256",
        "SHA-384" | "SHA384" | "sha-384" | "sha384" => "SHA-384",
        "SHA-512" | "SHA512" | "sha-512" | "sha512" => "SHA-512",
        other => other,
    }
}

fn pending<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    ctx.globals().get(PENDING)
}

fn value_to_js<'js>(ctx: &Ctx<'js>, value: Value) -> rquickjs::Result<rquickjs::Value<'js>> {
    match value {
        Value::Null => rquickjs::Null.into_js(ctx),
        Value::Bool(v) => v.into_js(ctx),
        Value::Number(v) => v.into_js(ctx),
        Value::String(v) => v.into_js(ctx),
        Value::Bytes(v) => v.into_js(ctx),
        Value::Array(items) => {
            let array = rquickjs::Array::new(ctx.clone())?;
            for (i, item) in items.into_iter().enumerate() {
                array.set(i, value_to_js(ctx, item)?)?;
            }
            array.into_js(ctx)
        }
        Value::Record(fields) => {
            let object = Object::new(ctx.clone())?;
            for (key, item) in fields {
                object.set(key, value_to_js(ctx, item)?)?;
            }
            object.into_js(ctx)
        }
    }
}
