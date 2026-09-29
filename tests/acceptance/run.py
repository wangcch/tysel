#!/usr/bin/env python3
"""Run bounded deployment/recovery gates and retain evidence, including on failure."""
import argparse
from dataclasses import dataclass, field
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import sys
import tempfile
import time

REPO = Path(__file__).resolve().parents[2]
BINARIES = ("tysel", "tysel-service", "tysel-worker")
CRASH_MODES = (
    "initial_admission", "before_effect", "after_commit", "after_return",
    "after_record", "projection_wait",
)


@dataclass
class Case:
    name: str
    script: str
    backend: str = "sqlite"
    instances: int = 1
    modes: tuple = ()
    timeout: int = 240
    arguments: tuple = ()
    environment: dict = field(default_factory=dict)


def cases_for(suite):
    cases = [Case("workflows", "tests/workflows/run.py",
                  arguments=("--output", "workflow-report.json"))]
    cases.append(Case("agent-triage", "tests/acceptance/agent_triage.py",
                      arguments=("--output", "agent-triage-report.json")))
    cases.append(Case("agent-triage-recovery", "tests/acceptance/agent_triage_recovery.py",
                      timeout=360, arguments=("--output", "agent-triage-recovery-report.json")))
    cases.append(Case("agent-triage-adversarial", "tests/acceptance/agent_triage_adversarial.py",
                      timeout=360, arguments=("--output", "agent-triage-adversarial-report.json")))
    cases.append(Case("agent-triage-deployment", "tests/acceptance/agent_triage_deployment.py",
                      timeout=360, arguments=("--output", "agent-triage-deployment-report.json")))
    cases.append(Case("agent-triage-initialization", "tests/acceptance/agent_triage_initialization.py",
                      timeout=120, arguments=("--output", "agent-triage-initialization-report.json")))
    cases.extend([
        Case("http-runtime-run", "tests/p1/acceptance.py", environment={"TYSEL_P1_USE_RUN": "1"}),
        Case("http-runtime-standalone", "tests/p1/acceptance.py"),
    ])
    for name in ("completion_recovery", "storage_recovery", "reload_recovery",
                 "error_isolation", "review_regressions"):
        cases.append(Case(name, f"tests/p1/{name}.py"))
    modes = CRASH_MODES if suite != "smoke" else (
        "initial_admission", "after_commit", "after_record",
    )
    cases.append(Case("crash-sqlite", "tests/p1/crash_recovery.py", modes=modes))
    if suite != "smoke":
        cases.extend(Case(f"crash-postgres-{instances}", "tests/p1/crash_recovery.py",
                          backend="postgres", instances=instances, modes=CRASH_MODES)
                     for instances in (1, 2))
    return cases


def timestamp():
    return datetime.now(timezone.utc).isoformat()


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def source_identity():
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=REPO, timeout=15)
    # Hash untracked acceptance sources as well as the tracked working diff.
    untracked = git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0")
    return {
        "commit": git("rev-parse", "HEAD").decode().strip(),
        "dirty": bool(git("status", "--porcelain")),
        "trackedDiffSha256": hashlib.sha256(git("diff", "HEAD", "--binary")).hexdigest(),
        "untrackedFiles": {
            name.decode(): sha256(REPO / name.decode()) for name in untracked
            if name and (REPO / name.decode()).is_file()
        },
    }


def binary_identity(bin_dir, target, commit, require_commit):
    binaries = {}
    for name in BINARIES:
        path = (bin_dir / name).resolve(strict=True)
        result = subprocess.run([str(path), "--build-info-json"], capture_output=True,
                                text=True, check=True, timeout=15)
        info = json.loads(result.stdout)
        if info.get("schemaVersion") != 1 or info.get("binary") != name:
            raise ValueError(f"invalid build information for {name}")
        if info.get("target") != target:
            raise ValueError(f"{name} does not match target {target}")
        if require_commit and info.get("sourceCommit") != commit:
            raise ValueError(f"{name} was not built from the checked-out commit")
        binaries[name] = {"path": str(path), "sha256": sha256(path), "buildInfo": info}
    identities = {tuple(item["buildInfo"].get(key) for key in
                        ("version", "target", "sourceCommit", "releaseId"))
                  for item in binaries.values()}
    if len(identities) != 1:
        raise ValueError("the three tools are not from the same build identity")
    return binaries


def case_environment(bin_dir, fixtures, case, postgres_url):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("TYSEL_", "OTEL_", "OPENAI_", "PYTHON"))}
    env.update(TMPDIR=str(fixtures), TMP=str(fixtures), TEMP=str(fixtures),
               PYTHONUNBUFFERED="1", PYTHONDONTWRITEBYTECODE="1", OTEL_SDK_DISABLED="true",
               TYSEL_GATE_BIN_DIR=str(bin_dir), TYSEL_BIN=str(bin_dir / "tysel"),
               TYSEL_GATE_INSTANCES=str(case.instances))
    if case.modes:
        env["TYSEL_GATE_CRASH_MODES"] = ",".join(case.modes)
    if case.backend == "postgres":
        env["TYSEL_GATE_POSTGRES_URL"] = postgres_url
        env["TYSEL_GATE_PSQL"] = os.environ.get("TYSEL_GATE_PSQL", "psql")
    env.update(case.environment)
    return env


def stop_group(process):
    """The case owns a new session; stop its descendants even if its parent exited."""
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            break
        if sig == signal.SIGTERM:
            time.sleep(.2)
    process.wait(timeout=5)


