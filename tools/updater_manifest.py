#!/usr/bin/env python3
"""Write latest.json, the manifest the app's updater reads (DECISIONS D41).

Usage: updater_manifest.py <dist folder> <version> <tag> <owner/repo> <notes.md> > latest.json

The app asks https://github.com/<repo>/releases/latest/download/latest.json, i.e. this
file on the newest *published* (not draft, not pre-release) release. Each platform entry
points at a file attached to that release plus the contents of its minisign `.sig`, made
by `tauri build` with the signing key; the app refuses anything the key didn't sign.

- macOS: one universal `.app.tar.gz` serves both Apple Silicon and Intel.
- Windows: the NSIS installer (the default install, per user); MSI installs get the MSI
  (`windows-x86_64-msi`) so an update doesn't install a second copy.

Exits non-zero if the macOS or NSIS signature is missing.
"""

import datetime
import json
import pathlib
import sys


def main() -> int:
    if len(sys.argv) != 6:
        print(__doc__, file=sys.stderr)
        return 2
    dist, version, tag, repo, notes = sys.argv[1:]
    dist = pathlib.Path(dist)
    base = f"https://github.com/{repo}/releases/download/{tag}"

    def entry(name: str):
        f = dist / name
        sig = dist / (name + ".sig")
        if not f.exists() or not sig.exists():
            return None
        return {"signature": sig.read_text().strip(), "url": f"{base}/{name}"}

    mac = entry(f"LUT-Matrix_{version}_macOS_universal.app.tar.gz")
    nsis = entry(f"LUT-Matrix_{version}_Windows_x64-setup.exe")
    msi = entry(f"LUT-Matrix_{version}_Windows_x64.msi")
    missing = [n for n, e in (("macOS .app.tar.gz", mac), ("Windows setup .exe", nsis)) if e is None]
    if missing:
        print(f"updater_manifest: no signed {', '.join(missing)} in {dist}", file=sys.stderr)
        return 1
    platforms = {
        "darwin-aarch64": mac,
        "darwin-x86_64": mac,
        "windows-x86_64": nsis,
        "windows-x86_64-nsis": nsis,
    }
    if msi:
        platforms["windows-x86_64-msi"] = msi
    manifest = {
        "version": version,
        "notes": pathlib.Path(notes).read_text().strip(),
        "pub_date": datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z"),
        "platforms": platforms,
    }
    json.dump(manifest, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
