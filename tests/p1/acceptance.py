"""Opt-in P1 fault acceptance using a disposable local fixture.

Build the debug CLI and service first. Run directly for packaged mode, or set
TYSEL_P1_USE_RUN=1 for the CLI server. The JSON result identifies the retained
temporary fixture and logs. No existing application or database is modified.
"""
import concurrent.futures
import http.client
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
ROOT = Path(tempfile.mkdtemp(prefix='tysel-assessment-'))
BIN = Path(os.environ.get('TYSEL_GATE_BIN_DIR', str(REPO / 'target/debug'))).resolve()
CLI = BIN / 'tysel'
STUB = BIN / 'tysel-service'
results = {'source_commit': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(), 'fixture': str(ROOT)}
source = '''export default {
  durable: {
    async done(ctx, input) { return {done: true}; },
    async delayed(ctx, input) {
      await ctx.sleep(50);
      return await ctx.step("resumed", () => 1);
    }
  },
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === "/error") throw new Error("ASSESSMENT_SENTINEL_PRIVATE_DETAILS");
    if (path === "/slow") await new Promise(resolve => setTimeout(resolve, 350));
    if (path === "/start") return Response.json(tysel.durable.start("done", {}));
    if (path === "/schedule") return Response.json(tysel.durable.start("delayed", {}));
    return new Response("ok");
  }
};'''
(ROOT / 'index.js').write_text(source)
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    port = sock.getsockname()[1]
(ROOT / 'tysel.toml').write_text(f'''schema_version = 1
[app]
name = "assessment"
entry = "index.js"
profile = "service"
[server]
listen = "127.0.0.1:{port}"
workers = 1
[limits]
cpu_ms_per_turn = 500
request_timeout_ms = 500
max_in_flight = 10
[durable]
store = "sqlite"
path = "./data/tysel.db"
[observability]
logs = "json"
''')
env = os.environ.copy()
for key in list(env):
    if key.startswith('TYSEL_DURABLE_') or key.startswith('OTEL_'):
        del env[key]
env['OTEL_SDK_DISABLED'] = 'true'
if os.environ.get('TYSEL_P1_USE_RUN'):
    command = [str(CLI), '-C', str(ROOT), 'run']
    results['build'] = {'status': 'skipped', 'reason': 'CLI run executes the source directly'}
else:
    build = subprocess.run([str(CLI), '-C', str(ROOT), 'build', '--stub', str(STUB), '--output', str(ROOT / 'app')], capture_output=True, text=True, env=env)
    results['build'] = {'returncode': build.returncode, 'stdout': build.stdout, 'stderr': build.stderr}
    if build.returncode:
        print(json.dumps(results, indent=2))
        raise SystemExit(1)
    command = [str(ROOT / 'app')]

def request(path):
    started = time.monotonic()
    conn = http.client.HTTPConnection('127.0.0.1', port, timeout=3)
    try:
        conn.request('GET', path)
        response = conn.getresponse()
        return {'status': response.status, 'body': response.read().decode(), 'elapsed_ms': round((time.monotonic()-started)*1000)}
    except Exception as error:
        return {'error': type(error).__name__ + ': ' + str(error), 'elapsed_ms': round((time.monotonic()-started)*1000)}
    finally:
        conn.close()

log = open(ROOT / 'service.log', 'w')
process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=log, env=env)
try:
    for _ in range(100):
        if request('/') .get('status') == 200:
            break
        if process.poll() is not None:
            raise RuntimeError('service exited at startup')
        time.sleep(.05)
    results['error_response'] = request('/error')
    with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
        results['queued_requests'] = list(executor.map(request, ['/slow'] * 3))
    results['completed_starts'] = [request('/start') for _ in range(3)]
    db = sqlite3.connect(ROOT / 'data/durable-events.db', timeout=3)
    results['retained_after_completion'] = {
        'programs': db.execute('SELECT COUNT(*) FROM durable_programs').fetchone()[0],
        'active_programs': db.execute('SELECT active_count FROM durable_program_stats').fetchone()[0],
        'completions': db.execute('SELECT COUNT(*) FROM durable_completions').fetchone()[0],
        'events': db.execute('SELECT COUNT(*) FROM durable_events').fetchone()[0],
        'wakeups': db.execute('SELECT COUNT(*) FROM durable_wakeups').fetchone()[0]
    }
    # Fault injection affects only this disposable fixture's SQLite database.
    db.execute('ALTER TABLE durable_programs RENAME TO assessment_programs_temporarily_unavailable')
    db.commit()
    time.sleep(.6)
    db.execute('ALTER TABLE assessment_programs_temporarily_unavailable RENAME TO durable_programs')
    db.commit()
    for attempt in range(60):
        resumed = request('/schedule')
        if resumed.get('status') == 200:
            break
        time.sleep(.1)
    results['start_after_store_recovery'] = resumed
    time.sleep(.9)
    results['after_store_recovery'] = {
        'http': request('/'),
        'pending_wakeups': db.execute('SELECT COUNT(*) FROM durable_wakeups').fetchone()[0],
        'resumed_steps': db.execute("SELECT COUNT(*) FROM durable_events WHERE event_key = 'resumed'").fetchone()[0],
        'process_alive': process.poll() is None
    }
    db.close()
finally:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    log.close()
results['service_log'] = (ROOT / 'service.log').read_text()

# Start a fresh instance so the shutdown observation is independent of the fault injection.
log = open(ROOT / 'shutdown.log', 'w')
process = subprocess.Popen(command, cwd=ROOT, stdout=log, stderr=log, env=env)
try:
    for _ in range(100):
        if request('/').get('status') == 200:
            break
        time.sleep(.05)
    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
        pending = executor.submit(request, '/slow')
        time.sleep(.1)
        process.send_signal(signal.SIGTERM)
        results['inflight_request_at_sigterm'] = pending.result()
finally:
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    log.close()
assert results['error_response']['status'] == 500
assert 'ASSESSMENT_SENTINEL' not in results['error_response']['body']
assert sorted(r['status'] for r in results['queued_requests']) == [200,504,504]
assert max(r['elapsed_ms'] for r in results['queued_requests']) < 750
assert results['retained_after_completion']['active_programs'] == 0
assert results['retained_after_completion']['completions'] == 3
assert results['after_store_recovery']['resumed_steps'] == 1
assert results['inflight_request_at_sigterm']['status'] == 200
(ROOT / 'results.json').write_text(json.dumps(results, indent=2))
print(json.dumps(results, indent=2))
