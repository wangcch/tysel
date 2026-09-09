import tempfile,pathlib,socket,subprocess,http.client,time,json,os
root=pathlib.Path(tempfile.mkdtemp(prefix='tysel-reassess-reload-'))
with socket.socket() as s:s.bind(('127.0.0.1',0));port=s.getsockname()[1]
(root/'tysel.toml').write_text(f'''[app]\nname="reload-probe"\nentry="index.js"\nprofile="service"\n[server]\nlisten="127.0.0.1:{port}"\n[durable]\nstore="sqlite"\npath="./data/durable.db"\n''')
source='''export default { durable: {async job(ctx,input){ return await ctx.waitForSignal("go"); }}, fetch(request){ if(new URL(request.url).pathname==='/signal'){tysel.durable.sendSignal(new URL(request.url).searchParams.get('id'),'go',{ok:true});return new Response('ok');}if(new URL(request.url).pathname==='/start') {try{return Response.json(tysel.durable.start('job',{ok:true}));}catch(e){return new Response(String(e),{status:500});}}return new Response('VERSION');}};'''
(root/'index.js').write_text(source.replace('VERSION','one'))
env={k:v for k,v in os.environ.items() if not k.startswith('TYSEL_DURABLE_')}
log=(root/'service.log').open('w');p=subprocess.Popen([os.environ.get('TYSEL_BIN',str(pathlib.Path(__file__).resolve().parents[2]/'target/debug/tysel')),'-C',str(root),'dev'],env=env,stdout=log,stderr=log)
def get(path):
 c=http.client.HTTPConnection('127.0.0.1',port,timeout=2);c.request('GET',path);r=c.getresponse();v=(r.status,r.read().decode());c.close();return v
def wait(body):
 for _ in range(200):
  try:
   if get('/')[1]==body:return
  except OSError:pass
  time.sleep(.05)
 raise RuntimeError('timeout '+str(root))
try:
 wait('one');time.sleep(1);before=get('/start');assert before[0]==200,before
 first=json.loads(before[1])['taskId']
 (root/'index.js').write_text(source.replace('VERSION','two'));time.sleep(2);wait('two')
 after=get('/start');assert after[0]==200,after
 second=json.loads(after[1])['taskId']
 assert get('/signal?id='+first)[0]==200
 assert get('/signal?id='+second)[0]==200
 import sqlite3
 db=sqlite3.connect('file:'+str(root/'data/durable-events.db')+'?mode=ro',uri=True)
 for _ in range(100):
  completed=db.execute('SELECT result_json FROM durable_completions').fetchall()
  if len(completed)==2:break
  time.sleep(.05)
 assert len(completed)==2,completed
 assert all(json.loads(row[0])=={'ok':True} for row in completed)
 db.close()
 print(json.dumps({'fixture':str(root),'before':before,'after':after,'completed':2}))

finally:
 p.terminate();p.wait(timeout=10);log.close();assert p.returncode==0,p.returncode;assert "panicked at" not in (root/"service.log").read_text()
