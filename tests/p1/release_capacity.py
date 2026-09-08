"""Closed-loop capacity probe; not an SLO or open-loop saturation certification."""
import concurrent.futures
import http.client
import json
import os
import statistics
import sqlite3
import subprocess
import threading
import time
from review_regressions import Service, BIN_DIR

assert BIN_DIR.name == 'release', 'set TYSEL_GATE_BIN_DIR to optimized binaries'
SETTINGS = '''[server]
listen="127.0.0.1:{port}"
workers=8
[limits]
request_timeout_ms=5000
cpu_ms_per_turn=1000
memory_mb=128
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
'''
SOURCE = '''export default {
 durable: {
  async one(ctx) {return await ctx.effect('one',async()=>42)},
  async ten(ctx) {let n=0;for(let i=0;i<10;i++)n+=await ctx.effect('e'+i,async()=>1);return n},
  async payload(ctx) {return await ctx.effect('payload',async()=>'x'.repeat(65536))}
 },
 fetch(req) {const path=new URL(req.url).pathname.slice(1);
  if(!path) return new Response('ready');
  if(path==='http') return Response.json({ok:true});
  return Response.json(tysel.durable.start(path,null));
 }
};'''

def phase(service, workload, clients, seconds):
    barrier=threading.Barrier(clients+1)
    def client():
        conn=http.client.HTTPConnection('127.0.0.1',service.port,timeout=12)
        samples=[];statuses={};barrier.wait();deadline=time.monotonic()+seconds
        while time.monotonic()<deadline:
            start=time.monotonic()
            try:
                conn.request('GET','/'+workload)
                response=conn.getresponse();body=response.read()
                status=str(response.status)
                if response.status==200:
                    data=json.loads(body)
                    if workload!='http':
                        if data.get('status')=='accepted' and data.get('taskId'):status='accepted'
                        elif data.get('status')!='completed' or data.get('value')!=({'one':42,'ten':10,'payload':'x'*65536}[workload]):status='invalid_result'
            except Exception as error:
                status=type(error).__name__;conn.close();conn=http.client.HTTPConnection('127.0.0.1',service.port,timeout=12)
            statuses[status]=statuses.get(status,0)+1;samples.append((time.monotonic()-start)*1000)
        conn.close();return samples,statuses
    with concurrent.futures.ThreadPoolExecutor(clients) as pool:
        futures=[pool.submit(client) for _ in range(clients)]
        start=time.monotonic();barrier.wait();rss=[]
        while not all(f.done() for f in futures):
            sample=subprocess.run(['ps','-p',str(service.process.pid),'-o','rss=,%cpu='],capture_output=True,text=True).stdout.split()
            if len(sample)==2:rss.append({'seconds':round(time.monotonic()-start,2),'rss_kib':int(sample[0]),'ps_cpu_percent':float(sample[1])})
            time.sleep(.5)
        elapsed=time.monotonic()-start;samples=[];statuses={}
        for future in futures:
            values,counts=future.result();samples+=values
            for status,count in counts.items():statuses[status]=statuses.get(status,0)+count
    samples.sort()
    completed = None
    if workload != 'http':
        with sqlite3.connect('file:'+str(service.root/'data/durable-events.db')+'?mode=ro',uri=True) as db:
            deadline=time.monotonic()+10
            while db.execute('SELECT active_count FROM durable_program_stats').fetchone()[0]:
                assert time.monotonic()<deadline, 'admitted tasks did not drain'
                time.sleep(.02)
            total=db.execute('SELECT count(*) FROM durable_completions').fetchone()[0]
            completed=total-getattr(service,'completed_count',0)
            service.completed_count=total
            assert completed==statuses.get('200',0)+statuses.get('accepted',0), 'admission/completion mismatch'
    return {'completed_tasks':completed,'workload':workload,'clients':clients,'seconds':elapsed,'requests':len(samples),'rps':len(samples)/elapsed,'status_counts':statuses,'p50_ms':statistics.median(samples),'p95_ms':samples[int(len(samples)*.95)],'p99_ms':samples[int(len(samples)*.99)],'rss_samples':rss}

results=[]
for workload in ['http','one','ten','payload']:
    with Service('capacity-'+workload,SOURCE,SETTINGS) as service:
        for clients in [1,4,16,32]:
            result=phase(service,workload,clients,int(os.environ.get('TYSEL_PHASE_SECONDS','5')))
            result['fixture']=str(service.root);results.append(result)
            print(json.dumps({k:v for k,v in result.items() if k!='rss_samples'}),flush=True)
            (service.root/'results.json').write_text(json.dumps(results,indent=2))
        if workload=='one':
            result=phase(service,workload,4,int(os.environ.get('TYSEL_SOAK_SECONDS','60')))
            result['fixture']=str(service.root);result['soak']=True;results.append(result)
            print(json.dumps({k:v for k,v in result.items() if k!='rss_samples'}),flush=True)
output=os.environ.get('TYSEL_CAPACITY_OUTPUT','/tmp/tysel-release-capacity.json')
with open(output,'w') as f:json.dump(results,f,indent=2)
print('RESULT_PATH='+output,flush=True)
