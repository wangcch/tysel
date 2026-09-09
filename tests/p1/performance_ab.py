"""macOS loopback A/B benchmark. Service CPU/RSS and disk I/O use proc_pid_rusage.

Timings retain default SQLite journaling. Child logs are drained into parent
memory so child-attributed storage writes are database/journal I/O, not logs.
Separate fixed-count probes also report retained database file growth.
Each phase starts a fresh process/database, warms up, then uses closed-loop clients.
"""
import concurrent.futures as futures
import ctypes as C
import hashlib
import http.client
import http.server
import json
import math
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import statistics
import subprocess
import sys
import threading
import time

ROOT = Path(os.environ.get('TYSEL_AB_ROOT', '/tmp/tysel-perf-20260909'))
OUT = ROOT / 'measurements'
OUT.mkdir(exist_ok=True)
SECONDS = float(os.environ.get('TYSEL_AB_SECONDS', '10'))
REPEATS = int(os.environ.get('TYSEL_AB_REPEATS', '3'))
STARTUP_TIMEOUT = 30
CLIENT_READY_TIMEOUT = 30
STOP_TIMEOUT = 15
BEFORE = Path(os.environ.get('TYSEL_AB_BEFORE_BIN',str(ROOT / 'before-bin')))
AFTER = ROOT / 'after-bin'
FIELDS = 'user_time system_time pkg_idle_wkups interrupt_wkups pageins wired_size resident_size phys_footprint proc_start_abstime proc_exit_abstime child_user_time child_system_time child_pkg_idle_wkups child_interrupt_wkups child_pageins child_elapsed_abstime diskio_bytesread diskio_byteswritten'.split()
class Usage(C.Structure):
    _fields_ = [('uuid', C.c_uint8 * 16)] + [(k, C.c_uint64) for k in FIELDS]
lib = C.CDLL('/usr/lib/libproc.dylib', use_errno=True)
lib.proc_pid_rusage.argtypes = [C.c_int, C.c_int, C.c_void_p]
class Timebase(C.Structure):
    _fields_=[('numer',C.c_uint32),('denom',C.c_uint32)]
timebase=Timebase()
C.CDLL('/usr/lib/libSystem.B.dylib').mach_timebase_info(C.byref(timebase))
TICK_NS=timebase.numer/timebase.denom
def usage(pid):
    u = Usage()
    if lib.proc_pid_rusage(pid, 2, C.byref(u)) != 0:
        raise OSError(C.get_errno(), 'proc_pid_rusage')
    return {k: getattr(u, k) for k in FIELDS}

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

SOURCE = '''export default {durable:{
 async one(ctx){return await ctx.effect('one',async()=>42)},
 async ten(ctx){let n=0;for(let i=0;i<10;i++)n+=await ctx.effect('e'+i,async()=>1);return n},
 async payload(ctx){return await ctx.effect('payload',async()=>'x'.repeat(65536))}
},fetch(req){const p=new URL(req.url).pathname.slice(1);if(!p)return new Response('ready');
 if(p==='http')return Response.json({ok:true});return Response.json(tysel.durable.start(p,null));}};'''

class Provider(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_GET(self):
        body=json.dumps({'full_name':'owner/repo','default_branch':'main'}).encode()
        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def do_POST(self):
        self.rfile.read(int(self.headers.get('content-length', '0')))
        body=json.dumps({'id':'fake', 'output_text':json.dumps({'type':'final','output':{'summary':'ready'}}), 'usage':{'input_tokens':12,'output_tokens':24}}).encode()
        self.send_response(200);self.send_header('Content-Type','application/json');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)

