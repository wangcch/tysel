"""Completion storage outage longer than a lease must not stop the service.

Run after building the runtime binaries. Uses disposable SQLite and loopback only.
"""
import http.server
import json
import sqlite3
import threading
import time
from review_regressions import Service

class Provider(http.server.BaseHTTPRequestHandler):
    calls = 0

    def do_GET(self):
        Provider.calls += 1
        body = b'42'
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass

provider = http.server.HTTPServer(('127.0.0.1', 0), Provider)
thread = threading.Thread(target=provider.serve_forever, daemon=True)
thread.start()
settings = '''[server]
listen="127.0.0.1:{port}"
[limits]
request_timeout_ms=1000
[permissions]
fetch=["127.0.0.1"]
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
'''
source = '''export default {
 durable: {async job(ctx) {
   return await ctx.effect('external', async () => {
     const response = await fetch('ORIGIN'); return await response.json();
   });
 }},
 fetch(req) {
   return new URL(req.url).pathname === '/' ? new Response('ok') :
     Response.json(tysel.durable.start('job', null, {idempotencyKey:'completion'}));
 }
};'''.replace('ORIGIN', 'http://127.0.0.1:' + str(provider.server_port))
try:
    with Service('completion-expiry-recovery', source, settings) as service:
        with sqlite3.connect(service.root / 'data/durable-events.db') as db:
            db.execute('CREATE TRIGGER fail_completion BEFORE INSERT ON durable_completions '
                       'BEGIN SELECT * FROM missing_completion_table; END')
            db.commit()
            assert service.request('/start')[0] == 500
            assert Provider.calls == 1
            # Exceed the 1 second request timeout plus 5 second lease margin.
            time.sleep(8)
            assert service.process.poll() is None, 'lease expiry stopped the service'
            assert service.request('/')[0] == 200
            assert db.execute('SELECT count(*) FROM durable_completions').fetchone()[0] == 0
            db.execute('DROP TRIGGER fail_completion')
            db.commit()
            start = time.monotonic()
            while db.execute('SELECT count(*) FROM durable_completions').fetchone()[0] == 0:
                assert service.process.poll() is None
                assert time.monotonic() - start < 10, 'completion did not recover'
                time.sleep(.02)
            assert Provider.calls == 1, 'recorded effect ran again'
            assert db.execute('SELECT count(*) FROM durable_events').fetchone()[0] == 2
            assert db.execute('SELECT generation FROM durable_executions').fetchone()[0] >= 2
            status, body, _ = service.request('/start')
            assert status == 200
            assert json.loads(body)['status'] == 'completed'
            assert json.loads(body)['value'] == 42
            result = {'fixture': str(service.root), 'outage_seconds': 8,
                      'service_survived': True, 'recovered': True,
                      'provider_calls': Provider.calls,
                      'recovery_after_fault_removed_seconds': time.monotonic() - start}
            (service.root / 'results.json').write_text(json.dumps(result, indent=2))
            print(json.dumps(result))
finally:
    provider.shutdown()
    provider.server_close()
    thread.join(timeout=2)
