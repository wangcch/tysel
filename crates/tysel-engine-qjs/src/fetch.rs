use rquickjs::{Ctx, Function, Module, Object};
use tokio::sync::{mpsc, oneshot};
use tysel_engine::{EngineError, HttpHead, HttpRequest};

use crate::isolate::{js_err, js_err_ctx};

const BOOTSTRAP: &str = include_str!("../../../runtime-js/web-api/runtime.js");

pub fn install_web_api(ctx: Ctx<'_>) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(BOOTSTRAP)
}

const BOOT_FETCH: &str = include_str!("../../../runtime-js/bootstrap/fetch.js");

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
