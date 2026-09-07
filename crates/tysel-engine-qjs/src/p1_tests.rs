use super::*;

#[test]
fn p1_utilities_and_abort_composition() {
    let value = eval(r#"(async () => {
      const order = [];
      queueMicrotask(() => order.push('microtask'));
      Promise.resolve().then(() => order.push('promise'));
      order.push('sync'); await Promise.resolve();
      if (order.join() !== 'sync,microtask,promise') return false;
      const uuid = crypto.randomUUID();
      if (!/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(uuid) || uuid === crypto.randomUUID()) return false;
      let binary = ''; for (let i=0;i<256;i++) binary += String.fromCharCode(i);
      if (atob(btoa(binary)) !== binary || atob(' Y Q==\n') !== 'a' || atob('YQ') !== 'a') return false;
      for (const f of [() => btoa('中'), () => atob('a'), () => atob('a===')]) {
        try { f(); return false; } catch(e) { if (e.name !== 'InvalidCharacterError') return false; }
      }
      const a = new AbortController(), b = new AbortController();
      const combined = AbortSignal.any([a.signal, b.signal, a.signal]);
      let calls = 0; combined.addEventListener('abort', () => calls++);
      const reason = {}; b.abort(reason); a.abort('later');
      if (combined.reason !== reason || calls !== 1) return false;
      if (AbortSignal.any([]).aborted || AbortSignal.any([a.signal,b.signal]).reason !== 'later') return false;
      try { AbortSignal.any([a.signal, {}]); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
      for (const invalid of [{}, 42, null]) {
        try { AbortSignal.any(invalid); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
      }
      return true;
    })()"#, config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn p1_decoder_handles_every_split_bom_and_flush() {
    let value = eval(r#"(() => {
      const bytes = new TextEncoder().encode('\ufeff中😀\ufeffend');
      for (let split=0;split<=bytes.length;split++) {
        const decoder = new TextDecoder();
        const actual = decoder.decode(bytes.subarray(0,split),{stream:true}) + decoder.decode(bytes.subarray(split),{stream:true}) + decoder.decode();
        if (actual !== '中😀\ufeffend') return false;
      }
      const decoder = new TextDecoder(); let result = '';
      for (const byte of bytes) result += decoder.decode(new Uint8Array([byte]),{stream:true});
      if (result + decoder.decode() !== '中😀\ufeffend') return false;
      if (decoder.decode(new Uint8Array([226,130]),{stream:true}) !== '' || decoder.decode() !== '\ufffd') return false;
      const fatal = new TextDecoder('utf-8',{fatal:true});
      fatal.decode(new Uint8Array([240]),{stream:true});
      try { fatal.decode(); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
      if (fatal.decode(new Uint8Array([65])) !== 'A') return false;
      for (const input of [[224,128],[237,160],[244,144]]) {
        try { fatal.decode(new Uint8Array(input),{stream:true}); return false; } catch(e) { if (!(e instanceof TypeError)) return false; }
      }
      return true;
    })()"#, config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn p1_stream_pipe_transform_backpressure_and_cancel() {
    let value = eval(r#"(async () => {
      let next=0, inFlight=0, maxInFlight=0;
      const source = new ReadableStream({pull(c) { if (next===5) c.close(); else c.enqueue(next++); }});
      const output=[];
      const sink=new WritableStream({async write(v) { inFlight++; maxInFlight=Math.max(maxInFlight,inFlight); await tysel.sleep(1); output.push(v); inFlight--; }});
      await source.pipeThrough(new TransformStream({transform(v,c){c.enqueue(v*2);}})).pipeTo(sink);
      if(output.join()!=='0,2,4,6,8'||maxInFlight!==1||source.locked||sink.locked) return false;
      let reason;
      const canceled=new ReadableStream({cancel(r){reason=r;}},{highWaterMark:0});
      const reader=canceled.getReader(); const pending=reader.read();
      await reader.cancel('stop'); if(!(await pending).done||reason!=='stop') return false;
      reader.releaseLock();
      const bytes = new TextEncoder().encode('中😀'); let index=0;
      const decoded = new ReadableStream({pull(c){if(index===bytes.length)c.close();else c.enqueue(bytes.slice(index,++index));}}).pipeThrough(new TextDecoderStream());
      let text=''; for await(const chunk of decoded) text+=chunk;
      return text==='中😀';
    })()"#, config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn p1_body_stream_locks_and_disturbance() {
    let value=eval(r#"(async()=>{
      const response = new Response(new Uint8Array([65,66]));
      const reader=response.body.getReader();
      if(response.bodyUsed) return false;
      try { await response.text(); return false; } catch(e){ if(!(e instanceof TypeError))return false; }
      const chunk=await reader.read();
      if(!response.bodyUsed||String(chunk.value)!=='65,66') return false;
      reader.releaseLock();
      try { response.clone(); return false; } catch(e){if(!(e instanceof TypeError))return false;}
      const request = new Request('http://local/',{body:'hello'});
      let text=''; for await(const bytes of request.body) text+=new TextDecoder().decode(bytes);
      if(text!=='hello'||!request.bodyUsed) return false;
      const retained=new Uint8Array([65]);
      const supplied=new Response(new ReadableStream({start(c){c.enqueue(retained);c.close();}}));
      const result=new Uint8Array(await supplied.arrayBuffer());result[0]=66;
      return retained[0]===65;
    })()"#,config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[tokio::test(flavor = "multi_thread")]
