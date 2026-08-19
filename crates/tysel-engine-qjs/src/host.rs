use rquickjs::{Ctx, Function, IntoJs, Object, Promise};
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
        Function::new(ctx.clone(), move |ctx, url: String, method: String| {
            submit(ctx, &io_http_start, |id| IoRequest::HttpGet { id, url, method })
        })?,
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
    ctx.globals().set("tysel", tysel)?;
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = async function(input, init) {
          init = init || {};
          const url = typeof input === "string" ? input : input.url;
          const method = String(init.method || (input && input.method) || "GET").toUpperCase();
          const status = await tysel._httpStart(String(url), method);
          const response = new Response(null, { status: status });
          response._stream = true;
          return response;
        };
        globalThis.tysel.httpGet = function(url) {
          return fetch(url);
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
    let globals = ctx.globals();
    globals.remove(PENDING)?;
    globals.remove("tysel")?;
    Ok(())
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
