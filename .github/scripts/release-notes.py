#!/usr/bin/env python3
"""Extract one version's release body from the repository changelog."""
import argparse
from datetime import date
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[2]


def version_headings(text):
    headings = []
    fence = None
    offset = 0
    for line in text.splitlines(keepends=True):
        marker = re.fullmatch(r" {0,3}(`{3,}|~{3,})([^\n]*)\n?", line)
        if fence:
            if (marker and marker[1][0] == fence[0]
                    and len(marker[1]) >= len(fence) and not marker[2].strip()):
                fence = None
        elif marker and (marker[1][0] == "~" or "`" not in marker[2]):
            fence = marker[1]
        else:
            heading = re.fullmatch(r"## (.+?)\s*", line)
            if heading:
                headings.append((heading[1], offset, offset + len(line)))
        offset += len(line)
    if fence:
        raise ValueError("unclosed changelog code fence")
    return headings


def extract(text, version, require_date=False):
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("expected an unprefixed release version")
    text = text.replace("\r\n", "\n")
    headings = version_headings(text)
    matches = [i for i, h in enumerate(headings)
               if re.fullmatch(r"\[" + re.escape(version) + r"\](?: - .+)?", h[0])]
    if len(matches) != 1:
        raise ValueError(f"expected exactly one changelog section for {version}")
    i = matches[0]
    if require_date:
        dated = re.fullmatch(r"\[" + re.escape(version) + r"\] - (\d{4}-\d{2}-\d{2})", headings[i][0])
        if not dated:
            raise ValueError(f"release {version} requires a YYYY-MM-DD date, not a draft status")
        try:
            date.fromisoformat(dated[1])
        except ValueError:
            raise ValueError(f"invalid release date for {version}: {dated[1]}") from None
    end = headings[i + 1][1] if i + 1 < len(headings) else len(text)
    body = text[headings[i][2]:end].strip()
    if not any(line.strip() and not line.lstrip().startswith('#') for line in body.splitlines()):
        raise ValueError(f"empty changelog section for {version}")
    return body + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version", nargs="?")
    parser.add_argument("--changelog", type=Path, default=ROOT / "CHANGELOG.md")
    parser.add_argument("--release-tag", help="require the matching vVERSION tag and a valid release date")
    args = parser.parse_args()
    version = args.version or json.loads((ROOT / "packages/tysel/package.json").read_text())["version"]
    try:
        if args.release_tag is not None and args.release_tag != f"v{version}":
            raise ValueError(f"release tag {args.release_tag} does not match version {version}")
        notes = extract(args.changelog.read_text(encoding="utf-8"), version,
                        require_date=args.release_tag is not None)
    except (OSError, ValueError) as error:
        parser.exit(1, f"release notes: {error}\n")
    sys.stdout.write(notes)


if __name__ == "__main__":
    main()
