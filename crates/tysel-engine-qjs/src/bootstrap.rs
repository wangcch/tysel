use std::ffi::CString;
use std::sync::OnceLock;

use rquickjs::{Ctx, Error, Result, qjs};

/// A fixed, embedded script's bytecode, shared only within this executable.
/// Each evaluation decodes fresh objects into the caller's runtime. No JS
/// values, contexts, host bindings or tenant-supplied source enter the cache.
pub(crate) struct CompiledScript {
    source: &'static str,
    bytecode: OnceLock<Box<[u8]>>,
}

impl CompiledScript {
    pub(crate) const fn new(source: &'static str) -> Self {
        Self { source, bytecode: OnceLock::new() }
    }

    pub(crate) fn eval(&self, ctx: &Ctx<'_>) -> Result<()> {
        // SAFETY: the context is live and locked by Context::with. Compiled
        // values are owned by this context and consumed exactly once below.
        // The only deserialized bytes were written by this same engine from
        // this fixed source; they are never read from disk or supplied by users.
        #[allow(unsafe_code)]
        unsafe {
            let raw = ctx.as_raw().as_ptr();
            qjs::JS_UpdateStackTop(qjs::JS_GetRuntime(raw));
            let compiled = if let Some(bytes) = self.bytecode.get() {
                qjs::JS_ReadObject(
                    raw,
                    bytes.as_ptr(),
                    bytes.len() as _,
                    qjs::JS_READ_OBJ_BYTECODE as i32,
                )
            } else {
                let source = CString::new(self.source)?;
                let flags = qjs::JS_EVAL_TYPE_GLOBAL
                    | qjs::JS_EVAL_FLAG_STRICT
                    | qjs::JS_EVAL_FLAG_COMPILE_ONLY;
                let compiled = qjs::JS_Eval(
                    raw,
                    source.as_ptr(),
                    self.source.len() as _,
                    c"eval_script".as_ptr(),
                    flags as i32,
                );
                if qjs::JS_IsException(compiled) {
                    return Err(Error::Exception);
                }
                let mut len = 0;
                let buffer =
                    qjs::JS_WriteObject(raw, &mut len, compiled, qjs::JS_WRITE_OBJ_BYTECODE as i32);
                if buffer.is_null() {
                    qjs::JS_FreeValue(raw, compiled);
                    return Err(Error::Exception);
                }
                let bytes = std::slice::from_raw_parts(buffer, len as usize).into();
                qjs::js_free(raw, buffer.cast());
                // Concurrent first users may compile independently. A loser
                // drops its bytes instead of blocking another isolate's budget.
                let _ = self.bytecode.set(bytes);
                compiled
            };
            if qjs::JS_IsException(compiled) {
                return Err(Error::Exception);
            }
            // JS_EvalFunction consumes compiled, including on an exception.
            let result = qjs::JS_EvalFunction(raw, compiled);
            if qjs::JS_IsException(result) {
                return Err(Error::Exception);
            }
            qjs::JS_FreeValue(raw, result);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use rquickjs::{Context, Runtime};

    use super::*;

    #[test]
    fn cached_code_uses_fresh_globals_and_bindings() {
        let script = CompiledScript::new(
            "globalThis.state = { value: hostValue() }; globalThis.getValue = () => state.value;",
        );
        for expected in [17, 29, 43] {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| {
                assert!(ctx.eval::<bool, _>("Object.prototype.leaked === undefined").unwrap());
                ctx.globals()
                    .set(
                        "hostValue",
                        rquickjs::Function::new(ctx.clone(), move || expected).unwrap(),
                    )
                    .unwrap();
                script.eval(&ctx).unwrap();
                assert_eq!(ctx.eval::<i32, _>("getValue()").unwrap(), expected);
                ctx.eval::<(), _>("state.value = -1; Object.prototype.leaked = true;").unwrap();
            });
        }
        assert!(script.bytecode.get().is_some());
    }

    #[test]
    fn concurrent_first_use_keeps_contexts_independent() {
        let script =
            Arc::new(CompiledScript::new("globalThis.value = (globalThis.value || 0) + 1;"));
        let ready = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let script = script.clone();
                let ready = ready.clone();
                std::thread::spawn(move || {
                    let runtime = Runtime::new().unwrap();
                    let context = Context::full(&runtime).unwrap();
                    ready.wait();
                    context.with(|ctx| {
                        script.eval(&ctx).unwrap();
                        assert_eq!(ctx.eval::<i32, _>("value").unwrap(), 1);
                    });
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    }

    #[test]
    fn failed_compilation_does_not_poison_the_cache() {
        let script = CompiledScript::new("globalThis.value = 42;");
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        runtime.set_memory_limit(1);
        context.with(|ctx| {
            assert!(script.eval(&ctx).is_err());
            assert!(script.bytecode.get().is_none());
            let _ = ctx.catch();
        });
        runtime.set_memory_limit(16 * 1024 * 1024);
        context.with(|ctx| {
            script.eval(&ctx).unwrap();
            assert_eq!(ctx.eval::<i32, _>("value").unwrap(), 42);
        });
    }

    #[test]
    fn cached_execution_still_obeys_interrupts() {
        let script = CompiledScript::new("if (globalThis.spin) { for (;;) {} }");
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| {
            script.eval(&ctx).unwrap();
            ctx.globals().set("spin", true).unwrap();
        });
        runtime.set_interrupt_handler(Some(Box::new(|| true)));
        context.with(|ctx| {
            assert!(script.eval(&ctx).is_err());
            let _ = ctx.catch();
        });
        runtime.set_interrupt_handler(None);
    }

    #[test]
    fn cached_decode_obeys_each_runtimes_memory_limit() {
        let script = CompiledScript::new("globalThis.value = { answer: 42 };");
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        context.with(|ctx| script.eval(&ctx).unwrap());
        drop(context);
        drop(runtime);

        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();
        runtime.set_memory_limit(1);
        context.with(|ctx| {
            assert!(script.eval(&ctx).is_err());
            let _ = ctx.catch();
        });
        runtime.set_memory_limit(16 * 1024 * 1024);
        context.with(|ctx| {
            script.eval(&ctx).unwrap();
            assert_eq!(ctx.eval::<i32, _>("value.answer").unwrap(), 42);
        });
    }

    #[test]
    fn web_api_objects_are_not_shared_between_runtimes() {
        let script = CompiledScript::new(include_str!("../../../runtime-js/web-api/runtime.js"));
        for _ in 0..3 {
            let runtime = Runtime::new().unwrap();
            let context = Context::full(&runtime).unwrap();
            context.with(|ctx| {
                script.eval(&ctx).unwrap();
                assert!(ctx.eval::<bool, _>("Request.prototype.leaked === undefined").unwrap());
                assert_eq!(
                    ctx.eval::<String, _>("new Request('https://example.test').redirect").unwrap(),
                    "follow"
                );
                ctx.eval::<(), _>("Request.prototype.leaked = true; globalThis.Request = null;")
                    .unwrap();
            });
        }
        eprintln!("Web API compiled cache: {} bytes", script.bytecode.get().unwrap().len());
    }
}