async fn p1_set_cookie_roundtrips_without_comma_splitting() {
    let cookie = "a=1; Expires=Wed, 21 Oct 2030 07:28:00 GMT";
    let addr = spawn_origin(move |_| async move {
        Ok::<_, Infallible>(
            Response::builder()
                .header("set-cookie", cookie)
                .header("set-cookie", "b=2; HttpOnly")
                .body(http_body_util::Full::new(Bytes::from_static(b"ok")))
                .unwrap(),
        )
    });
    let source = format!(
        r#"export default {{async fetch(request){{
      const response=await fetch('http://{addr}/');
      const headers=new Headers(response.headers);
      const cookies=headers.getSetCookie(); cookies.push('bad');
      if(headers.getSetCookie().length!==2) throw new Error('not a snapshot');
      return new Response(response.body,{{headers}});
    }}}};"#
    );
    let pool = IsolatePool::spawn(1, &source, config()).unwrap();
    let (head, body) = pool
        .dispatch_response(IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: "http://local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        }))
        .await
        .unwrap();
    assert_eq!(
        head.headers
            .iter()
            .filter(|(k, _)| k == "set-cookie")
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>(),
        [cookie, "b=2; HttpOnly"]
    );
    let OutgoingHttpBody::CheckedStream { mut chunks, completion } = body else {
        panic!("expected stream")
    };
    let mut bytes = vec![];
    while let Some(chunk) = chunks.recv().await {
        bytes.extend(chunk);
    }
    completion.await.unwrap().unwrap();
    assert_eq!(bytes, b"ok");
}

#[tokio::test]
async fn p1_response_stream_backpressure_disconnect_and_reuse() {
    const SOURCE: &str = r#"let pulls=0,canceled=false; export default {fetch(r){
      if(r.url.endsWith('/status'))return Response.json({pulls,canceled});
      return new Response(new ReadableStream({pull(c){pulls++;c.enqueue(new Uint8Array(1024));},cancel(){canceled=true;}},{highWaterMark:0}));
    }};"#;
    let pool = IsolatePool::spawn(1, SOURCE, config()).unwrap();
    let request = |path: &str, id| {
        IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: format!("http://local/{path}"),
            headers: vec![],
            body: vec![],
            request_id: id,
        })
    };
    let (_, body) = pool.dispatch_response(request("stream", 0)).await.unwrap();
    let OutgoingHttpBody::CheckedStream { chunks, completion } = body else { panic!("stream") };
    tokio::time::sleep(Duration::from_millis(40)).await;
    drop(chunks);
    assert!(completion.await.unwrap().is_err());
    let (_, body) = pool.dispatch_response(request("status", 1)).await.unwrap();
    let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
    let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(status["canceled"], true);
    assert!(status["pulls"].as_u64().unwrap() <= STREAM_WINDOW as u64 + 1);
}

