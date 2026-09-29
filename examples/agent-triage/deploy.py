#!/usr/bin/env python3
"""Run a pinned, single-caller triage release with a separate persistent namespace."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import AbstractContextManager, contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import queue
import shutil
import signal
import sqlite3
import stat
import subprocess
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

ARTIFACTS = ("caller", "plugin", "tysel-worker", "deploy.py")
LIMITS = dict(callMs=5000, jobMs=120000, perCustomer=4, active=8, retained=1000)


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.chmod(0o600)
    temporary.replace(path)


def build_info(path):
    try:
        result = subprocess.run([str(path), "--build-info-json"], capture_output=True, text=True, check=True, timeout=10)
        return json.loads(result.stdout)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        raise RuntimeError(f"cannot identify executable {Path(path).name}; use matching tools for this host") from error


def identity(info):
    return tuple(info.get(key) for key in ("version", "target", "sourceCommit", "releaseId"))


def verify(release):
    release = Path(release).resolve()
    metadata = json.loads((release / "release.json").read_text())
    if metadata.get("schemaVersion") != 1 or set(metadata.get("artifacts", {})) != set(ARTIFACTS):
        raise RuntimeError("invalid release metadata")
    for name in ARTIFACTS:
        path = release / name
        if not path.is_file():
            raise RuntimeError(f"missing {name}; deploy the complete release including matching tysel-worker")
        if sha256(path) != metadata["artifacts"][name]["sha256"]:
            raise RuntimeError(f"artifact mismatch: {name}; restore the original release bytes")
    expected = identity(metadata["toolchain"])
    for name in ("caller", "plugin", "tysel-worker"):
        info = build_info(release / name)
        if identity(info) != expected or info.get("binary") != ("tysel-worker" if name == "tysel-worker" else "tysel-service"):
            raise RuntimeError(f"incompatible toolchain: {name}; use the matching worker and runtime")
    host = ("linux" if platform.system() == "Linux" else "darwin" if platform.system() == "Darwin" else "unsupported")
    arch = {"aarch64": "arm64", "arm64": "arm64", "x86_64": "x64"}.get(platform.machine(), "unsupported")
    if metadata["toolchain"]["target"] != f"{host}-{arch}":
        raise RuntimeError("release target does not match this host")
    return metadata


def artifact_binding(metadata):
    return {name: metadata["artifacts"][name]["sha256"] for name in ARTIFACTS}


class StateLock(AbstractContextManager):
    def __init__(self, state):
        self.file = (Path(state) / "namespace.lock").open("a+")
        try:
            fcntl.flock(self.file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            self.file.close()
            raise RuntimeError("namespace is in use; stop its supervisor before backup, restore or another caller")

    def __exit__(self, *_):
        self.file.close()


def initialize(release, state, settings):
    metadata = verify(release)
    state = Path(state).resolve()
    if state.exists() and any(state.iterdir()):
        raise RuntimeError("initialization requires an empty namespace; existing state is never reset")
    if set(settings) != {"adapterOrigin", "adapterId", "customers"}:
        raise RuntimeError("settings require adapterOrigin, adapterId and customers")
    origin = urllib.parse.urlsplit(settings["adapterOrigin"])
    if origin.scheme != "http" or origin.hostname != "127.0.0.1" or not origin.port or origin.path or origin.query or origin.fragment or origin.username is not None or origin.password is not None:
        raise RuntimeError("this example permits an HTTP loopback adapter origin only")
    if not isinstance(settings["adapterId"], str) or not settings["adapterId"]:
        raise RuntimeError("adapterId must name the configured adapter")
    customers = settings["customers"]
    if not isinstance(customers, list) or not customers:
        raise RuntimeError("customers must be a nonempty development identity mapping")
    ids, tokens, tickets = set(), set(), set()
    for customer in customers:
        if set(customer) != {"id", "token", "tickets"} or not isinstance(customer["tickets"], list):
            raise RuntimeError("invalid customer mapping")
        if not all(isinstance(customer[key], str) and customer[key] for key in ("id", "token")) or customer["id"] in ids or customer["token"] in tokens:
            raise RuntimeError("customer IDs and tokens must be nonempty and distinct")
        ids.add(customer["id"]); tokens.add(customer["token"])
        for ticket in customer["tickets"]:
            if set(ticket) != {"id", "priority"} or not isinstance(ticket["id"], str) or not ticket["id"] or ticket["id"] in tickets or type(ticket["priority"]) is not int or not 0 <= ticket["priority"] <= 3:
                raise RuntimeError("ticket IDs must have one owner and priorities from 0 to 3")
            tickets.add(ticket["id"])
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    state.chmod(0o700)
    for name in ("caller/config", "caller/data", "plugin"):
        (state / name).mkdir(parents=True, exist_ok=True)
    config = dict(settings, bootId=str(uuid.uuid4()), dispatchToken=str(uuid.uuid4()), pluginOrigin="",
                  callerDigest=metadata["artifacts"]["caller"]["sha256"], pluginDigest=metadata["artifacts"]["plugin"]["sha256"], limits=LIMITS)
    write_json(state / "caller/config/service.json", config)
    write_json(state / "binding.json", dict(schemaVersion=1, namespaceId=str(uuid.uuid4()), artifacts=artifact_binding(metadata)))


def http(origin, path, method="GET", body=None, token=None, key=None, timeout=45):
    headers = {"Content-Type": "application/json"}
    if token: headers["Authorization"] = "Bearer " + token
    if key: headers["Idempotency-Key"] = key
    request = urllib.request.Request(origin + path, data=None if body is None else json.dumps(body).encode(), headers=headers, method=method)
    try:
        response = urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, json.loads(response.read())


class Process:
    def __init__(self, executable, cwd, worker, secret=""):
        env = {key: value for key, value in os.environ.items() if not key.startswith(("TYSEL_", "OTEL_", "OPENAI_", "TRIAGE_"))}
        env.update(TYSEL_WORKER=str(worker), TRIAGE_FIXTURE_TOKEN=secret, OTEL_SDK_DISABLED="true",
                   NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        self.logs = []
        self.process = subprocess.Popen([str(executable)], cwd=cwd, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        ready = queue.Queue()
        def drain(stream):
            for line in stream:
                self.logs.append(line)
                del self.logs[:-200]
                if line.startswith("tysel listen "):
                    ready.put("http://" + line.strip().removeprefix("tysel listen "))
            stream.close()
        self.readers = [threading.Thread(target=drain, args=(stream,), daemon=True) for stream in (self.process.stdout, self.process.stderr)]
        for reader in self.readers: reader.start()
        until = time.monotonic() + 12
        while time.monotonic() < until and self.process.poll() is None:
            try:
                self.origin = ready.get(timeout=.1)
                return
            except queue.Empty:
                pass
        self.close()
        raise RuntimeError(f"{Path(executable).name} did not start; check matching worker, embedded permissions and required Linux sandbox: " + "".join(self.logs)[-1500:])

    def close(self, crash=False):
        if self.process.poll() is None:
            self.process.send_signal(signal.SIGKILL if crash else signal.SIGTERM)
            try: self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill(); self.process.wait(timeout=5)
        for reader in self.readers:
            reader.join(timeout=5)
            if reader.is_alive(): raise RuntimeError("child pipe cleanup failed")


class Deployment(AbstractContextManager):
    def __init__(self, release, state, secret, *, dispatch=True):
        self.release, self.state = Path(release).resolve(), Path(state).resolve()
        self.caller = self.plugin = self.driver = None
        self.stop = threading.Event()
        self.errors = []
        self.metadata = verify(self.release)
        self.lock = StateLock(self.state)
        try:
            binding = json.loads((self.state / "binding.json").read_text())
            if binding["artifacts"] != artifact_binding(self.metadata):
                raise RuntimeError("namespace is bound to another release; restart the original artifacts or initialize a new namespace")
            self.config_path = self.state / "caller/config/service.json"
            self.config = json.loads(self.config_path.read_text())
            if self.config["callerDigest"] != binding["artifacts"]["caller"] or self.config["pluginDigest"] != binding["artifacts"]["plugin"]:
                raise RuntimeError("configuration does not match the namespace's pinned artifacts")
            self.plugin = Process(self.release / "plugin", self.state / "plugin", self.release / "tysel-worker")
            self.config.update(bootId=str(uuid.uuid4()), pluginOrigin=self.plugin.origin)
            write_json(self.config_path, self.config)
            self.caller = Process(self.release / "caller", self.state / "caller", self.release / "tysel-worker", secret)
            self.origin = self.caller.origin
            status, result = http(self.origin, "/health")
            if status != 200 or result != {"ready": True}:
                raise RuntimeError("caller health check failed; check config/service.json, embedded fs_read grant and writable SQLite state")
            if dispatch: self.start_driver()
        except BaseException:
            self.close()
            raise

    def start_driver(self):
        self.stop.clear()
        wake = threading.Event()

        def wake_completed(future):
            try:
                status, result = future.result()
            except Exception:
                return  # The driver observes and reports the original exception.
            if status == 200 and isinstance(result, dict) and result.get("status") == "completed":
                wake.set()

        def run():
            pending = {}
            retry_after = {}
            active_until = 0.0
            try:
                with ThreadPoolExecutor(max_workers=8) as pool:
                    while not self.stop.is_set():
                        wake.clear()
                        if self.caller.process.poll() is not None or self.plugin.process.poll() is not None:
                            raise RuntimeError("a deployment process exited; restart the same release and namespace")
                        for job_id, future in list(pending.items()):
                            if future.done():
                                status, result = future.result()
                                if status not in (200, 503): raise RuntimeError(f"dispatch failed with HTTP {status}")
                                if status == 200 and isinstance(result, dict) and result.get("status") == "completed":
                                    active_until = time.monotonic() + .25
                                else:
                                    retry_after[job_id] = time.monotonic() + .25
                                del pending[job_id]
                        now = time.monotonic()
                        retry_after = {job_id: deadline for job_id, deadline in retry_after.items() if deadline > now}
                        status, jobs = http(self.origin, "/internal/pending", token=self.config["dispatchToken"])
                        if status not in (200, 503): raise RuntimeError(f"outbox lookup failed with HTTP {status}")
                        if status == 503:
                            self.stop.wait(.25)
                            continue
                        if status == 200:
                            for job in jobs:
                                if job["id"] not in pending and len(pending) < 8 and time.monotonic() >= retry_after.get(job["id"], 0.0):
                                    future = pool.submit(http, self.origin, "/internal/dispatch", "POST", {"jobId": job["id"]}, self.config["dispatchToken"])
                                    pending[job["id"]] = future
                                    future.add_done_callback(wake_completed)
                        wake.wait(.05 if time.monotonic() < active_until else .25)
            except Exception as error:
                if not self.stop.is_set(): self.errors.append(str(error))
        self.driver = threading.Thread(target=run, daemon=True)
        self.driver.start()

    def close(self, crash=False):
        self.stop.set()
        failures = []
        for process in (self.caller, self.plugin):
            if process:
                try: process.close(crash)
                except Exception as error: failures.append(str(error))
        if self.driver:
            self.driver.join(timeout=10)
            if self.driver.is_alive(): failures.append("dispatcher cleanup failed")
        self.lock.__exit__()
        if failures: raise RuntimeError("; ".join(failures))

    def __exit__(self, *_):
        self.close()


def snapshot_files(root):
    return {str(path.relative_to(root)): sha256(path) for path in sorted(root.rglob("*")) if path.is_file() and path.name not in ("namespace.lock", "snapshot.json")}


def copy_namespace(source, destination, excluded):
    """Fill an exclusively created directory; target mount supplies security labels."""
    for path in sorted(source.iterdir()):
        if path.name in excluded:
            continue
        target = destination / path.name
        if path.is_symlink():
            raise RuntimeError("namespace snapshots require ordinary files and directories, not symlinks")
        if path.is_dir():
            target.mkdir(mode=0o700)
            copy_namespace(path, target, excluded)
        elif path.is_file():
            shutil.copyfile(path, target)
            shutil.copymode(path, target)
        else:
            raise RuntimeError("namespace snapshots require ordinary files and directories")
    # copytree/copystat also copy SELinux xattrs, which can fail when restoring
    # a virtiofs backup to a Podman volume. Keep POSIX modes, not source labels,
    # ACLs, ownership or timestamps. Hash verification covers file contents.
    shutil.copymode(source, destination)


def remove_partial_namespace(destination):
    # Completed subdirectories may already have their source's read-only mode.
    # Only change directories in this newly created copy, never source modes or
    # symlink targets. Parent directories outside the copy are left unchanged.
    def writable(directory):
        os.chmod(directory, 0o700, follow_symlinks=False)
        for child in directory.iterdir():
            if stat.S_ISDIR(child.lstat().st_mode):
                writable(child)
    writable(destination)
    shutil.rmtree(destination)


@contextmanager
def new_namespace(destination):
    # Keep creation outside the cleanup scope: a pre-existing directory, or a
    # different creator winning mkdir, never belongs to this operation.
    destination.mkdir(mode=0o700, parents=True)
    created = destination.lstat()
    try:
        yield
    except BaseException as original:
        try:
            try:
                current = destination.lstat()
            except FileNotFoundError:
                current = None
            if current is not None:
                if (not stat.S_ISDIR(current.st_mode) or
                        (current.st_dev, current.st_ino) != (created.st_dev, created.st_ino)):
                    raise RuntimeError("destination identity changed; refusing to remove it")
                remove_partial_namespace(destination)
        except Exception as cleanup_error:
            raise RuntimeError(f"operation failed: {original}; partial destination cleanup failed at "
                               f"{destination}: {cleanup_error}; inspect the retained path before retrying") from original
        raise


def backup(state, output):
    state, output = Path(state).resolve(), Path(output).resolve()
    if output == state or state in output.parents:
        raise RuntimeError("backup must be outside the state directory")
    if output.exists(): raise RuntimeError("backup destination must not exist")
    with StateLock(state):
        with sqlite3.connect(f"file:{state / 'caller/data/jobs.db'}?mode=ro", uri=True) as db:
            if db.execute("SELECT count(*) FROM triage_jobs WHERE state IN ('accepted','running','recovering') OR delivery='pending'").fetchone()[0]:
                raise RuntimeError("backup requires terminal jobs and completed delivery; restart and drain the original release first")
        with new_namespace(output):
            copy_namespace(state, output, {"namespace.lock"})
            write_json(output / "snapshot.json", dict(schemaVersion=1, mode="stopped-terminal-only", files=snapshot_files(output)))


def restore(release, source, state):
    metadata = verify(release)
    source, state = Path(source).resolve(), Path(state).resolve()
    if state.exists(): raise RuntimeError("restore requires a new destination; existing namespaces are never overwritten")
    if source in state.parents: raise RuntimeError("restore must be outside the backup directory")
    snapshot = json.loads((source / "snapshot.json").read_text())
    if snapshot.get("mode") != "stopped-terminal-only" or snapshot["files"] != snapshot_files(source):
        raise RuntimeError("backup integrity check failed")
    if json.loads((source / "binding.json").read_text())["artifacts"] != artifact_binding(metadata):
        raise RuntimeError("backup requires its original release artifacts")
    with new_namespace(state):
        copy_namespace(source, state, {"snapshot.json", "namespace.lock"})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, default=Path(__file__).resolve().parent)
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("init"); init.add_argument("--state", type=Path, required=True); init.add_argument("--config", type=Path, required=True)
    serve = sub.add_parser("serve"); serve.add_argument("--state", type=Path, required=True)
    save = sub.add_parser("backup"); save.add_argument("--state", type=Path, required=True); save.add_argument("--output", type=Path, required=True)
    load = sub.add_parser("restore"); load.add_argument("--backup", type=Path, required=True); load.add_argument("--state", type=Path, required=True)
    sub.add_parser("verify")
    args = parser.parse_args()
    if args.command == "init": initialize(args.release, args.state, json.loads(args.config.read_text()))
    elif args.command == "backup": backup(args.state, args.output)
    elif args.command == "restore": restore(args.release, args.backup, args.state)
    elif args.command == "verify": verify(args.release); print("release verified")
    else:
        def stop(*_): raise KeyboardInterrupt
        signal.signal(signal.SIGTERM, stop)
        signal.signal(signal.SIGINT, stop)
        with Deployment(args.release, args.state, os.environ.get("TRIAGE_FIXTURE_TOKEN", "")) as app:
            print(json.dumps({"url": app.origin, "state": str(app.state)}), flush=True)
            try:
                while not app.stop.wait(.25):
                    if app.errors: raise RuntimeError("; ".join(app.errors))
            except KeyboardInterrupt: pass


if __name__ == "__main__":
    try: main()
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        raise SystemExit(f"deployment error: {error}")
