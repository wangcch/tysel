#!/usr/bin/env python3
"""Validate real example workflows against locally built tools, without external services."""
import argparse
import contextlib
from datetime import datetime, timezone
import http.client
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import threading
import time

REPO = Path(__file__).resolve().parents[2]
CLI = REPO / "target/debug/tysel"
SERVICE = REPO / "target/debug/tysel-service"
WORKER = REPO / "target/debug/tysel-worker"


def wait_for(read, accept=bool, timeout=15):
    end = time.monotonic() + timeout
    value = None
    while time.monotonic() < end:
        value = read()
        if accept(value):
            return value
        time.sleep(.05)
    raise AssertionError(f"Timed out; last value: {value}")


class Running:
    def __init__(self, command, root, env):
        self.lines, self.events = [], []
        self.child = subprocess.Popen([str(x) for x in command], cwd=root, env=env,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        def drain(pipe):
            for line in pipe:
                self.lines.append(line.rstrip())
                try:
                    event = json.loads(line)
                    if isinstance(event, dict) and event.get("event") == "diagnostics":
                        self.events.append(event)
                except ValueError:
                    pass
        for pipe in [self.child.stdout, self.child.stderr]:
            threading.Thread(target=drain, args=(pipe,), daemon=True).start()

    def address(self):
        def read():
            for line in self.lines:
                if line.startswith("tysel listen "):
                    return line.split()[-1]
            if self.child.poll() is not None:
                raise AssertionError("Startup failed: " + "\n".join(self.lines))
        return wait_for(read)

    def close(self):
        if self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.child.kill()
                self.child.wait()

    def request(self, route="/", method="GET", body=None):
        connection = http.client.HTTPConnection(self.address(), timeout=10)
        connection.request(method, route, None if body is None else json.dumps(body),
                           {"Content-Type": "application/json"})
        response = connection.getresponse()
        data = json.loads(response.read())
        status = response.status
        connection.close()
        return status, data

    def edit(self, path, text):
        generation = self.events[-1]["generation"]
        path.write_text(text)
        return wait_for(lambda: next((event for event in reversed(self.events)
                                     if event["generation"] > generation), None))


@contextlib.contextmanager
def running(command, root, env):
    server = Running(command, root, env)
    try:
        server.address()
        yield server
    finally:
        server.close()


class Provider(BaseHTTPRequestHandler):
    calls = 0

    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", "0")))
        Provider.calls += 1
        body = json.dumps({"id": "local-workflow-test", "output_text": "Account looks healthy",
                           "usage": {"input_tokens": 4, "output_tokens": 3}}).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def run_command(root, env, *args, success=True):
    result = subprocess.run([str(CLI), *args], cwd=root, env=env, capture_output=True, text=True, timeout=60)
    assert (result.returncode == 0) == success, result.stdout + result.stderr
    return result


def verify(name, base, env):
    root = base / name
    run_command(base, env, "init", str(root), "--yes", "--no-tests")
    example = REPO / "examples" / name
    shutil.copytree(example / "src", root / "src", dirs_exist_ok=True)
    manifest = root / "tysel.toml"
    original_manifest = (example / "tysel.toml").read_text().replace("127.0.0.1:3000", "127.0.0.1:0")
    manifest.write_text(original_manifest)
    # The generated tsconfig is standalone. Link only built workspace types and the pinned compiler.
    (root / "node_modules/@tysel").mkdir(parents=True)
    (root / "node_modules/@tysel/types").symlink_to(REPO / "packages/tysel-types", target_is_directory=True)
    (root / "node_modules/typescript").symlink_to(REPO / "node_modules/typescript", target_is_directory=True)
    (root / "data").mkdir()
    (root / "data/example.txt").write_text("must remain inaccessible to isolated code")
    source = root / "src/index.ts"
    original_source = source.read_text()
    stages = []
    with running([CLI, "--error-format", "json", "dev"], root, env) as dev:
        wait_for(lambda: dev.events)
        assert dev.request()[0] == 200
        declarations = root / "tysel-env.d.ts"
        before = declarations.read_text()
        modified = re.sub(r'secrets\s*=\s*\[[^\]]*\]', 'secrets = ["OPENAI_API_KEY", "WORKFLOW_TOKEN"]', original_manifest)
        if modified == original_manifest:
            modified = original_manifest.replace('[permissions]', '[permissions]\nsecrets = ["WORKFLOW_TOKEN"]')
        assert dev.edit(manifest, modified)["diagnostics"] == []
        assert "WORKFLOW_TOKEN" in declarations.read_text()
        stages.append("dev permission change refreshes types")
        invalid = modified.replace('[server]', '[server]\nworkers = 0')
        last_valid = declarations.read_text()
        error = dev.edit(manifest, invalid)["diagnostics"][0]
        assert error["code"] == "TYSEL_MANIFEST_INVALID"
        assert declarations.read_text() == last_valid
        assert dev.request()[0] == 200
        cli_error = json.loads(run_command(root, env, "--error-format", "json", "check", success=False).stderr)
        assert cli_error["diagnostics"][0] == error
        assert dev.edit(manifest, original_manifest)["diagnostics"] == []
        assert declarations.read_text() == before
        stages.append("invalid manifest preserves service and types; CLI/dev diagnostics agree")
        bad_source = "import 'node:fs';\n" + original_source
        error = dev.edit(source, bad_source)["diagnostics"][0]
        assert error["code"] == "TYSEL_NODE_BUILTIN_UNSUPPORTED" and error["start"]["line"] == 1
        assert dev.request()[0] == 200
        assert dev.edit(source, original_source)["diagnostics"] == []
        stages.append("source error is located and cleared after repair")
        if name == "isolated-plugin":
            for route in ["/probe/fetch", "/probe/filesystem"]:
                status, data = dev.request(route)
                assert status == 403 and data["denied"] is True
            stages.append("isolated dev denies network and filesystem probes")
    result = run_command(root, env, "check")
    assert "types     ok" in result.stdout, result.stdout
    run_command(root, env, "types", "--check")
    stages.append("pinned TypeScript check and read-only generated-type check pass")
    binary = root / "dist" / name
    result = run_command(root, env, "build", "--stub", str(SERVICE), "--output", str(binary))
    assert "Type check       passed" in result.stdout
    if name == "isolated-plugin":
        assert "matching tysel-worker required" in result.stdout
    stages.append("native development artifact builds with typecheck enabled")
    # Run from a deployment directory containing no TS source, node_modules, or manifest.
    deploy = root / "deployment"
    deploy.mkdir()
    app = deploy / name
    shutil.copy2(binary, app)
    if name == "isolated-plugin":
        missing = subprocess.run([str(app)], cwd=deploy, env=env, capture_output=True, text=True, timeout=15)
        assert missing.returncode != 0 and "worker binary not found" in missing.stderr, missing.stderr
        shutil.copy2(WORKER, deploy / "tysel-worker")
        stages.append("isolated deployment requires matching worker companion; missing worker fails clearly")
    with running([app], deploy, env) as packaged:
        assert packaged.request()[0] == 200
        if name == "isolated-plugin":
            for route in ["/probe/fetch", "/probe/filesystem"]:
                status, data = packaged.request(route)
                assert status == 403 and data["denied"] is True
        if name == "durable-agent":
            initial_calls = Provider.calls
            status, data = packaged.request("/runs", "POST", {"customerId": "acceptance-1"})
            assert status == 202 and data["status"] == "awaiting_approval", data
            run_id = data["runId"]
    stages.append("artifact runs without source, manifest, or npm dependencies")
    if name == "durable-agent":
        with running([app], deploy, env) as restarted:
            assert restarted.request("/runs/" + run_id)[1]["status"] == "awaiting_approval"
            restarted.request("/runs/" + run_id + "/approval", "POST", {"approved": True})
            completed = wait_for(lambda: restarted.request("/runs/" + run_id)[1], lambda row: row["status"] == "completed")
            assert completed["saveCount"] == 1
        with running([app], deploy, env) as restarted_again:
            assert restarted_again.request("/runs/" + run_id)[1]["saveCount"] == 1
        assert Provider.calls - initial_calls == 1
        stages.append("two artifact restarts preserve durable state: one LLM call and one final save")
    return {"example": name, "passed": True, "stages": stages}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    base = Path(tempfile.mkdtemp(prefix="tysel-workflows-")).resolve()
    print(f"Workflow fixtures: {base}", flush=True)
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("TYSEL_", "OPENAI_"))}
    provider = ThreadingHTTPServer(("127.0.0.1", 0), Provider)
    threading.Thread(target=provider.serve_forever, daemon=True).start()
    env.update(TYSEL_LLM_ENDPOINT=f"http://127.0.0.1:{provider.server_port}/v1/responses",
               TYSEL_LLM_MODEL="workflow-test", OPENAI_API_KEY="local-test-key")
    report = {"timestamp": datetime.now(timezone.utc).isoformat(), "platform": os.uname().sysname, "architecture": os.uname().machine, "scope": "Local debug tools/artifacts; fake local LLM; no release or Linux isolation claim", "projects": []}
    try:
        for name in ["hello-service", "isolated-plugin", "durable-agent"]:
            try:
                result = verify(name, base, env)
            except Exception as error:
                result = {"example": name, "passed": False, "error": str(error)}
            report["projects"].append(result)
            print(json.dumps(result), flush=True)
    finally:
        provider.shutdown()
        provider.server_close()
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
    return 0 if all(row["passed"] for row in report["projects"]) else 1


if __name__ == "__main__":
    raise SystemExit(main())