#[tokio::test]
async fn p1_response_stream_reports_failure_after_headers() {
    let pool=IsolatePool::spawn(1,r#"export default {fetch(){let i=0;return new Response(new ReadableStream({pull(c){if(i++===0)c.enqueue(new Uint8Array([65]));else throw new Error('broken stream');}},{highWaterMark:0}));}};"#,config()).unwrap();
    let (_, body) = pool
        .dispatch_response(IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: "http://local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        }))
        .await
        .unwrap();
    let OutgoingHttpBody::CheckedStream { mut chunks, completion } = body else { panic!("stream") };
    assert_eq!(chunks.recv().await.unwrap(), [65]);
    assert!(chunks.recv().await.is_none());
    assert!(completion.await.unwrap().unwrap_err().to_string().contains("broken stream"));
}

#[test]
fn p1_headers_validate_iterables_and_prototype_names() {
    let value=eval(r#"(()=>{
      const h=new Headers((function*(){yield ['x','a'];yield ['set-cookie','a=1'];yield ['set-cookie','b=2'];})());
      if(new Headers().get('constructor')!==null||h.get('x')!=='a'||new Headers(h).getSetCookie().length!==2)return false;
      h.set('set-cookie','c=3');if(h.getSetCookie().join()!=='c=3')return false;
      h.delete('set-cookie');if(h.getSetCookie().length)return false;
      try{h.set('bad name','x');return false;}catch(e){if(!(e instanceof TypeError))return false;}
      try{h.set('x','a\r\nb');return false;}catch(e){if(!(e instanceof TypeError))return false;}
      return true;
    })()"#,config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[tokio::test]
async fn p1_request_stream_cancel_releases_pending_native_read() {
    let pool = IsolatePool::spawn(
        1,
        r#"export default {async fetch(request){
      const reader=request.body.getReader();const pending=reader.read();
      await reader.cancel('stop');
      return Response.json({done:(await pending).done,used:request.bodyUsed});
    }};"#,
        config(),
    )
    .unwrap();
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_WINDOW);
    let (_, body) = pool
        .dispatch_response(IncomingHttp {
            method: "POST".into(),
            url: "http://local/".into(),
            headers: vec![],
            body: rx,
            ws_in: None,
            ws_out: None,
            request_id: 0,
        })
        .await
        .unwrap();
    let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
    assert_eq!(String::from_utf8(bytes).unwrap(), r#"{"done":true,"used":true}"#);
    tokio::time::timeout(Duration::from_secs(1), tx.closed())
        .await
        .expect("native inbound reader released");
}

#[tokio::test(flavor = "multi_thread")]
async fn p1_fetch_body_reader_cancel_does_not_poison_other_fetch() {
    let addr = serve_slow_body();
    let second = serve_bytes(Bytes::from_static(b"ok"));
    let source = format!(
        r#"(async()=>{{
      const response=await fetch('http://{addr}/');
      const reader=response.body.getReader();const pending=reader.read();
      await reader.cancel();if(!(await pending).done||!response.bodyUsed)return false;
      return await (await fetch('http://{second}/')).text()==='ok';
    }})()"#
    );
    assert_eq!(
        tokio::task::spawn_blocking(move || eval(&source, config())).await.unwrap().unwrap(),
        Value::Bool(true)
    );
}

