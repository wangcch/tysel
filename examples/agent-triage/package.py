#!/usr/bin/env python3
"""Package two embedded applications, their exact worker and a portable supervisor."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from deploy import ARTIFACTS, build_info, identity, sha256, verify

EXAMPLE = Path(__file__).resolve().parent
REPO = EXAMPLE.parents[1]


def package(binary_dir, output, *, caller_source=None, plugin_source=None, caller_manifest=None, plugin_manifest=None):
    binary_dir, output = Path(binary_dir).resolve(), Path(output).resolve()
    if output.exists() and any(output.iterdir()): raise RuntimeError("package output must be empty")
    infos = {name: build_info(binary_dir / name) for name in ("tysel", "tysel-service", "tysel-worker")}
    if any(info.get("binary") != name for name, info in infos.items()) or len({identity(info) for info in infos.values()}) != 1:
        raise RuntimeError("package requires matching CLI, runtime and worker build identities")
    output.mkdir(parents=True, exist_ok=True)
    sources = {}
    builds = {}
    with tempfile.TemporaryDirectory(prefix="triage-package-") as temporary:
        for name, source, manifest in (("caller", caller_source or EXAMPLE / "src", caller_manifest or EXAMPLE / "tysel.toml"),
                                       ("plugin", plugin_source or REPO / "examples/isolated-plugin/src", plugin_manifest or EXAMPLE / "plugin.toml")):
            project = Path(temporary) / name
            shutil.copytree(source, project / "src")
            shutil.copy(manifest, project / "tysel.toml")
            sources[name] = {str(path.relative_to(project)): sha256(path) for path in sorted(project.rglob("*")) if path.is_file()}
            command = [str(binary_dir / "tysel"), "-C", str(project), "build", "--stub", str(binary_dir / "tysel-service"), "--output", str(output / name)]
            env = {key: value for key, value in os.environ.items() if not key.startswith(("TYSEL_", "OTEL_"))}
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=60)
            if result.returncode: raise RuntimeError(f"{name} package failed: {result.stdout}{result.stderr}")
            builds[name] = result.stdout
    # Preserve bytes and executable modes, while the destination filesystem
    # supplies its own security labels (e.g. a Podman named volume).
    shutil.copy(binary_dir / "tysel-worker", output / "tysel-worker")
    shutil.copy(EXAMPLE / "deploy.py", output / "deploy.py")
    metadata = dict(schemaVersion=1, toolchain=infos["tysel-service"], sources=sources, buildOutput=builds,
                    artifacts={name: dict(sha256=sha256(output / name), bytes=(output / name).stat().st_size) for name in ARTIFACTS})
    (output / "release.json").write_text(json.dumps(metadata, indent=2) + "\n")
    verify(output)
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/debug")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    metadata = package(args.bin_dir, args.output)
    print(json.dumps(dict(release=str(args.output.resolve()), target=metadata["toolchain"]["target"], artifacts=list(metadata["artifacts"]))))


if __name__ == "__main__":
    main()
