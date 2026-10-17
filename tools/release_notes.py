#!/usr/bin/env python3
"""Print release notes for a version from CHANGELOG.md, or check release readiness.

    python3 tools/release_notes.py              # notes for the version in Cargo.toml
    python3 tools/release_notes.py 0.2.0        # notes for 0.2.0
    python3 tools/release_notes.py --version    # just print the Cargo.toml version

Exits non-zero if CHANGELOG.md has no non-empty "## [<version>]" section, so the release
workflow stops before building anything for a version that hasn't been written up.
Standard library only (Python 3.11+ for tomllib).
"""
import re
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def cargo_version() -> str:
    with open(ROOT / "Cargo.toml", "rb") as f:
        return tomllib.load(f)["workspace"]["package"]["version"]


def notes(version: str) -> str:
    text = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    head = re.compile(r"^## \[" + re.escape(version) + r"\][^\n]*\n", re.M)
    m = head.search(text)
    if not m:
        sys.exit(f"CHANGELOG.md has no '## [{version}]' section")
    nxt = re.compile(r"^## \[", re.M).search(text, m.end())
    body = text[m.end(): nxt.start() if nxt else len(text)].strip()
    # Drop link-reference definitions ("[0.1.0]: https://...") that may follow the last section.
    body = re.sub(r"^\[[^\]]+\]: \S+\s*$", "", body, flags=re.M).strip()
    if not body:
        sys.exit(f"CHANGELOG.md section [{version}] is empty")
    return body + "\n"


def main() -> None:
    args = sys.argv[1:]
    if args == ["--version"]:
        print(cargo_version())
        return
    version = args[0] if args else cargo_version()
    sys.stdout.write(notes(version.lstrip("v")))


if __name__ == "__main__":
    main()
