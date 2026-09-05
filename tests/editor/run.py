#!/usr/bin/env python3
"""Run real VS Code providers in a disposable profile. No npm install required."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile

repo = Path(__file__).resolve().parents[2]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--code", required=True, help="VS Code executable (not a running user profile)")
parser.add_argument("--toml-extension", required=True, type=Path, help="Installed Even Better TOML extension directory")
parser.add_argument("--output", type=Path, help="Optional JSON evidence destination")
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix="tysel-editor-smoke-"))
project = root / "project"
cli = repo / "target/debug/tysel"
if not cli.is_file() or not (repo / "packages/tysel-types/dist/index.d.ts").is_file():
    parser.error("Build tysel-cli and @tysel/types first; see README.md")
subprocess.run([str(cli), "init", str(project), "--yes", "--no-tests"], check=True, capture_output=True)
subprocess.run(["git", "init", "-q", str(project)], check=True)
ignored = subprocess.run(["git", "-C", str(project), "check-ignore", "-q", ".tysel/manifest.schema.json"])
if ignored.returncode != 1:
    raise RuntimeError("The generated editor schema is ignored by Git")
# Resolve the built declarations locally; this tests workspace output, not npm publication.
(project / "node_modules/@tysel").mkdir(parents=True)
(project / "node_modules/@tysel/types").symlink_to(repo / "packages/tysel-types", target_is_directory=True)
(project / "input").mkdir()
manifest = project / "tysel.toml"
manifest.write_text(manifest.read_text().replace('127.0.0.1:3000', '127.0.0.1:0'))
profile = root / "user-data/User"
profile.mkdir(parents=True)
(profile / "settings.json").write_text(json.dumps({
    "telemetry.telemetryLevel": "off", "update.mode": "none",
    "extensions.autoUpdate": False, "extensions.autoCheckUpdates": False,
    "extensions.ignoreRecommendations": True, "security.workspace.trust.enabled": False,
    "workbench.startupEditor": "none", "chat.disableAIFeatures": True,
    "evenBetterToml.schema.catalogs": [], "files.autoSave": "off",
}))
extensions = root / "extensions"
extensions.mkdir()
# Copy only the explicitly selected extension, leaving the normal profile untouched.
shutil.copytree(args.toml_extension.resolve(), extensions / args.toml_extension.name)
report = root / "report.json"
env = os.environ.copy()
env.pop("ELECTRON_RUN_AS_NODE", None)
env.update(TYSEL_EDITOR_CLI=str(cli), TYSEL_EDITOR_REPORT=str(report))
command = [args.code, "--new-window", "--skip-welcome", "--skip-release-notes",
           "--disable-workspace-trust", "--user-data-dir=" + str(root / "user-data"),
           "--extensions-dir=" + str(extensions),
           "--extensionDevelopmentPath=" + str(Path(__file__).parent.resolve()),
           "--extensionTestsPath=" + str(Path(__file__).with_name("test.cjs")), str(project)]
print(f"Isolated editor evidence: {root}", flush=True)
with (root / "editor.log").open("w") as log:
    child = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    try:
        code = child.wait(timeout=180)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGTERM)
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.wait()
        raise RuntimeError(f"Editor verification timed out; inspect {root / 'editor.log'}")
if report.exists():
    evidence = json.loads(report.read_text())
    warnings = [line for line in (root / "editor.log").read_text(errors="replace").splitlines()
                if "TypeScript Server Error" in line or line.startswith("Could not find source file:")]
    evidence["editorWarnings"] = list(dict.fromkeys(warnings))
    report.write_text(json.dumps(evidence, indent=2) + "\n")
    print(report.read_text())
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(report, args.output)
else:
    raise RuntimeError(f"Editor did not write a report; inspect {root / 'editor.log'}")
raise SystemExit(code)
