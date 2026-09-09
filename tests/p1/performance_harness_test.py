"""Bounded failure-path checks for the macOS performance harness; no network."""
import os
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import threading
import time
import types
import unittest


def exercise(case):
    with tempfile.TemporaryDirectory(prefix='tysel-perf-failure-') as root:
        os.environ['TYSEL_AB_ROOT'] = root
        h = runpy.run_path(str(Path(__file__).with_name('performance_ab.py')))
        g = h['phase'].__globals__
        if case in ('client_failure', 'barrier_timeout'):
            stopped = []; closed = []; lock = threading.Lock(); count = [0]
            class Resource:
                def close(self): closed.append(True)
            class Service:
                process = types.SimpleNamespace(pid=0)
                def __init__(self, *args): pass
                def warm(self): pass
                def completed(self): return 0
                def connection(self):
                    with lock:
                        count[0] += 1; index = count[0]
                    if index == 1:
                        if case == 'client_failure': raise OSError('injected connection error')
                        time.sleep(.15)
                    return Resource(), Resource()
                def stop(self, **kwargs): stopped.append(True)
            g.update(Service=Service, usage=lambda pid: {}, CLIENT_READY_TIMEOUT=.05)
            try:
                h['phase']('before', 'one', 2, 0)
                raise AssertionError('failure was not propagated')
            except OSError as error:
                assert case == 'client_failure' and str(error) == 'injected connection error'
            except threading.BrokenBarrierError:
                assert case == 'barrier_timeout'
            assert stopped == [True]
            assert len(closed) == (2 if case == 'client_failure' else 4), closed
        else:
            children = []; connections = []
            popen = subprocess.Popen
            def launch(*args, **kwargs):
                code = "import time;print('startup diagnostic',flush=True);time.sleep(60)"
                if case == 'startup_forced':
                    code = "import signal;signal.signal(signal.SIGTERM,signal.SIG_IGN);" + code
                child = popen([sys.executable, '-u', '-c', code], stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)
                children.append(child)
                return child
            class Http:
                status = 503
                def __init__(self, *args, **kwargs): self.closed = False; connections.append(self)
                def request(self, *args): pass
                def getresponse(self): return self
                def read(self): return b'not ready'
                def close(self): self.closed = True
            g.update(port=lambda: 1, STARTUP_TIMEOUT=.3, STOP_TIMEOUT=.1,
                     subprocess=types.SimpleNamespace(Popen=launch, PIPE=-1, STDOUT=-2,
                                                      TimeoutExpired=subprocess.TimeoutExpired),
                     http=types.SimpleNamespace(client=types.SimpleNamespace(HTTPConnection=Http)))
            try:
                try:
                    h['Service']('before', 'one', case)
                    raise AssertionError('startup did not fail')
                except AssertionError as error:
                    assert str(error) == 'startup timeout', error
                assert all(p.poll() is not None and p.stdout.closed for p in children)
                assert connections and all(c.closed for c in connections)
                assert 'startup diagnostic' in (Path(root)/'measurements'/case/'service.log').read_text()
                if case == 'startup_forced': assert children[0].returncode == -9
            finally:
                for child in children:
                    if child.poll() is None: child.kill(); child.wait(timeout=2)


class FailurePaths(unittest.TestCase):
    def test_bounded_failures(self):
        for case in ('client_failure', 'barrier_timeout', 'startup_timeout', 'startup_forced'):
            with self.subTest(case=case):
                result = subprocess.run([sys.executable, __file__, case], capture_output=True,
                                        text=True, timeout=8)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == '__main__':
    if len(sys.argv) == 2: exercise(sys.argv[1])
    else: unittest.main()
