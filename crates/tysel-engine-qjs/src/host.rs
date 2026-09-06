use hmac::{Hmac, Mac};
use rquickjs::{Ctx, Exception, Function, IntoJs, Object, Promise, TypedArray};
use sha2::{Digest, Sha256, Sha384, Sha512};
use tysel_engine::{InterruptReason, Value};

use crate::DurableSession;
use crate::queue::{IoHandle, IoRequest, OpId};

const PENDING: &str = "__tysel_pending";
const MAX_REDIS_TTL_SECONDS: f64 = 31_536_000.0;
const CAPABILITY_API: &str = include_str!("../../../runtime-js/capability-client/runtime.js");
const DURABLE_CONTROL_API: &str = include_str!("../../../runtime-js/durable/control.js");

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
    let io_http_cancel_body = io.clone();
    let io_cancel = io.clone();
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
    let io_redis_get = io.clone();
    let io_redis_set = io.clone();
    let io_redis_del = io.clone();
    let io_redis_exists = io.clone();
    let io_redis_expire = io.clone();
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
        "_secretRef",
        Function::new(ctx.clone(), move |ctx, name: String| {
            submit(ctx, &io_secret, |id| IoRequest::SecretRef { id, name })
        })?,
    )?;
    tysel.set(
        "_readBody",
        Function::new(ctx.clone(), move |ctx| {
            submit(ctx, &io_body, |id| IoRequest::ReadBody { id })
        })?,
    )?;
    tysel.set(
        "_httpStart",
        Function::new(
            ctx.clone(),
            move |ctx, url: String, method: String, headers_json: String, body: TypedArray<u8>| {
                let body = bytes::Bytes::copy_from_slice(
                    body.as_bytes()
                        .ok_or_else(|| Exception::throw_type(&ctx, "request body is detached"))?,
                );
                submit_cancellable(ctx, &io_http_start, |id| IoRequest::HttpGet {
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
        Function::new(ctx.clone(), move |ctx, body_id: u64| {
            submit_cancellable(ctx, &io_http_read, |id| IoRequest::HttpRead { id, body_id })
        })?,
    )?;
    tysel.set(
        "_httpCancelBody",
        Function::new(ctx.clone(), move |body_id: u64| {
            io_http_cancel_body.outbound.clear(body_id);
        })?,
    )?;
    tysel
        .set("_cancelOp", Function::new(ctx.clone(), move |id: u64| io_cancel.cancel(OpId(id)))?)?;
    tysel.set(
        "_envKeys",
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
            let raw = bytes.as_bytes().unwrap_or_default();
            // QuickJS copies the UTF-8 input into its own string storage. Borrow
            // valid input here instead of allocating an intermediate Rust String.
            if fatal {
                let text = std::str::from_utf8(raw)
                    .map_err(|_| Exception::throw_type(&ctx, "UTF-8 decode failed"))?;
                rquickjs::String::from_str(ctx, text)
            } else {
                let text = String::from_utf8_lossy(raw);
                rquickjs::String::from_str(ctx, &text)
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
        "_redisGet",
        Function::new(ctx.clone(), move |ctx, key: String| {
            submit(ctx, &io_redis_get, |id| IoRequest::RedisGet { id, key })
        })?,
    )?;
    tysel.set(
        "_redisSet",
        Function::new(
            ctx.clone(),
            move |ctx, key: String, value: String, ttl_seconds: Option<f64>| {
                let ttl_seconds = ttl_seconds.map(|value| redis_ttl(&ctx, value)).transpose()?;
                submit(ctx, &io_redis_set, |id| IoRequest::RedisSet { id, key, value, ttl_seconds })
            },
        )?,
    )?;
    tysel.set(
        "_redisDel",
        Function::new(ctx.clone(), move |ctx, keys_json: String| {
            submit(ctx, &io_redis_del, |id| IoRequest::RedisDel { id, keys_json })
        })?,
    )?;
    tysel.set(
        "_redisExists",
        Function::new(ctx.clone(), move |ctx, key: String| {
            submit(ctx, &io_redis_exists, |id| IoRequest::RedisExists { id, key })
        })?,
    )?;
    tysel.set(
        "_redisExpire",
        Function::new(ctx.clone(), move |ctx, key: String, ttl_seconds: f64| {
            let ttl_seconds = redis_ttl(&ctx, ttl_seconds)?;
            submit(ctx, &io_redis_expire, |id| IoRequest::RedisExpire { id, key, ttl_seconds })
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
    ctx.eval::<(), _>(CAPABILITY_API)?;
    if durable_enabled {
        ctx.eval::<(), _>(DURABLE_API)?;
    }
    ctx.eval::<(), _>(DURABLE_CONTROL_API)?;
    Ok(())
}

fn redis_ttl(ctx: &Ctx<'_>, value: f64) -> rquickjs::Result<u64> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(Exception::throw_type(ctx, "redis TTL must be a finite integer"));
    }
    if !(1.0..=MAX_REDIS_TTL_SECONDS).contains(&value) {
        return Err(Exception::throw_type(ctx, "redis TTL must be between 1 and 31536000 seconds"));
    }
    Ok(value as u64)
}

const DURABLE_API: &str = include_str!("../../../runtime-js/durable/runtime.js");

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
    submit_operation(ctx, io, request).map(|(promise, _)| promise)
}

fn submit_cancellable<'js>(
    ctx: Ctx<'js>,
    io: &IoHandle,
    request: impl FnOnce(OpId) -> IoRequest,
) -> rquickjs::Result<Object<'js>> {
    let (promise, id) = submit_operation(ctx.clone(), io, request)?;
    let operation = Object::new(ctx)?;
    operation.set("id", id.0)?;
    operation.set("promise", promise)?;
    Ok(operation)
}

fn submit_operation<'js>(
    ctx: Ctx<'js>,
    io: &IoHandle,
    request: impl FnOnce(OpId) -> IoRequest,
) -> rquickjs::Result<(Promise<'js>, OpId)> {
    let (promise, resolve, reject) = Promise::new(&ctx)?;
    let id = io.submit(request);
    let entry = Object::new(ctx.clone())?;
    entry.set("resolve", resolve)?;
    entry.set("reject", reject)?;
    pending(&ctx)?.set(id.0.to_string(), entry)?;
    Ok((promise, id))
}

pub fn settle(ctx: &Ctx<'_>, id: OpId, result: Result<Value, String>) -> rquickjs::Result<bool> {
    let pending = pending(ctx)?;
    let key = id.0.to_string();
    let Some(entry): Option<Object> = pending.get(&key)? else {
        return Ok(false);
    };
    pending.remove(&key)?;
    match result {
        Ok(value) => {
            let resolve: Function = entry.get("resolve")?;
            // HTTP bytes must use the QuickJS allocator so retained chunks count
            // against the isolate heap limit. External Vec-backed buffers bypass
            // that limit. Nested capability values keep their array representation.
            let value = match value {
                Value::Bytes(bytes) => {
                    TypedArray::<u8>::new_copy(ctx.clone(), &bytes)?.into_js(ctx)?
                }
                value => value_to_js(ctx, value)?,
            };
            resolve.call::<(rquickjs::Value<'_>,), ()>((value,))?;
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
    Ok(true)
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

/// Forget host promises owned by a request that has already returned.
///
/// They are intentionally not rejected: running their rejection handlers on
/// the next event-loop turn would let callbacks from the old request mutate a
/// reused isolate while it is serving a new request.
pub fn discard_operations(ctx: &Ctx<'_>, ids: &[OpId]) -> rquickjs::Result<()> {
    let pending = pending(ctx)?;
    for id in ids {
        pending.remove(id.0.to_string())?;
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
