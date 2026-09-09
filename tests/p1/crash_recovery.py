"""G1/G2 SIGKILL gates. Build CLI and service binaries first.

All traffic is loopback. Fake provider survives the child runtime crash and
reconciles a committed operation using its stable identity. Database inspection
is read-only; both CLI and standalone run against the same files after restart.
"""
import concurrent.futures
import http.client
import http.server
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import tempfile
import threading
import time

REPO = Path(__file__).resolve().parents[2]
BIN = Path(os.environ.get('TYSEL_GATE_BIN_DIR', str(REPO / 'target/debug'))).resolve()
PG_URL = os.environ.get('TYSEL_GATE_POSTGRES_URL')
PG_CONTAINER = os.environ.get('TYSEL_GATE_POSTGRES_CONTAINER')
INSTANCES = int(os.environ.get('TYSEL_GATE_INSTANCES', '1'))
assert INSTANCES in (1, 2)

def pg_query(sql):
    assert PG_CONTAINER, 'dedicated PostgreSQL fixture container required'
    result = subprocess.run(['podman', 'exec', PG_CONTAINER, 'psql', '-U', 'postgres', '-d', 'tysel_validation', '-qAt', '-v', 'ON_ERROR_STOP=1', '-c', sql], check=True, capture_output=True, text=True)
    return result.stdout.strip()

ROOT = Path(tempfile.mkdtemp(prefix='tysel-g12-crash-'))
print('FIXTURE='+str(ROOT), flush=True)

def wait_for(fn, seconds=15):
    until=time.monotonic()+seconds
    while time.monotonic()<until:
        value=fn()
        if value:return value
        time.sleep(.02)
    raise AssertionError('condition timed out')

class Provider(http.server.ThreadingHTTPServer):
    daemon_threads=True
    def __init__(self,mode):
        self.mode=mode; self.calls={}; self.creates=0; self.result=None
        self.blocked=threading.Event();self.release=threading.Event();self.lock=threading.Lock()
        super().__init__(('127.0.0.1',0),Handler)
class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self,*args):pass
    def do_GET(self):self.respond()
    def do_POST(self):self.respond()
    def respond(self):
        server=self.server
        with server.lock:
            server.calls[self.path]=server.calls.get(self.path,0)+1
            first=server.calls[self.path]==1
            if self.path=='/create':
                server.creates+=1
                server.result={'operationId':'stable-operation'}
            value={'found':server.result is not None,'result':server.result} if self.path=='/lookup' else server.result or {}
        gate={'before_effect':'/before','initial_admission':'/before','after_commit':'/create','after_return':'/after-return','after_record':'/after','projection_wait':'/projection'}[server.mode]
        if self.path==gate and first:
            server.blocked.set();server.release.wait(20)
        data=json.dumps(value).encode()
        try:
            self.send_response(200);self.send_header('content-length',str(len(data)));self.end_headers();self.wfile.write(data)
        except (BrokenPipeError,ConnectionResetError):pass

