//! Opt-in local P0/P1 measurement; no timing assertions. See the assessment report.
use std::time::Instant;
use tysel_engine::{HttpRequest, IsolateConfig};
use tysel_engine_qjs::{IsolatePool, eval};

fn config() -> IsolateConfig {
    IsolateConfig {
        memory_limit_bytes: 16 * 1024 * 1024,
        request_timeout_ms: 30_000,
        cpu_ms_per_turn: 5000,
    }
}
fn percentile(values: &mut [f64], p: usize) -> f64 {
    values.sort_by(f64::total_cmp);
    values[(values.len() - 1) * p / 100]
}
fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "cold".into());
    if mode == "encode" {
        let result = eval(r#"(()=>{
          const encoder=new TextEncoder(), rows=[];
          for(const unicode of [false,true])for(const small of [false,true]) {
            const source=(unicode?'中😀':'a').repeat(unicode?150000:1048576);
            const expected=encoder.encode(source);
            const capacity=small?64:expected.length;
            let expectedWritten=Math.min(expected.length,capacity);
            while(expectedWritten<expected.length && expectedWritten>0 && (expected[expectedWritten]&0xc0)===0x80)expectedWritten--;
            const target=new Uint8Array(capacity), rounds=small?1000:100;
            for(const mode of ['encode-copy','encodeInto']) {
              target.fill(99);
              const start=Date.now();let written=0;
              for(let i=0;i<rounds;i++) {
                if(mode==='encodeInto')written+=encoder.encodeInto(source,target).written;
                else {
                  const bytes=encoder.encode(source);let end=Math.min(bytes.length,capacity);
                  while(end<bytes.length && end>0 && (bytes[end]&0xc0)===0x80)end--;
                  target.set(bytes.subarray(0,end));written+=end;
                }
              }
              const milliseconds=Date.now()-start;
              if(written!==expectedWritten*rounds)throw new Error('encoding byte count mismatch');
              for(let i=0;i<capacity;i++)if(target[i] !== (i<expectedWritten?expected[i]:99))throw new Error('encoding output mismatch');
              rows.push({unicode,small,mode,milliseconds,rounds,written});
            }
          }
          return JSON.stringify(rows);
        })()"#, config()).unwrap();
        if let tysel_engine::Value::String(json) = result {
            println!("{json}");
        }
        return;
    }
    if mode == "cold" {
        let mut times = Vec::new();
        for i in 0..52 {
            let start = Instant::now();
            assert!(eval("true", config()).is_ok());
            if i >= 2 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
            }
        }
        println!(
            "{}",
            serde_json::json!({"mode":mode,"n":times.len(),"median_ms":percentile(&mut times,50),"p95_ms":percentile(&mut times,95)})
        );
        return;
    }
    if mode == "bootstrap" || mode == "bootstrap-streams" {
        let rt = rquickjs::Runtime::new().unwrap();
        let ctx = rquickjs::Context::full(&rt).unwrap();
        ctx.with(|ctx| ctx.eval::<(), _>("globalThis.tysel={};")).unwrap();
        rt.run_gc();
        let before = rt.memory_usage();
        let start = Instant::now();
        ctx.with(|ctx| ctx.eval::<(), _>(include_str!("../../../runtime-js/web-api/runtime.js")))
            .unwrap();
        if mode == "bootstrap-streams" {
            ctx.with(|ctx| {
                let host: rquickjs::Object = ctx.globals().get("tysel")?;
                host.set("_loadStreams", rquickjs::Function::new(ctx.clone(), load_streams)?)
            })
            .unwrap();
            ctx.with(|ctx| ctx.eval::<(), _>("void ReadableStream;")).unwrap();
        }
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        rt.run_gc();
        let after = rt.memory_usage();
        println!(
            "{}",
            serde_json::json!({"mode":mode,"milliseconds":elapsed,"js_used_bytes":after.memory_used_size-before.memory_used_size,"malloc_bytes":after.malloc_size-before.malloc_size})
        );
        return;
    }
    if mode == "stream-memory" {
        let mib: usize = std::env::args().nth(2).unwrap_or_else(|| "64".into()).parse().unwrap();
        let size = mib * 1024 * 1024;
        let source = format!(
            "export default {{fetch(){{let n=0;return new Response(new ReadableStream({{pull(c){{if(n==={size})c.close();else{{c.enqueue(new Uint8Array(16384));n+=16384;}}}}}},{{highWaterMark:0}}));}}}};"
        );
        let pool = IsolatePool::spawn(1, &source, config()).unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let start = Instant::now();
        let received = rt.block_on(async {
            let (_, mut chunks) = pool
                .dispatch(HttpRequest {
                    method: "GET".into(),
                    url: "http://local/".into(),
                    headers: vec![],
                    body: vec![],
                    request_id: 1,
                })
                .await
                .unwrap();
            let mut received = 0;
            while let Some(chunk) = chunks.recv().await {
                received += chunk.len();
            }
            received
        });
        assert_eq!(received, size);
        println!(
            "{}",
            serde_json::json!({"mode":mode,"mib":mib,"bytes":received,"milliseconds":start.elapsed().as_secs_f64()*1000.0,"configured_js_heap_bytes":config().memory_limit_bytes})
        );
        return;
    }
    let (source, count, expected) = match mode.as_str() {
        "small" => ("export default {fetch(){return Response.json({ok:true,value:42});}};".to_owned(), 5000, 22),
        "buffered" => ("export default {fetch(){return new Response(new Uint8Array(1048576));}};".to_owned(), 100, 1048576),
        "chunks" => ("export default {fetch(){return new Response(Array.from({length:64},()=>new Uint8Array(16384)));}};".to_owned(),100,1048576),
        "stream16k" | "stream1k" | "stream64k" => {
            let chunk = match mode.as_str(){"stream1k"=>1024,"stream64k"=>65536,_=>16384};
            (format!("export default {{fetch(){{let n=0;return new Response(new ReadableStream({{pull(c){{if(n===1048576)c.close();else{{c.enqueue(new Uint8Array({chunk}));n+={chunk};}}}}}},{{highWaterMark:0}}));}}}};"),100,1048576)
        },
        _ => panic!("unknown measurement"),
    };
    let pool = IsolatePool::spawn(1, &source, config()).unwrap();
    let mut times = Vec::new();
    for i in 0..count + 10 {
        let start = Instant::now();
        let (_, body) = pool
            .dispatch_sync(HttpRequest {
                method: "GET".into(),
                url: "http://local/".into(),
                headers: vec![],
                body: vec![],
                request_id: i,
            })
            .unwrap();
        assert_eq!(body.len(), expected);
        if i >= 10 {
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    let total: f64 = times.iter().sum();
    println!(
        "{}",
        serde_json::json!({"mode":mode,"n":count,"median_ms":percentile(&mut times,50),"p95_ms":percentile(&mut times,95),"total_ms":total,"requests_per_second":count as f64*1000.0/total,"mib_per_second":count as f64*expected as f64/1048576.0*1000.0/total})
    );
}

fn load_streams(ctx: rquickjs::Ctx<'_>) -> rquickjs::Result<rquickjs::Object<'_>> {
    ctx.eval(concat!(
        "(function(globalThis) {\n",
        include_str!("../../../runtime-js/web-api/vendor/web-streams-polyfill/polyfill.js"),
        "\nreturn globalThis; })(Object.create(globalThis))"
    ))
}
