use rquickjs::{Ctx, Exception, Function, IntoJs, Object, Promise, TypedArray};
use tysel_engine::{InterruptReason, Value};

use crate::queue::{IoHandle, IoRequest, OpId};

const PENDING: &str = "__tysel_pending";

pub fn install(ctx: Ctx<'_>, io: IoHandle, isolate_id: u32) -> rquickjs::Result<()> {
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
        globalThis.tysel.secrets = {
          ref(name) {
            return tysel.secretRef(String(name));
          },
        };
        "#,
    )?;
    Ok(())
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
