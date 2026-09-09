"""Loopback regression for an effect recording failure; uses disposable SQLite only.
Run after building tysel: python3 tests/p1/storage_recovery.py
"""
import json
import sqlite3
import time
from review_regressions import Service

SETTINGS = '[server]\nlisten="127.0.0.1:{port}"\n[limits]\nrequest_timeout_ms=1000\n[durable]\nstore="sqlite"\npath="./data/app.db"\n[observability]\nlogs="json"\n'
SOURCE = """export default {
  durable: {async job(ctx) {
    try {return await ctx.effect('write', async () => 42)}
    catch {return 99}
  }},
  fetch(req) {
    return new URL(req.url).pathname === '/' ? new Response('ok') :
      Response.json(tysel.durable.start('job', null, {idempotencyKey: 'review'}));
  }
};"""

with Service('effect-storage-recovery', SOURCE, SETTINGS) as service:
    with sqlite3.connect(service.root / 'data/durable-events.db') as db:
        db.execute("CREATE TRIGGER fail_effect BEFORE INSERT ON durable_events "
                   "WHEN NEW.sequence=1 BEGIN SELECT RAISE(FAIL, 'temporary storage fault'); END")
        db.commit()
        assert service.request('/start')[0] == 500
        assert db.execute('SELECT state FROM durable_executions').fetchone()[0] == 'running'
        assert db.execute('SELECT count(*) FROM durable_completions').fetchone()[0] == 0
        db.execute('DROP TRIGGER fail_effect')
        db.commit()
        start = time.monotonic()
        while db.execute('SELECT count(*) FROM durable_completions').fetchone()[0] == 0:
            assert time.monotonic() - start < 10, 'task did not recover'
            time.sleep(.02)
        elapsed = time.monotonic() - start
        assert json.loads(db.execute('SELECT result_json FROM durable_completions').fetchone()[0]) == 42
        status, body, _ = service.request('/start')
        assert status == 200
        assert json.loads(body)['status'] == 'completed'
        assert json.loads(body)['value'] == 42
        result = {'fixture': str(service.root), 'recovered': True,
                  'caught_fallback_not_committed': True, 'recovery_seconds': elapsed}
        (service.root / 'results.json').write_text(json.dumps(result, indent=2))
        print(json.dumps(result))
