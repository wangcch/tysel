#!/usr/bin/env python3
"""Caller attribution on new local namespaces; separate from release acceptance."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import threading
import time
import traceback

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('caller_capacity', HERE / 'capacity.py')
c = importlib.util.module_from_spec(spec)
spec.loader.exec_module(c)
b = c.bench


def proc_snapshot(pid):
    values = {}
    for line in Path(f'/proc/{pid}/smaps_rollup').read_text().splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == 'kB':
            values[parts[0].rstrip(':')] = int(parts[1])
    stat = c.parse_cpu_stat(Path(f'/proc/{pid}/stat').read_text())
    threads = [int(p.name) for p in Path(f'/proc/{pid}/task').iterdir()]
    c.require(all(k in values for k in ('Pss', 'Private_Dirty', 'Private_Clean', 'Anonymous')), 'incomplete smaps')
    return dict(memoryKiB=values, cpu=stat, threads=len(threads))


class Sampler:
    def __init__(self, roles):
        self.roles, self.samples, self.errors = roles, [], []
        self.phase, self.started = 'busy', time.monotonic()
        self.started_unix_ms = time.time_ns() // 1000000
        self.stop = threading.Event()
        self.sample()
        self.identities = {pid: row['cpu']['startTicks'] for pid, row in self.samples[0]['processes'].items()}
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def sample(self):
        processes = {pid: proc_snapshot(int(pid)) for pid in self.roles}
        if hasattr(self, 'identities'):
            c.require(all(row['cpu']['startTicks'] == self.identities[pid] for pid, row in processes.items()), 'PID reuse')
        self.samples.append(dict(atMs=(time.monotonic()-self.started)*1000, phase=self.phase, processes=processes))

    def run(self):
        try:
            while not self.stop.wait(.05): self.sample()
        except BaseException as error: self.errors.append(str(error))

    def close(self):
        self.stop.set(); self.thread.join(timeout=10)
        c.require(not self.thread.is_alive() and not self.errors, 'sampler failed: '+str(self.errors))
        self.sample()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--support', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--rounds', type=int, default=3)
    parser.add_argument('--quiet-seconds', type=int, default=60)
    parser.add_argument('--kind', choices=['baseline', 'instrumented', 'candidate'], required=True)
    parser.add_argument('--clients', type=int, choices=[4,8])
    parser.add_argument('--release', type=Path, help='Reuse exact packaged artifacts from the paired experiment')
    args = parser.parse_args()
    c.require(__debug__ and platform.system() == 'Linux', 'requires Linux and active assertions')
    c.require(args.rounds > 0 and args.quiet_seconds >= 5, 'invalid coverage')
    out=args.output.resolve(); out.mkdir(parents=True,exist_ok=False)
    plan=json.loads((HERE/'capacity-protocol.json').read_text())
    c.validate_plan(plan)
    data=dict(status='running',kind=args.kind,command=__import__('sys').argv,plan=plan,
              measuredJobsPerRound=512,warmups=8,activeIdleSec=5,quietIdleSec=args.quiet_seconds,
              note='No candidate gain claims from instrumented runs; sampler adds observation cost in all runs',
              binaries={n:b.sha(args.bin_dir/n) for n in ('tysel','tysel-service','tysel-worker')},
              supportSha256=b.sha(args.support),runnerSha256=b.sha(Path(__file__)),startedAtUnixMs=time.time_ns()//1000000,rounds=[],processCleanup=[])
    b.save(out/'measurements.json',data)
    support=provider=None
    original_process=b.deploy.Process
    logs=[]
    class Capture(list):
        def __init__(self,initial,path):
            super().__init__(initial);self.file=path.open('w');self.lock=threading.Lock()
            self.file.writelines(initial);self.file.flush();logs.append(self.file)
        def append(self,line):
            with self.lock: self.file.write(line);self.file.flush()
            super().append(line)
    class Process(original_process):
        def __init__(self,*a,**kw):
            super().__init__(*a,**kw)
            self.logs=Capture(self.logs,out/f'process-{self.process.pid}.log')
    b.deploy.Process=Process
    try:
        release=out/'release'
        if args.release:
            b.deploy.verify(args.release)
            shutil.copytree(args.release,release)
            data['package']=b.deploy.verify(release)
        else:
            data['package']=b.package.package(args.bin_dir,release)
        support=b.Support(args.support);provider=c.fixture(plan)
        data['system']=support.call(op='system')
        for block in range(args.rounds):
            cells=[(4,20),(8,0)] if block%2==0 else [(8,0),(4,20)]
            for clients,delay in cells:
                if args.clients is not None and clients != args.clients: continue
                state=out/f'namespace-{block}-c{clients}';provider.capacity_delay_ms=delay;before_reads=provider.count()
                original_http=b.deploy.http;observer=c.paired.Observer(original_http);b.deploy.http=observer
                row=dict(round=block,clients=clients,adapterDelayMs=delay)
                sampler=None
                try:
                    with c.application(release,state,provider,support,observer=observer) as app:
                        c.jobs(app,provider,plan,8,clients,'warm');c.drained(app)
                        pids=support.call(op='memory',roots=b.roots(app))['pids']
                        roles={str(pid):'caller' if pid==app.caller.process.pid else 'plugin' if pid==app.plugin.process.pid else 'worker' for pid in pids}
                        sampler=Sampler(roles)
                        try:
                            cpu_before=c.cpu_snapshot(pids);start=time.monotonic()
                            samples=c.jobs(app,provider,plan,512,clients,'measure')
                            elapsed=(time.monotonic()-start)*1000;cpu_after=c.cpu_snapshot(pids)
                            c.drained(app);sampler.phase='drained-supervisor-active';sampler.sample()
                            print(json.dumps(dict(stage='workload-drained',round=block,clients=clients,jobsPerSec=512000/elapsed)),flush=True)
                            time.sleep(5);sampler.sample()
                            # Only this fresh, drained diagnostic namespace is affected.
                            app.stop.set();app.driver.join(timeout=10)
                            c.require(not app.driver.is_alive() and not app.errors,'supervisor did not drain cleanly')
                            sampler.phase='quiet';sampler.sample()
                            quiet_start=time.monotonic();boundaries={}
                            for second in sorted(set([5,min(30,args.quiet_seconds),args.quiet_seconds])):
                                time.sleep(max(0,quiet_start+second-time.monotonic()));sampler.sample()
                                boundaries[str(second)]=sampler.samples[-1]
                        finally: sampler.close()
                        b.attach_audit(app,samples)
                        c.require(provider.count()-before_reads==520,'physical reads mismatch')
                        c.require(b.query(state,'SELECT count(*) AS n FROM triage_jobs')[0]['n']==520,'retention mismatch')
                        seen=set()
                        for sample in samples:
                            c.paired.checks.sample_check(sample,'lookup',plan['size'],delay,seen,clients=clients)
                        row.update(samples=samples,durationMs=elapsed,jobsPerSec=512000/elapsed,roles=roles,
                                   cpuBefore=cpu_before,cpuAfter=cpu_after,nativeCpuMsPerJob=c.cpu_delta(cpu_before,cpu_after,os.sysconf('SC_CLK_TCK'))/512,
                                   memory=sampler.samples,memoryOriginUnixMs=sampler.started_unix_ms,quietBoundaries=boundaries,physicalReads=520,
                                   quietThreadNames={pid:dict(__import__('collections').Counter(p.read_text().strip() for p in Path(f'/proc/{pid}/task').glob('*/comm'))) for pid in roles})
                finally: b.deploy.http=original_http
                row['httpObservation']=observer.report();data['rounds'].append(row)
                data['processCleanup']=support.cleanup;b.save(out/'measurements.json',data)
                caller=next(pid for pid,role in row['roles'].items() if role=='caller')
                peak=max(s['processes'][caller]['memoryKiB']['Pss'] for s in row['memory'])/1024
                print(json.dumps(dict(round=block,clients=clients,peakCallerMiB=peak,jobsPerSec=row['jobsPerSec'])),flush=True)
        data['status']='complete'
    except BaseException:
        data['status']='error';data['error']=traceback.format_exc();raise
    finally:
        try:
            b.cleanup_all(('provider',provider.close if provider else lambda:None),('support',support.close if support else lambda:None))
        except BaseException:
            data['status']='error';data['cleanupError']=traceback.format_exc();raise
        finally:
            for log in logs: log.close()
            b.deploy.Process=original_process
            data['processCleanup']=support.cleanup if support else [];b.save(out/'measurements.json',data)


if __name__=='__main__': main()