#[tokio::test]
async fn p1_stale_request_body_cannot_read_next_request() {
    let pool = IsolatePool::spawn(
        1,
        r#"let stale;export default{async fetch(r){
      if(!stale){stale=r.body;return new Response('stored');}
      try{await stale.getReader().read();return new Response('bad');}catch{}
      return new Response(await r.text());
    }};"#,
        config(),
    )
    .unwrap();
    for (id, expected) in [(0, b"stored".as_slice()), (1, b"new".as_slice())] {
        let (_, body) = pool
            .dispatch_response(IncomingHttp::from(HttpRequest {
                method: "POST".into(),
                url: "http://local/".into(),
                headers: vec![],
                body: b"new".to_vec(),
                request_id: id,
            }))
            .await
            .unwrap();
        let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
        assert_eq!(bytes, expected);
    }
}

#[test]
fn p1_pipe_abort_and_tee_lifecycle() {
    let value=eval(r#"(async()=>{
      const controller=new AbortController();let sourceReason,sinkReason;
      const source=new ReadableStream({cancel(reason){sourceReason=reason;}},{highWaterMark:0});
      const sink=new WritableStream({abort(reason){sinkReason=reason;}});
      const pipe=source.pipeTo(sink,{signal:controller.signal});controller.abort('stop');
      try{await pipe;return false;}catch(e){if(e!=='stop')return false;}
      if(sourceReason!=='stop'||sinkReason!=='stop'||source.locked||sink.locked)return false;
      const [a,b]=new ReadableStream({start(c){c.enqueue(7);c.close();}}).tee();
      const collect=async stream=>{let result=[];for await(const x of stream)result.push(x);return result.join();};
      return (await Promise.all([collect(a),collect(b)])).join()==='7,7';
    })()"#,config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[test]
fn p1_microtask_exceptions_are_reported() {
    let error=eval("(async()=>{queueMicrotask(()=>{throw new Error('microtask failure')});await tysel.sleep(1);})()",config()).unwrap_err();
    assert!(error.to_string().contains("microtask failure"));
}

#[test]
fn p1_any_cannot_be_blocked_by_public_event_listeners() {
    let value=eval(r#"(()=>{const a=new AbortController();a.signal.addEventListener('abort',e=>e.stopImmediatePropagation());const combined=AbortSignal.any([a.signal]);a.abort('reason');return combined.aborted&&combined.reason==='reason';})()"#,config()).unwrap();
    assert_eq!(value, Value::Bool(true));
}

#[tokio::test]
async fn p1_lazily_opened_stale_body_cannot_read_next_request() {
    let pool = IsolatePool::spawn(
        1,
        r#"let stale;export default{async fetch(r){
      if(!stale){stale=r;return new Response('stored');}
      try{await new Request(stale).body.getReader().read();return new Response('bad copy');}catch{}
      try{await stale.body.getReader().read();return new Response('bad');}catch{}
      return new Response(await r.text());
    }};"#,
        config(),
    )
    .unwrap();
    for (id, expected) in [(0, b"stored".as_slice()), (1, b"new".as_slice())] {
        let (_, body) = pool
            .dispatch_response(IncomingHttp::from(HttpRequest {
                method: "POST".into(),
                url: "http://local/".into(),
                headers: vec![],
                body: b"new".to_vec(),
                request_id: id,
            }))
            .await
            .unwrap();
        let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
        assert_eq!(bytes, expected);
    }
}

#[test]
fn p1_stream_queues_obey_isolate_heap_limit() {
    let error=eval(r#"(()=>{const stream=new ReadableStream({start(c){for(let i=0;i<64;i++)c.enqueue(new Uint8Array(128*1024));}});return stream.locked;})()"#,IsolateConfig{memory_limit_bytes:2*1024*1024,..config()}).unwrap_err();
    assert!(matches!(
        error,
        EngineError::Interrupted(InterruptReason::MemoryLimit) | EngineError::Isolate(_)
    ));
}

#[tokio::test]
async fn p1_retained_stream_releases_output_sender_on_timeout() {
    let pool=IsolatePool::spawn(1,r#"let retained;export default{fetch(){retained=new ReadableStream({pull(){return new Promise(()=>{});}});return new Response(retained);}};"#,IsolateConfig{request_timeout_ms:100,..config()}).unwrap();
    let (_, body) = pool
        .dispatch_response(IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: "http://local/".into(),
            headers: vec![],
            body: vec![],
            request_id: 0,
        }))
        .await
        .unwrap();
    let OutgoingHttpBody::CheckedStream { mut chunks, completion } = body else { panic!("stream") };
    assert!(
        tokio::time::timeout(Duration::from_secs(1), chunks.recv())
            .await
            .expect("sender released")
            .is_none()
    );
    assert!(matches!(
        completion.await.unwrap(),
        Err(EngineError::Interrupted(InterruptReason::Timeout))
    ));
}

