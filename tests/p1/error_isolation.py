"""Regression gate: task errors must not stop the service or acknowledge failed retries."""
import sys,time,json
from review_regressions import Service
settings='''[server]
listen="127.0.0.1:{port}"
[limits]
request_timeout_ms=1000
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
'''
source='''export default {durable:{async bad(ctx){await ctx.sleep(1);return await ctx.effect('x'.repeat(257),async()=>42)},async business(){throw new Error('business')},async good(){return 42}},fetch(req){const name=new URL(req.url).pathname.slice(1);return name?Response.json(tysel.durable.start(name,null,{idempotencyKey:name})):new Response('ok')}}'''
with Service('final-review',source,settings) as s:
 result={'fixture':str(s.root),'business_first':s.request('/business'),'business_retry':s.request('/business'),'good_before':s.request('/good'),'bad':s.request('/bad')}
 time.sleep(1)
 try: result['good_after']=s.request('/good')
 except OSError as e: result['good_after']={'error':str(e)}
 result['process_exit']=s.process.poll()
 result['log']=(s.root/'service.log').read_text()
 assert result['business_first'][0] == 500
 assert result['business_retry'][0] == 500
 assert result['bad'][0] == 200
 assert result['process_exit'] is None
 assert result['good_after'][0] == 200
 assert json.loads(result['good_after'][1])['value'] == 42
 print(json.dumps(result))
 (s.root/'results.json').write_text(json.dumps(result,indent=2))