def run(mode,standalone):
    root=ROOT/(('standalone-' if standalone else 'run-')+mode+'-'+str(time.time_ns()));root.mkdir()
    provider=Provider(mode);thread=threading.Thread(target=provider.serve_forever,daemon=True);thread.start()
    origin='http://127.0.0.1:'+str(provider.server_port)
    with socket.socket() as sock:sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    source='''const origin=ORIGIN;
export default {
 durable: {async job(ctx,input) {
   if(input.mode==='projection_wait') await fetch(origin+'/projection');
   if(input.mode!=='initial_admission') await ctx.waitForSignal('go');
   const result=await ctx.effect('create',async()=>{
     const prior=await (await fetch(origin+'/lookup')).json();
     if(prior.found) return prior.result;
     if(input.mode==='before_effect'||input.mode==='initial_admission') await fetch(origin+'/before');
     const created=await (await fetch(origin+'/create',{method:'POST'})).json();
     if(input.mode==='after_return') await fetch(origin+'/after-return');
     return created;
   });
   if(input.mode==='after_record') await fetch(origin+'/after');
   return result;
 }},
 async fetch(request) {
   const path=new URL(request.url).pathname;
   if(path==='/start') return Response.json(tysel.durable.start('job',{mode:MODE},{idempotencyKey:'bound-run'}));
   if(path==='/signal'||path==='/signal-lost') {
     const data=await request.json();
     tysel.durable.sendSignal(data.taskId,'go',{approved:true},{idempotencyKey:'decision-1'});
     if(path==='/signal-lost') await new Promise(()=>{});
     return new Response('ok');
   }
   return new Response('healthy');
 }
};'''.replace('ORIGIN',json.dumps(origin)).replace('MODE',json.dumps(mode))
    (root/'index.js').write_text(source)
    (root/'tysel.toml').write_text(f'''[app]
name="crash-gate"
entry="index.js"
profile="service"
[server]
listen="127.0.0.1:{port}"
workers=2
[permissions]
fetch=["127.0.0.1"]
[limits]
request_timeout_ms=1000
cpu_ms_per_turn=500
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
''')
    env={k:v for k,v in os.environ.items() if not k.startswith(('TYSEL_DURABLE_','OTEL_'))}
    env['OTEL_SDK_DISABLED']='true'
    schema = 'gate_' + str(time.time_ns())
    if PG_URL:
        pg_query('CREATE SCHEMA ' + schema)
        env['TYSEL_DURABLE_POSTGRES_URL'] = PG_URL + ('&' if '?' in PG_URL else '?') + 'options=-csearch_path%3D' + schema

    if standalone:
        subprocess.run([str(BIN/'tysel'),'-C',str(root),'build','--stub',str(BIN/'tysel-service'),'--output',str(root/'app')],env=env,check=True,capture_output=True)
        env['PATH']='';command=[str(root/'app')]
    else:command=[str(BIN/'tysel'),'-C',str(root),'run']
    def request(path,data=None):
        conn=http.client.HTTPConnection('127.0.0.1',port,timeout=3)
        try:
            conn.request('POST' if data is not None else 'GET',path,None if data is None else json.dumps(data),{'content-type':'application/json'})
            response=conn.getresponse();body=response.read().decode()
            assert response.status==200,(response.status,body)
            return body
        finally:conn.close()
    def healthy():
        try:return request('/')=='healthy'
        except (OSError,AssertionError):return False
    log=(root/'service.log').open('w');process=None;peer=None;peer_log=None
    executor=concurrent.futures.ThreadPoolExecutor(1)
    try:
        process=subprocess.Popen(command,cwd=root,env=env,stdout=log,stderr=log);wait_for(healthy)
        # Prepare the peer artifact before holding a one-second effect open.
        if INSTANCES == 2:
            assert PG_URL, 'multi-instance gate requires shared PostgreSQL'
            peer_root=root/'peer';peer_root.mkdir()
            with socket.socket() as sock:sock.bind(('127.0.0.1',0));peer_port=sock.getsockname()[1]
            (peer_root/'index.js').write_text(source)
            (peer_root/'tysel.toml').write_text((root/'tysel.toml').read_text().replace(str(port),str(peer_port)))
            peer_command=[str(BIN/'tysel'),'-C',str(peer_root),'run']
            if standalone:
                subprocess.run([str(BIN/'tysel'),'-C',str(peer_root),'build','--stub',str(BIN/'tysel-service'),'--output',str(peer_root/'app')],env={**env,'PATH':os.environ.get('PATH','')},check=True,capture_output=True)
                peer_command=[str(peer_root/'app')]
        # The initial request is deliberately left unacknowledged in two cases.
        pending=executor.submit(request,'/start')
        db_path=root/'data/durable-events.db'
        def rows(sql):
            if PG_URL:
                sql = sql.replace('lower(hex(task_id))', "encode(task_id,'hex')")
                data = json.loads(pg_query("SET search_path TO "+schema+"; SET default_transaction_read_only=on; SELECT COALESCE(json_agg(row_to_json(t)),'[]'::json) FROM ("+sql+") t"))
                return [list(row.values()) for row in data]
            if not db_path.exists():return []
            with sqlite3.connect('file:'+str(db_path)+'?mode=ro',uri=True) as db:return db.execute(sql).fetchall()
        task=wait_for(lambda:rows('SELECT lower(hex(task_id)) FROM durable_programs'))[0][0]
        if mode not in ('initial_admission','projection_wait'):
            pending.result()
            if mode=='after_commit':pending=executor.submit(request,'/signal-lost',{'taskId':task})
            else:request('/signal',{'taskId':task})
        assert provider.blocked.wait(8),(mode,provider.calls)
        if mode=='after_record':assert rows("SELECT count(*) FROM durable_events WHERE kind='effect'")[0][0]==1
        if INSTANCES == 2:
            peer_log=(peer_root/'service.log').open('w')
            peer=subprocess.Popen(peer_command,cwd=peer_root,env=env,stdout=peer_log,stderr=peer_log)
            def peer_ready():
                try:
                    conn=http.client.HTTPConnection('127.0.0.1',peer_port,timeout=1)
                    conn.request('GET','/');response=conn.getresponse();body=response.read();conn.close()
                    return response.status==200 and body==b'healthy'
                except OSError:return False
            wait_for(peer_ready)
        process.send_signal(signal.SIGKILL);process.wait(timeout=5)
        provider.release.set()
        try:pending.result(timeout=3)
        except (OSError,http.client.HTTPException,AssertionError):pass
        process=subprocess.Popen(command,cwd=root,env=env,stdout=log,stderr=log);wait_for(healthy)
        # Repeated admission binds to the original task and cannot run a replacement.
        repeated=json.loads(request('/start'));assert repeated['taskId']==task,repeated
        if mode in ('projection_wait','after_commit'):request('/signal',{'taskId':task})
        started=time.monotonic()
        completed=wait_for(lambda:rows('SELECT result_json FROM durable_completions'),15)
        assert json.loads(completed[0][0])=={'operationId':'stable-operation'},completed
        if mode!='initial_admission':
            request('/signal',{'taskId':task}) # duplicate after consumption and completion
            assert rows("SELECT count(*) FROM durable_events WHERE kind='signal'")[0][0]==1
            assert rows('SELECT count(*) FROM durable_signal_inbox')[0][0]==0
        assert rows('SELECT count(*) FROM durable_programs')[0][0]==1
        assert rows('SELECT active_count FROM durable_program_stats')[0][0]==0
        assert provider.creates==1,provider.calls
        result={'backend':'postgres' if PG_URL else 'sqlite','instances':INSTANCES,'mode':mode,'standalone':standalone,'task_id':task,'completed':True,'create_requests':provider.creates,'provider_calls':dict(provider.calls),'recovery_seconds':round(time.monotonic()-started,3)}
        print(json.dumps(result),flush=True);return result
    finally:
        provider.release.set()
        if process is not None and process.poll() is None:
            process.terminate()
            try:process.wait(timeout=5)
            except subprocess.TimeoutExpired:process.kill();process.wait()
        if peer is not None and peer.poll() is None:
            peer.terminate()
            try:peer.wait(timeout=5)
            except subprocess.TimeoutExpired:peer.kill();peer.wait()
        if peer_log:peer_log.close()
        executor.shutdown(wait=True);log.close();provider.shutdown();provider.server_close();thread.join()
        if 'result' in locals():
            for service in (process,peer):
                if service is not None:
                    assert service.returncode==0, f'Unclean shutdown: {service.returncode}'
        for service_log in [root/'service.log', root/'peer'/'service.log']:
            if service_log.exists():
                content=service_log.read_text()
                assert 'panicked at' not in content, f'Runtime panic: {service_log}'

if __name__=='__main__':
    results=[]
    for standalone in ((True,) if os.environ.get('TYSEL_GATE_STANDALONE_ONLY') else (False,True)):
        for mode in ('initial_admission','before_effect','after_commit','after_return','after_record','projection_wait'):
            results.append(run(mode,standalone))
            (ROOT/'results.json').write_text(json.dumps(results,indent=2))
    print('RESULT_PATH='+str(ROOT/'results.json'),flush=True)