#[tokio::test]
async fn p1_idle_stream_disconnect_cancels_without_waiting_for_source_cleanup() {
    let pool = IsolatePool::spawn(
        1,
        r#"let canceled=false;
      export default{fetch(r){
        if(r.url.endsWith('/status'))return Response.json({canceled});
        return new Response(new ReadableStream({
          pull(){return new Promise(()=>{});},
          cancel(){canceled=true;return new Promise(()=>{});}
        },{highWaterMark:0}));
      }};"#,
        IsolateConfig { request_timeout_ms: 5000, ..config() },
    )
    .unwrap();
    let request = |path: &str, id| {
        IncomingHttp::from(HttpRequest {
            method: "GET".into(),
            url: format!("http://local/{path}"),
            headers: vec![],
            body: vec![],
            request_id: id,
        })
    };
    let (_, body) = pool.dispatch_response(request("stream", 1)).await.unwrap();
    let OutgoingHttpBody::CheckedStream { chunks, completion } = body else { panic!("stream") };
    drop(chunks);
    let result = tokio::time::timeout(Duration::from_secs(1), completion)
        .await
        .expect("disconnect must not wait for the request deadline")
        .unwrap();
    assert!(result.is_err());
    let (_, body) =
        tokio::time::timeout(Duration::from_secs(1), pool.dispatch_response(request("status", 2)))
            .await
            .expect("worker must be reusable")
            .unwrap();
    let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["canceled"], true);
}

#[test]
fn p1_any_marks_all_dependents_before_reentrant_callbacks() {
    let result = eval(
        r#"(()=>{
      const a=new AbortController(), b=new AbortController();
      const first=AbortSignal.any([a.signal]);
      const second=AbortSignal.any([a.signal,b.signal]);
      const nested=AbortSignal.any([second,b.signal]);
      let marked=false;
      first.addEventListener('abort',()=>{marked=second.aborted&&nested.aborted;b.abort('b');});
      a.abort('a');
      return marked&&second.reason==='a'&&nested.reason==='a';
    })()"#,
        config(),
    )
    .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[tokio::test]
async fn p1_any_unused_dependents_are_collectible_across_requests() {
    let pool = IsolatePool::spawn(
        1,
        r#"const root=new AbortController();
      const observed=AbortSignal.any([AbortSignal.any([root.signal])]);
      export default{fetch(r){
        if(r.url.endsWith('/abort')){root.abort('done');return new Response(observed.reason);}
        for(let i=0;i<64;i++)AbortSignal.any([root.signal]);
        return new Response('ok');
      }};"#,
        IsolateConfig { memory_limit_bytes: 4 * 1024 * 1024, cpu_ms_per_turn: 500, ..config() },
    )
    .unwrap();
    for id in 0..401 {
        let last = id == 400;
        let (_, body) = pool
            .dispatch_response(IncomingHttp::from(HttpRequest {
                method: "GET".into(),
                url: if last { "http://local/abort" } else { "http://local/" }.into(),
                headers: vec![],
                body: vec![],
                request_id: id,
            }))
            .await
            .expect("discarded dependents must not exhaust the heap");
        let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
        assert_eq!(bytes, if last { b"done".as_slice() } else { b"ok".as_slice() });
    }
}