class Service:
    def __init__(self, version, workload, label):
        self.root=OUT/label;self.root.mkdir();self.port=port();self.workload=workload
        app=workload=='app'
        # Default app comparison isolates application changes on one runtime.
        # Opt in when comparing runtime changes against identical application source.
        same_runtime = app and os.environ.get('TYSEL_AB_APP_COMPARE_RUNTIME') != '1'
        bin_dir=AFTER if same_runtime or version=='after' else BEFORE
        env={k:v for k,v in os.environ.items() if not k.startswith(('TYSEL_','OTEL_'))}
        env.update(OTEL_SDK_DISABLED='true', TYSEL_WORKER=str(bin_dir/'tysel-worker'))
        if app:
            shutil.copytree(ROOT/('app-'+version)/'src',self.root/'src')
            (self.root/'src/entry.ts').write_text("import {createApp} from './app.js';export default createApp({githubBaseUrl:"+json.dumps(PROVIDER_URL)+"});")
            source='src/entry.ts';database='agent-runner.db'
            env.update(OPENAI_API_KEY='fake-key',GITHUB_TOKEN='fake-key',TYSEL_LLM_ENDPOINT=PROVIDER_URL+'/v1/responses',TYSEL_LLM_MODEL='fake',TYSEL_LLM_ALIAS='default',TYSEL_LLM_SECRET='OPENAI_API_KEY')
        else:
            (self.root/'index.js').write_text(SOURCE);source='index.js';database='app.db'
        (self.root/'tysel.toml').write_text(f'''[app]
name="perf-ab"
entry="{source}"
profile="service"
[server]
listen="127.0.0.1:{self.port}"
workers=8
[permissions]
fetch=["127.0.0.1"]
secrets=["OPENAI_API_KEY","GITHUB_TOKEN"]
[limits]
request_timeout_ms=10000
cpu_ms_per_turn=1000
memory_mb=128
max_in_flight=64
[durable]
store="sqlite"
path="./data/{database}"
[observability]
logs="off"
''')
        self.logs=[]
        self.process=subprocess.Popen([str(bin_dir/'tysel'),'-C',str(self.root),'run'],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
        self.log_thread=threading.Thread(target=lambda:self.logs.extend(self.process.stdout),daemon=True)
        try:
            self.log_thread.start()
            deadline=time.monotonic()+STARTUP_TIMEOUT
            while True:
                assert self.process.poll() is None,''.join(self.logs)
                c=http.client.HTTPConnection('127.0.0.1',self.port,timeout=2)
                try:
                    c.request('GET','/health' if app else '/');r=c.getresponse();r.read()
                    if r.status==200:break
                except OSError:pass
                finally:c.close()
                assert time.monotonic()<deadline,'startup timeout';time.sleep(.05)
            self.dbpath=self.root/'data/durable-events.db'
        except BaseException as error:
            try:self.stop(check_exit=False)
            except Exception as cleanup_error:raise error from cleanup_error
            raise
    def stop(self, check_exit=True):
        forced=False
        try:
            if self.process.poll() is None:self.process.terminate()
            try:self.process.wait(timeout=STOP_TIMEOUT)
            except subprocess.TimeoutExpired:
                forced=True;self.process.kill();self.process.wait(timeout=5)
        finally:
            if self.log_thread.ident is not None:self.log_thread.join(timeout=2)
            (self.root/'service.log').write_text(''.join(self.logs))
            if not self.log_thread.is_alive():self.process.stdout.close()
        if check_exit:
            assert not forced,'service required SIGKILL during shutdown'
            assert self.process.returncode==0,''.join(self.logs)
    def operation(self, conn, db):
        started=time.monotonic()
        if self.workload=='app':
            conn.request('POST','/runs',json.dumps({'repository':'owner/repo','tagName':'final-perf','changes':'benchmark change'}),{'Content-Type':'application/json'})
        else:conn.request('GET','/'+self.workload)
        r=conn.getresponse();body=r.read();admission=(time.monotonic()-started)*1000
        assert r.status in (200,202),(r.status,body[:200])
        data=json.loads(body)
        if self.workload=='http':assert data=={'ok':True}
        elif self.workload=='app':
            rid=data['runId'];deadline=time.monotonic()+15
            while True:
                row=db.execute('SELECT status,task_id FROM runs WHERE run_id=?',(rid,)).fetchone()
                if row and row[0]=='completed' and row[1] and db.execute('SELECT 1 FROM runtime.durable_completions WHERE task_id=?',(bytes.fromhex(row[1].zfill(32)),)).fetchone():break
                assert not row or row[0] not in ('failed','cancelled','rejected'),row
                assert time.monotonic()<deadline,'completion timeout';time.sleep(.002)
        else:
            expected={'one':42,'ten':10,'payload':'x'*65536}[self.workload]
            if data.get('status')=='accepted':
                deadline=time.monotonic()+15
                while True:
                    row=db.execute('SELECT result_json FROM durable_completions WHERE task_id=?',(bytes.fromhex(data['taskId'].zfill(32)),)).fetchone()
                    if row:assert json.loads(row[0])==expected;break
                    assert time.monotonic()<deadline,'completion timeout';time.sleep(.002)
            else:assert data.get('status')=='completed' and data.get('value')==expected,data
        return (time.monotonic()-started)*1000,admission
    def connection(self):
        dbfile=self.root/'data/agent-runner.db' if self.workload=='app' else self.dbpath
        db=sqlite3.connect('file:'+str(dbfile)+'?mode=ro',uri=True)
        try:
            if self.workload=='app':db.execute('ATTACH DATABASE ? AS runtime',('file:'+str(self.dbpath)+'?mode=ro',))
            return http.client.HTTPConnection('127.0.0.1',self.port,timeout=15),db
        except BaseException:
            db.close();raise
    def warm(self):
        conn,db=self.connection()
        try:
            for _ in range(20):self.operation(conn,db)
        finally:conn.close();db.close()
    def completed(self):
        with sqlite3.connect('file:'+str(self.dbpath)+'?mode=ro',uri=True) as db:
            return db.execute('SELECT count(*) FROM durable_completions').fetchone()[0]

def percentile(values,p):return sorted(values)[max(0,math.ceil(len(values)*p)-1)]
def phase(version,workload,clients,repeat):
    s=Service(version,workload,f'{workload}-c{clients}-r{repeat}-{version}')
    try:
        s.warm();count0=s.completed();start_usage=usage(s.process.pid);barrier=threading.Barrier(clients+1)
        cancelled=threading.Event();errors=[];error_lock=threading.Lock()
        def client():
            conn=db=None
            try:
                conn,db=s.connection();samples=[];admissions=[]
                barrier.wait(timeout=CLIENT_READY_TIMEOUT);deadline=time.monotonic()+SECONDS
                while not cancelled.is_set() and time.monotonic()<deadline:
                    latency,admission=s.operation(conn,db);samples.append(latency);admissions.append(admission)
                return samples,admissions,time.monotonic()
            except BaseException as error:
                if not isinstance(error, threading.BrokenBarrierError):
                    with error_lock:errors.append(error)
                cancelled.set();barrier.abort()
                raise
            finally:
                if conn is not None:conn.close()
                if db is not None:db.close()
        with futures.ThreadPoolExecutor(clients) as pool:
            jobs=[]
            try:
                jobs=[pool.submit(client) for _ in range(clients)]
                start=time.monotonic();barrier.wait(timeout=CLIENT_READY_TIMEOUT);rss=[];footprint=[]
                while not all(j.done() for j in jobs):
                    u=usage(s.process.pid);rss.append(u['resident_size']);footprint.append(u['phys_footprint']);time.sleep(.1)
                end_usage=usage(s.process.pid)
                if errors:raise errors[0]
                data=[j.result() for j in jobs]
            except BaseException as error:
                cancelled.set();barrier.abort()
                if isinstance(error, threading.BrokenBarrierError) and errors:raise errors[0] from error
                raise
        duration=max(x[2] for x in data)-start;samples=sum([x[0] for x in data],[]);admissions=sum([x[1] for x in data],[]);n=len(samples)
        deadline=time.monotonic()+5
        while workload!='http' and s.completed()-count0!=n:
            assert time.monotonic()<deadline,'completion count mismatch';time.sleep(.02)
        cpu_ns=sum(end_usage[k]-start_usage[k] for k in ['user_time','system_time'])*TICK_NS
        result={'version':version,'workload':workload,'clients':clients,'repeat':repeat,'completed':n,'seconds':duration,'throughput_s':n/duration,'p95_ms':percentile(samples,.95),'p99_ms':percentile(samples,.99),'admission_p95_ms':percentile(admissions,.95),'admission_p99_ms':percentile(admissions,.99),'cpu_core_percent':cpu_ns/1e9/duration*100,'cpu_ms_per_op':cpu_ns/1e6/n,'rss_peak_mib':max(rss)/2**20,'footprint_peak_mib':max(footprint)/2**20,'database_storage_write_bytes_per_op':(end_usage['diskio_byteswritten']-start_usage['diskio_byteswritten'])/n,'fixture':str(s.root)}
        (s.root/'latencies.json').write_text(json.dumps({'completion_ms':samples,'admission_ms':admissions}))
        return result
    finally:s.stop(check_exit=sys.exc_info()[0] is None)

def write_probe(version,workload):
    s=Service(version,workload,f'write-{workload}-{version}')
    try:
        s.warm();paths=[s.dbpath]+([s.root/'data/agent-runner.db'] if workload=='app' else [])
        configs={}
        for path in paths:
            with sqlite3.connect('file:'+str(path)+'?mode=ro',uri=True) as db:
                configs[path.name]={'journal_mode':db.execute('PRAGMA journal_mode').fetchone()[0],'page_size':db.execute('PRAGMA page_size').fetchone()[0]}
        count0=s.completed();sizes={p.name:p.stat().st_size for p in paths};before=usage(s.process.pid)
        conn,db=s.connection()
        for _ in range(50):s.operation(conn,db)
        conn.close();db.close();assert s.completed()-count0==50
        time.sleep(.1);after=usage(s.process.pid)
        return {'version':version,'workload':workload,'operations':50,'configuration':configs,'database_storage_write_bytes_per_op':(after['diskio_byteswritten']-before['diskio_byteswritten'])/50,'retained_database_growth_bytes_per_op':{p.name:(p.stat().st_size-sizes[p.name])/50 for p in paths}}
    finally:s.stop(check_exit=sys.exc_info()[0] is None)

if __name__=='__main__':
    provider=http.server.ThreadingHTTPServer(('127.0.0.1',0),Provider);PROVIDER_URL=f'http://127.0.0.1:{provider.server_port}'
    threading.Thread(target=provider.serve_forever,daemon=True).start()
    results=[];wal=[]
    workloads=os.environ.get('TYSEL_AB_WORKLOADS','http,one,ten,payload,app').split(',')
    try:
        for workload in workloads:
            for clients in ([8] if workload=='http' else [1,8]):
                for repeat in range(REPEATS):
                    for version in (['before','after'] if repeat%2==0 else ['after','before']):
                        result=phase(version,workload,clients,repeat);results.append(result);print(json.dumps(result),flush=True)
                        (OUT/'results.json').write_text(json.dumps(results,indent=2)+'\n')
            if workload!='http':
                for version in ['before','after']:
                    result=write_probe(version,workload);wal.append(result);print('WRITE '+json.dumps(result),flush=True)
                    (OUT/'writes.json').write_text(json.dumps(wal,indent=2)+'\n')
    finally:provider.shutdown();provider.server_close()