def collect_diagnostics(fixtures, destination):
    # Exclude binaries, databases and linked node_modules from CI uploads.
    for directory, dirs, files in os.walk(fixtures):
        dirs[:] = [name for name in dirs if name not in ("node_modules", ".git")]
        for name in files:
            source = Path(directory) / name
            if source.is_symlink() or not (name.endswith(".log") or name == "results.json"):
                continue
            target = destination / source.relative_to(fixtures)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)


def run_case(case, bin_dir, output, postgres_url=None):
    destination = output / case.name
    destination.mkdir()
    command = [sys.executable, "-u", str(REPO / case.script), *case.arguments]
    result = {"name": case.name, "backend": case.backend, "instances": case.instances,
              "crashModes": list(case.modes), "command": command,
              "timeoutSeconds": case.timeout, "startedAt": timestamp(), "status": "failed"}
    write_json(destination / "case.json", {**result, "status": "running"})
    started = time.monotonic()
    with tempfile.TemporaryDirectory(prefix="tysel-acceptance-") as temporary:
        fixtures = Path(temporary)
        try:
            env = case_environment(bin_dir, fixtures, case, postgres_url)
            with (destination / "runner.log").open("w") as log:
                process = subprocess.Popen(command, cwd=destination, env=env,
                                           stdout=log, stderr=subprocess.STDOUT,
                                           start_new_session=True)
                try:
                    result["exitCode"] = process.wait(timeout=case.timeout)
                    result["status"] = "passed" if result["exitCode"] == 0 else "failed"
                except subprocess.TimeoutExpired:
                    result["status"] = "timed_out"
                finally:
                    stop_group(process)
        except Exception as error:
            result.update(status="failed", error=str(error))
        finally:
            collect_diagnostics(fixtures, destination / "fixtures")
    result.update(finishedAt=timestamp(), elapsedSeconds=round(time.monotonic() - started, 3))
    write_json(destination / "case.json", result)
    return result


def execute_cases(cases, bin_dir, output, report, postgres_url=None):
    report["plannedCases"] = [case.name for case in cases]
    write_json(output / "evidence.json", report)
    for case in cases:
        print(f"Acceptance: {case.name}", flush=True)
        result = run_case(case, bin_dir, output, postgres_url)
        report["cases"].append(result)
        write_json(output / "evidence.json", report)
        print(f"  {result['status']} ({result['elapsedSeconds']}s)", flush=True)
        if result["status"] != "passed":
            break
    report["status"] = "passed" if cases and all(
        row["status"] == "passed" for row in report["cases"]
    ) and len(report["cases"]) == len(cases) else "failed"
    return 0 if report["status"] == "passed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("smoke", "full", "release"), required=True)
    parser.add_argument("--bin-dir", type=Path, required=True)
    parser.add_argument("--profile", choices=("debug", "release"), required=True,
                        help="Build profile used by the caller; recorded separately from build metadata")
    parser.add_argument("--target", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New or empty evidence directory")
    parser.add_argument("--require-build-commit", action="store_true")
    parser.add_argument("--case", action="append", dest="selected_cases",
                        help="Run a named case for local diagnosis; forbidden for release admission")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if any(output.iterdir()):
        parser.error("evidence directory must be empty; previous results must not be reused")
    bin_dir = args.bin_dir.resolve()
    report = {"schemaVersion": 1, "suite": args.suite, "status": "running",
              "startedAt": timestamp(), "target": args.target, "buildProfile": args.profile,
              "host": {"os": platform.system(), "architecture": platform.machine()},
              "caseSelection": args.selected_cases or "all", "cases": []}
    code = 1
    write_json(output / "evidence.json", report)
    try:
        report["source"] = source_identity()
        report["binaries"] = binary_identity(bin_dir, args.target, report["source"]["commit"],
                                             args.require_build_commit)
        postgres_url = os.environ.get("TYSEL_GATE_POSTGRES_URL")
        cases = cases_for(args.suite)
        if args.selected_cases:
            if args.suite == "release":
                raise ValueError("release admission cannot select a partial case matrix")
            if not set(args.selected_cases) <= {case.name for case in cases}:
                raise ValueError("unknown acceptance case")
            cases = [case for case in cases if case.name in args.selected_cases]
        if args.suite == "release":
            if args.profile != "release" or not args.require_build_commit or report["source"]["dirty"]:
                raise ValueError("release gates require release tools, --require-build-commit and a clean checkout")
        if any(case.backend == "postgres" for case in cases):
            if not postgres_url or not shutil.which(os.environ.get("TYSEL_GATE_PSQL", "psql")):
                raise ValueError("full/release gates require TYSEL_GATE_POSTGRES_URL and a psql client")
        code = execute_cases(cases, bin_dir, output, report, postgres_url)
        # Detect an artifact being replaced while the acceptance suite was running.
        if any(sha256(Path(item["path"])) != item["sha256"] for item in report["binaries"].values()):
            raise ValueError("a tool binary changed during acceptance")
    except KeyboardInterrupt:
        report.update(status="cancelled")
        code = 130
    except Exception as error:
        report.update(status="failed", error=str(error))
        code = 1
    finally:
        report["finishedAt"] = timestamp()
        write_json(output / "evidence.json", report)
    print(f"Acceptance {report['status']}: {output / 'evidence.json'}", flush=True)
    return code


if __name__ == "__main__":
    raise SystemExit(main())
