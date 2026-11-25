#!/usr/bin/env python3
"""Bump the release version everywhere it lives.

Usage: python scripts/bump_version.py 0.2.0 [--date 2026-12-01]

Updates Cargo.toml, the remoteplayforever entry in Cargo.lock, installer/installer.iss and
turns the "Unreleased" section of CHANGELOG.md into the new release. The release workflow
refuses a tag that does not match Cargo.toml and installer.iss, so run this before tagging.
"""
import argparse
import datetime
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO_URL = "https://github.com/KayZedd/remoteplayforever"
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+$")


def sub_once(pattern: str, repl: str, text: str, what: str, flags: int = 0) -> str:
    new, count = re.subn(pattern, repl, text, count=1, flags=flags)
    if count != 1:
        raise SystemExit(f"could not find {what}")
    return new


def bump_cargo_toml(text: str, version: str) -> str:
    return sub_once(
        r'^version = "[^"]+"', f'version = "{version}"', text, "version in Cargo.toml", re.M
    )


def bump_cargo_lock(text: str, version: str) -> str:
    return sub_once(
        r'(name = "remoteplayforever"\nversion = ")[^"]+(")',
        rf"\g<1>{version}\g<2>",
        text,
        "remoteplayforever in Cargo.lock",
    )


def bump_installer(text: str, version: str) -> str:
    return sub_once(
        r'#define MyAppVersion "[^"]+"',
        f'#define MyAppVersion "{version}"',
        text,
        "MyAppVersion in installer.iss",
    )


def bump_changelog(text: str, version: str, date: str) -> str:
    m = re.search(r"^\[0\.\d+\.\d+\]: .*compare/v([\d.]+)\.\.\.HEAD$|^\[Unreleased\]: .*compare/v([\d.]+)\.\.\.HEAD$", text, re.M)
    if not m:
        raise SystemExit("could not find the [Unreleased] link in CHANGELOG.md")
    previous = m.group(1) or m.group(2)
    if previous == version:
        raise SystemExit(f"CHANGELOG.md already ends at v{version}")
    text = sub_once(
        r"^## \[Unreleased\]\n",
        f"## [Unreleased]\n\n## [{version}] - {date}\n",
        text,
        "the ## [Unreleased] heading",
        re.M,
    )
    text = sub_once(
        r"^\[Unreleased\]: .*$",
        f"[Unreleased]: {REPO_URL}/compare/v{version}...HEAD\n"
        f"[{version}]: {REPO_URL}/compare/v{previous}...v{version}",
        text,
        "the [Unreleased] link",
        re.M,
    )
    return text


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("version", help="new version, X.Y.Z")
    parser.add_argument("--date", default=datetime.date.today().isoformat())
    args = parser.parse_args(argv)
    if not VERSION_RE.match(args.version):
        raise SystemExit("version must look like 1.2.3")

    edits = [
        ("Cargo.toml", lambda t: bump_cargo_toml(t, args.version)),
        ("Cargo.lock", lambda t: bump_cargo_lock(t, args.version)),
        ("installer/installer.iss", lambda t: bump_installer(t, args.version)),
        ("CHANGELOG.md", lambda t: bump_changelog(t, args.version, args.date)),
    ]
    # Compute everything first so a failure leaves no file half-edited.
    results = [(ROOT / name, fn((ROOT / name).read_text(encoding="utf-8"))) for name, fn in edits]
    for path, text in results:
        path.write_text(text, encoding="utf-8", newline="\n")
        print(f"updated {path.relative_to(ROOT)}")
    print(f"Next: review `git diff`, commit, then `git tag v{args.version} && git push origin v{args.version}`.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
