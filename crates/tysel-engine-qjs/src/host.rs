use rquickjs::{Ctx, Exception, Function, IntoJs, Object, Promise, TypedArray};
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
    let io_sqlite_exec = io.clone();
    let io_sqlite_query = io.clone();
    let io_pg_exec = io.clone();
    let io_pg_query = io.clone();
    let io_fs_read = io.clone();
    let io_fs_write = io.clone();
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
    let durable_enabled = durable.is_some();
    if let Some(durable) = durable {
        let lookup = durable.clone();
        let record = durable.clone();
        let record_sleep = durable.clone();
        tysel.set(
            "_durableLookup",
            Function::new(ctx.clone(), move |ctx, kind: String, key: String| {
                lookup.lookup_json(&kind, &key).map_err(|err| Exception::throw_type(&ctx, &err))
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
        "#,
    )?;
    if durable_enabled {
        ctx.eval::<(), _>(DURABLE_API)?;
    }
    Ok(())
}

const DURABLE_API: &str = r#"
(() => {
  let active = false;

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
