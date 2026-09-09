#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import unittest
import subprocess
import sys
import tempfile

spec = importlib.util.spec_from_file_location("release_notes", Path(__file__).with_name("release-notes.py"))
notes = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notes)


class ReleaseNotesTests(unittest.TestCase):
    def test_exact_version_and_boundaries(self):
        text = "# Changelog\n\n## [Unreleased]\nFuture\n\n## [0.3.0] - Unreleased\n\n### Fixed\n- Recovery.\n\n## [0.2.0]\nOld\n"
        self.assertEqual(notes.extract(text, "0.3.0"), "### Fixed\n- Recovery.\n")
        self.assertEqual(notes.extract(text, "0.2.0"), "Old\n")

    def test_prerelease_is_not_stable(self):
        text = "## [0.3.0-rc.1]\nPreview\n## [0.3.0]\nStable\n"
        self.assertEqual(notes.extract(text, "0.3.0-rc.1"), "Preview\n")
        self.assertEqual(notes.extract(text, "0.3.0"), "Stable\n")

    def test_fenced_headings_preserve_migration_steps(self):
        for opening, interior, closing in [
            ("```markdown", "## Example heading", "```"),
            ("~~~~markdown", "## [0.3.0]\n~~~\n## More examples", "~~~~"),
            ("  ````", "```\n## [0.2.0]", "  `````"),
        ]:
            body = f"### Migration\n{opening}\n{interior}\n{closing}\n- Required migration step.\n"
            text = f"## [0.3.0] - 2026-09-09\n{body}\n## [0.2.0]\nOld\n"
            with self.subTest(opening=opening):
                self.assertEqual(notes.extract(text, "0.3.0", require_date=True), body)
                self.assertEqual(notes.extract(text.replace("\n", "\r\n"), "0.3.0"), body)
        with self.assertRaisesRegex(ValueError, "unclosed"):
            notes.extract("## [0.3.0]\n```\n## Example\n", "0.3.0")

    def test_release_dates(self):
        for suffix in ["", " - Unreleased", " - Unpublished draft", " - 2026-02-29", " - 2026-13-01", " - 2026-9-09"]:
            text = f"## [0.3.0]{suffix}\n- Changes.\n"
            with self.subTest(suffix=suffix):
                self.assertEqual(notes.extract(text, "0.3.0"), "- Changes.\n")
                with self.assertRaises(ValueError):
                    notes.extract(text, "0.3.0", require_date=True)
        for version in ["0.3.0", "0.3.0-rc.1"]:
            self.assertEqual(notes.extract(f"## [{version}] - 2024-02-29\n- Changes.\n", version, require_date=True), "- Changes.\n")

    def test_cli_release_mode(self):
        script = str(Path(__file__).with_name("release-notes.py").resolve())
        with tempfile.TemporaryDirectory() as folder:
            changelog = Path(folder) / "CHANGELOG.md"
            for status, tag, succeeds in [
                ("Unreleased", None, True),
                ("Unreleased", "v0.3.0", False),
                ("2026-09-09", "v0.3.0", True),
                ("2026-09-09", "v0.2.0", False),
                ("2026-02-30", "v0.3.0", False),
            ]:
                changelog.write_text(f"## [0.3.0] - {status}\n- Changes.\n")
                command = [sys.executable, script, "0.3.0", "--changelog", str(changelog)]
                if tag:
                    command += ["--release-tag", tag]
                result = subprocess.run(command, capture_output=True, text=True)
                with self.subTest(status=status, tag=tag):
                    self.assertEqual(result.returncode == 0, succeeds, result.stderr)
                    self.assertEqual(result.stdout, "- Changes.\n" if succeeds else "")

    def test_missing_duplicate_empty_and_invalid_fail(self):
        for text, version in [
            ("## [0.2.0]\nOld", "0.3.0"),
            ("## [0.3.0]\nA\n## [0.3.0] - Today\nB", "0.3.0"),
            ("## [0.3.0]\n\n### Fixed\n", "0.3.0"),
            ("## [0.3.0]\nValid", "v0.3.0"),
        ]:
            with self.subTest(text=text, version=version), self.assertRaises(ValueError):
                notes.extract(text, version)


if __name__ == "__main__":
    unittest.main()
