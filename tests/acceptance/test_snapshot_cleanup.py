#!/usr/bin/env python3
"""Offline backup/restore failure injection; no runtime, container or network.

The source namespaces, SQLite files, artifact binding and copy functions are
real. Only restore's executable metadata verification is mocked: these tests
cover snapshot cleanup, not artifact admission or SIGKILL/power-loss recovery.
"""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import tempfile
import traceback
import unittest
from unittest.mock import patch


REPO = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("snapshot_cleanup_deploy", REPO / "examples/agent-triage/deploy.py")
deploy = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(deploy)


def fingerprint(root):
    """Include bytes, modes, empty directories and links without following them."""
    result = {}
    for path in [root] + sorted(root.rglob("*")):
        info = path.lstat()
        if stat.S_ISLNK(info.st_mode):
            content = ("symlink", os.readlink(path))
        elif stat.S_ISREG(info.st_mode):
            content = ("file", path.read_bytes())
        elif stat.S_ISDIR(info.st_mode):
            content = ("directory",)
        else:
            content = ("special", stat.S_IFMT(info.st_mode))
        result[str(path.relative_to(root))] = (stat.S_IMODE(info.st_mode), content)
    return result


def exception_chain(error):
    pending, seen = [error], set()
    while pending:
        current = pending.pop()
        if id(current) in seen:
            continue
        seen.add(id(current))
        yield current
        pending.extend(item for item in (current.__cause__, current.__context__) if item is not None)


class SnapshotCleanupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="tysel-snapshot-cleanup-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.state = self.root / "state"
        (self.state / "caller/config").mkdir(parents=True)
        (self.state / "caller/data").mkdir()
        (self.state / "namespace.lock").touch(mode=0o600)
        self.metadata = dict(schemaVersion=1, artifacts={
            name: dict(sha256=str(index) * 64) for index, name in enumerate(deploy.ARTIFACTS)})
        deploy.write_json(self.state / "binding.json", dict(artifacts=deploy.artifact_binding(self.metadata)))
        deploy.write_json(self.state / "caller/config/service.json", dict(fixture="offline-only"))
        with sqlite3.connect(self.state / "caller/data/jobs.db") as db:
            db.execute("CREATE TABLE triage_jobs (state TEXT, delivery TEXT)")
            db.execute("INSERT INTO triage_jobs VALUES ('succeeded', 'completed')")
        with sqlite3.connect(self.state / "caller/data/durable-events.db") as db:
            db.execute("CREATE TABLE fixture_events (value TEXT)")
            db.execute("INSERT INTO fixture_events VALUES ('retained history')")
        for path, mode in ((self.state, 0o700), (self.state / "caller", 0o710),
                           (self.state / "caller/config", 0o700), (self.state / "caller/data", 0o750),
                           (self.state / "caller/data/jobs.db", 0o640),
                           (self.state / "caller/data/durable-events.db", 0o600)):
            path.chmod(mode)
        self.parent = self.root / "destinations"
        self.parent.mkdir()
        (self.parent / "sibling").mkdir()
        (self.parent / "sibling/keep.txt").write_bytes(b"unrelated destination")
        self.sibling_before = fingerprint(self.parent / "sibling")
        self.counter = 0

    def case_paths(self, operation):
        self.counter += 1
        source = self.state
        if operation == "restore":
            source = self.root / f"snapshot-{self.counter}"
            deploy.backup(self.state, source)
        return source, self.parent / f"{operation}-{self.counter}"

    def invoke(self, operation, source, destination):
        if operation == "backup":
            return deploy.backup(source, destination)
        # No executables are built or run; the binding itself is checked for real.
        with patch.object(deploy, "verify", return_value=self.metadata):
            return deploy.restore(self.root / "unused-release", source, destination)

    def source_and_sibling_unchanged(self, source, before):
        self.assertEqual(fingerprint(source), before)
        self.assertEqual(fingerprint(self.parent / "sibling"), self.sibling_before)
        self.assertTrue(self.parent.is_dir())
        # Every backup failure path must release its real flock.
        with deploy.StateLock(self.state):
            pass

    def retry_same_path(self, operation, source, destination):
        self.invoke(operation, source, destination)
        self.assertTrue(destination.is_dir())
        self.assertEqual(deploy.snapshot_files(destination), deploy.snapshot_files(source))
        if operation == "backup":
            manifest = json.loads((destination / "snapshot.json").read_text())
            self.assertEqual(manifest["mode"], "stopped-terminal-only")
            self.assertEqual(manifest["files"], deploy.snapshot_files(source))
        else:
            self.assertFalse((destination / "snapshot.json").exists())
        self.assertFalse((destination / "namespace.lock").exists())

    def test_special_files_are_rejected_without_residue_and_same_path_can_retry(self):
        for operation in ("backup", "restore"):
            for fault in ("fifo", "symlink"):
                with self.subTest(operation=operation, fault=fault):
                    source, destination = self.case_paths(operation)
                    invalid = source / ("zz-" + fault)
                    if fault == "fifo":
                        os.mkfifo(invalid)
                    else:
                        # Directory symlinks are excluded by snapshot_files, so
                        # restore reaches copy rejection rather than hash denial.
                        invalid.symlink_to(source / "caller", target_is_directory=True)
                    before = fingerprint(source)
                    try:
                        with self.assertRaisesRegex(RuntimeError, "ordinary files and directories"):
                            self.invoke(operation, source, destination)
                        self.assertFalse(destination.exists(), "failed snapshot left its destination behind")
                        self.source_and_sibling_unchanged(source, before)
                    finally:
                        invalid.unlink()
                    self.retry_same_path(operation, source, destination)

    def test_copy_failure_after_partial_writes_cleans_only_owned_destination(self):
        copyfile = shutil.copyfile
        for operation in ("backup", "restore"):
            with self.subTest(operation=operation):
                source, destination = self.case_paths(operation)
                before = fingerprint(source)
                failure = OSError("injected copy failure")
                copied = []
                def fail_after_copy(src, dst, *args, **kwargs):
                    result = copyfile(src, dst, *args, **kwargs)
                    copied.append(Path(dst))
                    if len(copied) == 2:
                        raise failure
                    return result
                with patch.object(deploy.shutil, "copyfile", side_effect=fail_after_copy):
                    with self.assertRaises(OSError) as raised:
                        self.invoke(operation, source, destination)
                self.assertIs(raised.exception, failure)
                self.assertEqual(len(copied), 2)
                self.assertFalse(destination.exists())
                self.source_and_sibling_unchanged(source, before)
                self.retry_same_path(operation, source, destination)

    def test_manifest_write_failure_removes_copied_snapshot_and_temporary_file(self):
        source, destination = self.case_paths("backup")
        before = fingerprint(source)
        failure = OSError("injected manifest write failure")
        def fail_manifest(path, value):
            self.assertEqual(path, destination / "snapshot.json")
            self.assertTrue((destination / "caller/data/jobs.db").is_file())
            path.with_suffix(".tmp").write_text("partial manifest")
            raise failure
        with patch.object(deploy, "write_json", side_effect=fail_manifest):
            with self.assertRaises(OSError) as raised:
                deploy.backup(source, destination)
        self.assertIs(raised.exception, failure)
        self.assertFalse(destination.exists())
        self.source_and_sibling_unchanged(source, before)
        self.retry_same_path("backup", source, destination)

    def test_existing_file_or_directory_is_never_removed(self):
        for operation in ("backup", "restore"):
            for kind in ("file", "directory"):
                with self.subTest(operation=operation, kind=kind):
                    source, destination = self.case_paths(operation)
                    if kind == "file":
                        destination.write_bytes(b"preexisting file")
                        destination.chmod(0o640)
                    else:
                        destination.mkdir(mode=0o750)
                        (destination / "keep.txt").write_bytes(b"preexisting directory")
                    before, existing = fingerprint(source), fingerprint(destination)
                    with self.assertRaisesRegex(RuntimeError, "destination"):
                        self.invoke(operation, source, destination)
                    self.assertEqual(fingerprint(destination), existing)
                    self.source_and_sibling_unchanged(source, before)

    def test_creator_that_wins_mkdir_race_is_never_removed(self):
        mkdir = Path.mkdir
        for operation in ("backup", "restore"):
            for kind in ("file", "directory"):
                with self.subTest(operation=operation, kind=kind):
                    source, destination = self.case_paths(operation)
                    before = fingerprint(source)
                    winner = []
                    def competing_creator(path, *args, **kwargs):
                        if path == destination:
                            if kind == "file":
                                path.write_bytes(b"competing file")
                            else:
                                mkdir(path, *args, **kwargs)
                                (path / "winner.txt").write_bytes(b"competing directory")
                            winner.append(fingerprint(path))
                            raise FileExistsError("injected destination creation race")
                        return mkdir(path, *args, **kwargs)
                    with patch.object(Path, "mkdir", competing_creator):
                        with self.assertRaises(FileExistsError):
                            self.invoke(operation, source, destination)
                    self.assertEqual(len(winner), 1)
                    self.assertEqual(fingerprint(destination), winner[0])
                    self.source_and_sibling_unchanged(source, before)

    def test_read_only_copied_directory_does_not_prevent_failure_cleanup(self):
        copyfile = shutil.copyfile
        for operation in ("backup", "restore"):
            with self.subTest(operation=operation):
                source, destination = self.case_paths(operation)
                readonly = source / "aaa-readonly"
                readonly.mkdir()
                (readonly / "retained.txt").write_bytes(b"mode must remain unchanged")
                (readonly / "retained.txt").chmod(0o400)
                readonly.chmod(0o500)
                # A restore's hash manifest must include the added ordinary file.
                if operation == "restore":
                    deploy.write_json(source / "snapshot.json", dict(schemaVersion=1, mode="stopped-terminal-only",
                                                                    files=deploy.snapshot_files(source)))
                before = fingerprint(source)
                failure = OSError("injected failure after read-only directory copied")
                observed = []
                def fail_after_readonly(src, dst, *args, **kwargs):
                    if Path(dst) == destination / "binding.json":
                        observed.append(stat.S_IMODE((destination / "aaa-readonly").stat().st_mode))
                        raise failure
                    return copyfile(src, dst, *args, **kwargs)
                try:
                    with patch.object(deploy.shutil, "copyfile", side_effect=fail_after_readonly):
                        with self.assertRaises(OSError) as raised:
                            self.invoke(operation, source, destination)
                    self.assertIs(raised.exception, failure)
                    self.assertEqual(observed, [0o500])
                    self.assertFalse(destination.exists())
                    self.source_and_sibling_unchanged(source, before)
                    self.retry_same_path(operation, source, destination)
                    self.assertEqual(stat.S_IMODE((destination / "aaa-readonly").stat().st_mode), 0o500)
                finally:
                    readonly.chmod(0o700)
                    shutil.rmtree(readonly)

    def test_replaced_destination_root_is_never_removed_or_chmodded(self):
        copyfile = shutil.copyfile
        for operation in ("backup", "restore"):
            for kind in ("directory", "symlink"):
                with self.subTest(operation=operation, replacement=kind):
                    source, destination = self.case_paths(operation)
                    before = fingerprint(source)
                    detached = self.parent / f"detached-{self.counter}"
                    original = OSError("injected failure after destination replacement")
                    winner = []
                    def replace_root(src, dst, *args, **kwargs):
                        copyfile(src, dst, *args, **kwargs)
                        destination.rename(detached)
                        if kind == "directory":
                            destination.mkdir(mode=0o750)
                            (destination / "winner.txt").write_bytes(b"replacement owner")
                        else:
                            destination.symlink_to(self.parent / "sibling", target_is_directory=True)
                        winner.append(fingerprint(destination))
                        raise original
                    with patch.object(deploy.shutil, "copyfile", side_effect=replace_root):
                        with self.assertRaisesRegex(RuntimeError, "identity changed") as raised:
                            self.invoke(operation, source, destination)
                    self.assertTrue(any(error is original for error in exception_chain(raised.exception)))
                    self.assertIn(str(destination), str(raised.exception))
                    self.assertEqual(fingerprint(destination), winner[0])
                    self.assertTrue(detached.is_dir())
                    self.source_and_sibling_unchanged(source, before)
                    if kind == "symlink":
                        destination.unlink()
                    else:
                        shutil.rmtree(destination)
                    shutil.rmtree(detached)
                    self.retry_same_path(operation, source, destination)

    def test_cleanup_failure_keeps_original_error_and_identifies_residual_path(self):
        copyfile, rmtree = shutil.copyfile, shutil.rmtree
        for operation in ("backup", "restore"):
            with self.subTest(operation=operation):
                source, destination = self.case_paths(operation)
                before = fingerprint(source)
                original = OSError("injected copy failure")
                cleanup = OSError("injected cleanup failure")
                def fail_copy(src, dst, *args, **kwargs):
                    copyfile(src, dst, *args, **kwargs)
                    raise original
                with patch.object(deploy.shutil, "copyfile", side_effect=fail_copy):
                    with patch.object(deploy.shutil, "rmtree", side_effect=cleanup):
                        with self.assertRaises(Exception) as raised:
                            self.invoke(operation, source, destination)
                self.assertTrue(destination.is_dir(), "injected cleanup failure must retain partial data")
                chain = list(exception_chain(raised.exception))
                self.assertTrue(any(error is original for error in chain), "original copy failure was hidden")
                formatted = "".join(traceback.format_exception(type(raised.exception), raised.exception,
                                                             raised.exception.__traceback__))
                self.assertIn("injected copy failure", formatted)
                self.assertIn("injected cleanup failure", formatted)
                self.assertIn(str(destination), str(raised.exception))
                self.assertRegex(str(raised.exception).lower(), "cleanup|partial|residu|remov|left")
                self.source_and_sibling_unchanged(source, before)
                rmtree(destination)
                self.retry_same_path(operation, source, destination)


if __name__ == "__main__":
    unittest.main(verbosity=2)