#[tokio::test]
async fn p1_sync_handler_drains_microtasks_within_its_request() {
    let pool = IsolatePool::spawn(
        1,
        r#"let completed=false;
      export default {fetch(r){
        if(r.url.endsWith('/status'))return new Response(String(completed));
        queueMicrotask(()=>{completed=true;});
        return new Response('queued');
      }};"#,
        config(),
    )
    .unwrap();
    for (id, path, expected) in [(1, "queue", "queued"), (2, "status", "true")] {
        let (_, body) = pool
            .dispatch_response(IncomingHttp::from(HttpRequest {
                method: "GET".into(),
                url: format!("http://local/{path}"),
                headers: vec![],
                body: vec![],
                request_id: id,
            }))
            .await
            .unwrap();
        let OutgoingHttpBody::Buffered(bytes) = body else { panic!("buffer") };
        assert_eq!(bytes, expected.as_bytes());
    }
}

#[test]
fn p1_buffered_bodies_do_not_initialize_streams() {
    let result = eval(r#"(async()=>{
      const lazy=()=>typeof Object.getOwnPropertyDescriptor(globalThis,'ReadableStream').get==='function';
      if(!lazy())return false;
      const response=Response.json({ok:true});
      if(await response.text()!=='{"ok":true}'||!lazy())return false;
      await new Request('http://local/',{body:new Uint8Array([65])}).arrayBuffer();
      if(!lazy())return false;
      const stream=new Response('A').body;
      const constructor=ReadableStream;
      if(lazy()||constructor!==ReadableStream||!(stream instanceof constructor))return false;
      const reader=stream.getReader();
      return (await reader.read()).value[0]===65;
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_stream_globals_preserve_identity_and_writable_assignment() {
    let result = eval(
        r#"(()=>{
      // Assigning before another constructor is read must not be overwritten
      // later when the remaining globals are used.
      const custom=function(){};
      WritableStream=custom;
      const original=ReadableStream;
      return WritableStream===custom && ReadableStream===original &&
        Object.getOwnPropertyDescriptor(globalThis,'ReadableStream').writable===true &&
        typeof TransformStream==='function';
    })()"#,
        config(),
    )
    .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_lazy_streams_preserve_defined_and_deleted_globals() {
    let result = eval(r#"(async()=>{
      const custom=function CustomReadable() {};
      Object.defineProperty(globalThis,'ReadableStream',{value:custom,writable:true,configurable:true});
      Object.defineProperty(globalThis,'WritableStream',{value:42,configurable:false});
      delete globalThis.ByteLengthQueuingStrategy;
      const transform=new TransformStream();
      if(ReadableStream!==custom || WritableStream!==42 || 'ByteLengthQueuingStrategy' in globalThis)return false;
      // Body branding uses the private constructor, even when the public name
      // was replaced before the first load.
      const response=new Response(transform.readable);
      const writer=transform.writable.getWriter();
      const reading=response.text();
      await writer.write(new Uint8Array([65])); await writer.close();
      return await reading==='A' && typeof CountQueuingStrategy==='function';
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_lazy_streams_allow_nonconfigurable_accessor() {
    let result = eval(
        r#"(()=>{
      Object.defineProperty(globalThis,'ReadableStream',{configurable:false});
      const constructor=ReadableStream;
      const stream=new constructor();
      return ReadableStream===constructor && stream instanceof constructor &&
        typeof TransformStream==='function' && new Response(stream).body===stream;
    })()"#,
        config(),
    )
    .unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_body_read_errors_do_not_wait_for_cancel() {
    let result = eval(r#"(async()=>{
      for(const method of ['text','json','arrayBuffer']) {
        for(const cancellation of ['pending','reject','throw']) {
          let canceled=false;
          const stream=new ReadableStream({
            start(c){c.enqueue('invalid HTTP chunk');},
            cancel(){
              canceled=true;
              if(cancellation==='throw')throw new Error('cleanup failure');
              return cancellation==='pending' ? new Promise(()=>{}) : Promise.reject(new Error('cleanup failure'));
            },
          });
          const response=new Response(stream);
          const result=await Promise.race([
            response[method]().then(()=> 'success',e=>e instanceof TypeError ? 'TypeError' : e.message),
            tysel.sleep(30).then(()=> 'timeout'),
          ]);
          if(result!=='TypeError'||!canceled||stream.locked||!response.bodyUsed)return false;
        }
      }
      return true;
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_url_preserves_paths_ports_and_query_bytes() {
    let result=eval(r#"(()=>{
      const u=new URL('https://example.com:00443/a//b?q=%E4%B8%AD+%FF');
      if(u.pathname!=='/a//b'||u.port!==''||u.host!=='example.com'||u.searchParams.get('q')!=='中 �')return false;
      if(new URL('../c//',u).pathname!=='/a/c//')return false;
      u.pathname='/a//%2e/b/..';if(u.pathname!=='/a//')return false;
      u.port='0080';u.protocol='http';if(u.port!==''||u.origin!=='http://example.com')return false;
      u.host='[::1]:00080';if(u.host!=='[::1]')return false;
      const params=u.searchParams;u.search='?q=%EF%BB%BF%FF+%41';
      if(params!==u.searchParams||params.get('q')!=='\ufeff� A')return false;
      params.set('q','\ud800😀');
      return params.get('q')==='�😀' && u.search==='?q=%EF%BF%BD%F0%9F%98%80';
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_consumed_buffered_body_preserves_lazy_stream_state() {
    let result=eval(r#"(async()=>{
      const first=new Response('A');await first.text();
      if(typeof Object.getOwnPropertyDescriptor(globalThis,'ReadableStream').get!=='function')return false;
      for(const kind of ['response','request']) for(const method of ['text','json','arrayBuffer']) {
        for(const early of [false,true])for(const binary of [false,true]) {
          const data=binary?new TextEncoder().encode('{"ok":true}'):'{"ok":true}';
          const owner=kind==='response'?new Response(data):new Request('http://local/',{method:'POST',body:data});
          const initial=early?owner.body:null;
          await owner[method]();
          const body=owner.body;
          if(!owner.bodyUsed||body!==owner.body||body.locked||(early&&body!==initial))return false;
          try{new Response(body);return false;}catch(e){if(!(e instanceof TypeError))return false;}
          try{owner.clone();return false;}catch(e){if(!(e instanceof TypeError))return false;}
          const reader=body.getReader();
          if(!(await reader.read()).done)return false;
          reader.releaseLock();
          try{await owner.text();return false;}catch(e){if(!(e instanceof TypeError))return false;}
        }
      }
      const chunks=new Response([new Uint8Array([65]),'B']);await chunks.arrayBuffer();
      try{new Response(chunks.body);return false;}catch(e){if(!(e instanceof TypeError))return false;}
      return true;
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn p1_headers_live_iteration_tracks_mutation() {
    let result=eval(r#"(()=>{
      const h=new Headers({a:'1',b:'2',c:'3'}),seen=[];
      h.forEach((value,key)=>{seen.push([key,value]);if(key==='a'){h.delete('b');h.set('c','updated');h.append('d','4');}});
      if(JSON.stringify(seen)!=='[["a","1"],["c","updated"],["d","4"]]')return false;
      const entries=h.entries();entries.next();h.delete('c');
      const copied=new Headers(entries);if(copied.has('c')||copied.get('d')!=='4')return false;
      const cookies=new Headers([['a','1'],['set-cookie','first=1'],['set-cookie','second=2'],['z','last']]);
      const iterator=cookies.entries();iterator.next();iterator.next();cookies.delete('set-cookie');
      // Cursor indexes the current flattened list; it is already past z.
      if(!iterator.next().done)return false;
      cookies.append('zz','after end');
      const resumed=iterator.next();
      return !resumed.done && resumed.value[0]==='zz' && iterator.next().done;
    })()"#,config()).unwrap();
    assert_eq!(result, Value::Bool(true));
}
