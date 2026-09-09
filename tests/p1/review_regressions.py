"""Opt-in loopback regression gates for the P1 review follow-up.

Run from any directory after building tysel and tysel-worker. Uses disposable
SQLite files and sends signals only to child workers of its own test service.
Temporary fixtures and timing samples are retained for inspection.
"""
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
BIN_DIR = Path(os.environ.get("TYSEL_GATE_BIN_DIR", str(REPO / "target/debug"))).resolve()
ROOT = Path(tempfile.mkdtemp(prefix="tysel-review-gates-"))


class Service:
    def __init__(self, name, source, settings):
        self.root = ROOT / name
        self.root.mkdir()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            self.port = sock.getsockname()[1]
        (self.root / "index.js").write_text(source)
        (self.root / "tysel.toml").write_text(
            f'[app]\nname="{name}"\nentry="index.js"\n' + settings.format(port=self.port)
        )
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(("TYSEL_DURABLE_", "OTEL_"))}
        env.update(OTEL_SDK_DISABLED="true", TYSEL_WORKER=str(BIN_DIR / "tysel-worker"))
        self.log = (self.root / "service.log").open("w")
        self.process = subprocess.Popen(
            [str(BIN_DIR / "tysel"), "-C", str(self.root), "run"],
            stdout=self.log, stderr=self.log, env=env,
        )

    def __enter__(self):
        try:
            deadline = time.monotonic() + 60
            while time.monotonic() < deadline:
                assert self.process.poll() is None, (self.root / "service.log").read_text()
                try:
                    if self.request("/")[0] == 200:
                        return self
                except OSError:
                    pass
                time.sleep(.05)
            raise AssertionError("service did not become ready")
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def request(self, path):
        connection = http.client.HTTPConnection("127.0.0.1", self.port, timeout=15)
        start = time.monotonic()
        try:
            connection.request("GET", path)
            response = connection.getresponse()
            body = response.read().decode()
            return response.status, body, (time.monotonic() - start) * 1000
        finally:
            connection.close()

    def workers(self):
        children = subprocess.run(["pgrep", "-P", str(self.process.pid)],
                                  capture_output=True, text=True).stdout.split()
        return [int(pid) for pid in children if "tysel-worker" in subprocess.run(
            ["ps", "-p", pid, "-o", "comm="], capture_output=True, text=True).stdout]

    def __exit__(self, *_):
        if self.process.poll() is None:
            for pid in self.workers():
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        self.log.close()


def durable_gate():
    settings = '''[server]
listen="127.0.0.1:{port}"
[limits]
memory_mb=256
cpu_ms_per_turn=4000
request_timeout_ms=10000
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
'''
    source = '''export default {
      durable: {
        async large(){return Array(300000).fill(0)},
        async delayed(ctx){await ctx.sleep(100);return 42}
      },
      fetch(req){const name=new URL(req.url).pathname.slice(1);
        return name ? Response.json(tysel.durable.start(name,null)) : new Response('ready');}
    };'''
    with Service("durable", source, settings) as service:
        status, body, elapsed = service.request("/large")
        assert status == 200, body
        assert len(json.loads(body)["value"]) == 300000
        with sqlite3.connect(service.root / "data/durable-events.db") as db:
            db.execute("CREATE TRIGGER fail_completion BEFORE INSERT ON durable_completions "
                       "BEGIN SELECT * FROM missing_test_table; END")
            db.commit()
            status, body, _ = service.request("/delayed")
            assert status == 200, body
            task = bytes.fromhex(json.loads(body)["taskId"])
            deadline = time.monotonic() + 3
            while '"state":"recovering"' not in (service.root / "service.log").read_text():
                assert time.monotonic() < deadline, "scheduler did not report storage failure"
                time.sleep(.01)
            assert db.execute("SELECT count(*) FROM durable_completions WHERE task_id=?", (task,)).fetchone()[0] == 0
            assert db.execute("SELECT count(*) FROM durable_wakeups WHERE task_id=?", (task,)).fetchone()[0] == 0
            # Any attempt to reload this program would fail its source hash check.
            db.execute("UPDATE durable_programs SET source='throw 42' WHERE task_id=?", (task,))
            db.execute("DROP TRIGGER fail_completion")
            db.commit()
            deadline = time.monotonic() + 4
            while db.execute("SELECT count(*) FROM durable_completions WHERE task_id=?", (task,)).fetchone()[0] != 1:
                assert time.monotonic() < deadline, "completion never recovered"
                time.sleep(.02)
        return {"large_result_status": status, "large_result_ms": elapsed, "completion_recovered": True}


def isolated_gate():
    settings = '''profile="isolated"
[server]
listen="127.0.0.1:{port}"
[limits]
memory_mb=16
cpu_ms_per_turn=200
request_timeout_ms=300
'''
    source = "let n=0;export default {fetch(req){if(new URL(req.url).pathname==='/mutate')n++;return new Response(String(n));}}"
    with Service("isolated", source, settings) as service:
        pid, = service.workers()
        samples = []
        for _ in range(200):
            status, _, elapsed = service.request("/count")
            assert status == 200
            samples.append(elapsed)
        assert service.workers() == [pid], "normal requests should reuse their worker"
        os.kill(pid, signal.SIGSTOP)
        status, _, elapsed = service.request("/mutate")
        assert status == 504 and elapsed < 1000, (status, elapsed)
        time.sleep(.1)
        assert pid not in service.workers(), "timed-out frozen worker is still alive"
        assert service.request("/count")[:2] == (200, "0")
        replacement, = service.workers()
        os.kill(replacement, signal.SIGSTOP)
        assert service.request("/mutate")[0] == 504
        start = time.monotonic()
        service.process.terminate()
        service.process.wait(timeout=2)
        samples.sort()
        return {"timeout_ms": elapsed, "shutdown_ms": (time.monotonic() - start) * 1000,
                "debug_loopback_p50_ms": samples[len(samples)//2],
                "debug_loopback_p95_ms": samples[int(len(samples)*.95)],
                "latency_samples_ms": samples, "worker_reaped": True}



def finalization_read_gate():
    settings = """[server]
listen="127.0.0.1:{port}"
[limits]
memory_mb=32
cpu_ms_per_turn=500
request_timeout_ms=4000
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
"""
    source = """export default {
      durable:{async work(ctx){await ctx.sleep(100);await new Promise(r=>setTimeout(r,1200));return 42;}},
      fetch(req){return new URL(req.url).pathname==='/'?new Response('ok'):Response.json(tysel.durable.start('work',null));}
    };"""
    with Service("finalization-read", source, settings) as service:
        status, body, _ = service.request("/start")
        assert status == 200, body
        task = bytes.fromhex(json.loads(body)["taskId"])
        with sqlite3.connect(service.root / "data/durable-events.db") as db:
            deadline = time.monotonic() + 3
            while db.execute("SELECT count(*) FROM durable_wakeups WHERE task_id=?", (task,)).fetchone()[0]:
                assert time.monotonic() < deadline
                time.sleep(.005)
            db.execute("ALTER TABLE durable_signal_waits RENAME TO unavailable_waits")
            db.commit()
            deadline = time.monotonic() + 3
            while '"state":"recovering"' not in (service.root / "service.log").read_text():
                assert time.monotonic() < deadline, "finalization read fault was not supervised"
                time.sleep(.01)
            db.execute("ALTER TABLE unavailable_waits RENAME TO durable_signal_waits")
            db.execute("UPDATE durable_programs SET source='throw 42' WHERE task_id=?", (task,))
            db.commit()
            deadline = time.monotonic() + 3
            while db.execute("SELECT count(*) FROM durable_completions WHERE task_id=?", (task,)).fetchone()[0] != 1:
                assert time.monotonic() < deadline, "result was lost after finalization read failure"
                time.sleep(.01)
        assert '"state":"completion_pending"' in (service.root / "service.log").read_text()
        return {"recovered_without_replay": True}


def partial_batch_gate():
    settings = """[server]
listen="127.0.0.1:{port}"
[limits]
memory_mb=32
cpu_ms_per_turn=500
request_timeout_ms=4000
[durable]
store="sqlite"
path="./data/app.db"
[observability]
logs="json"
"""
    source = """export default {
      durable:{async work(ctx,input){await ctx.sleep(500);return input;}},
      fetch(req){let p=new URL(req.url).pathname;return p==='/'?new Response('ok'):Response.json(tysel.durable.start('work',p==='/a'?1:2));}
    };"""
    with Service("partial-batch", source, settings) as service:
        with sqlite3.connect(service.root / "data/durable-events.db") as db:
            db.execute("CREATE TRIGGER fail_second BEFORE INSERT ON durable_completions "
                       "WHEN NEW.result_json='2' BEGIN SELECT json('malformed'); END")
            db.commit()
            task_ids = [json.loads(service.request(path)[1])["taskId"] for path in ("/a", "/b")]
            deadline = time.monotonic() + 3
            while '"state":"recovering"' not in (service.root / "service.log").read_text():
                assert time.monotonic() < deadline
                time.sleep(.01)
            db.execute("DROP TRIGGER fail_second")
            db.commit()
            deadline = time.monotonic() + 3
            while db.execute("SELECT count(*) FROM durable_completions").fetchone()[0] != 2:
                assert time.monotonic() < deadline
                time.sleep(.01)
        # Allow the completion callback following the transaction to finish.
        deadline = time.monotonic() + 1
        while True:
            events = [json.loads(line) for line in (service.root / "service.log").read_text().splitlines() if line.startswith("{")]
            completed = [event["taskId"] for event in events if event.get("state") == "completed"]
            if sorted(completed) == sorted(task_ids):
                break
            assert time.monotonic() < deadline, (task_ids, completed)
            time.sleep(.01)
        return {"completion_notifications": len(completed), "exactly_once_per_task": True}

if __name__ == "__main__":
    result = {"fixture": str(ROOT), "durable": durable_gate(), "isolated": isolated_gate(),
              "finalization_read": finalization_read_gate(), "partial_batch": partial_batch_gate()}
    (ROOT / "results.json").write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
