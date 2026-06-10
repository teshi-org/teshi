#!/usr/bin/env python3
"""
Release preparation script.

Analyzes commits since the last tag, suggests the next version,
and updates version fields across all component files.

Usage:
    python scripts/release.py            # dry-run (show what would happen)
    python scripts/release.py --apply    # actually bump files
    python scripts/release.py --tag      # bump + commit + tag

Requirements: Python 3.8+, git available on PATH.
"""

import argparse
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# HARD RULE: teshi is locked to 0.7.x until the lock is intentionally removed.
ALLOWED_MAJOR = 0
ALLOWED_MINOR = 7

VERSION_FILES: list[tuple[str, str, re.Pattern, str]] = [
    # (label, file_path, regex, replacement)
    ("Cargo.toml (root)", "Cargo.toml",
     re.compile(r'^version = "(\d+\.\d+\.\d+)"', re.MULTILINE),
     'version = "{version}"'),
    ("desktop/src-tauri/Cargo.toml", "desktop/src-tauri/Cargo.toml",
     re.compile(r'^version = "(\d+\.\d+\.\d+)"', re.MULTILINE),
     'version = "{version}"'),
    ("desktop/package.json", "desktop/package.json",
     re.compile(r'"version": "(\d+\.\d+\.\d+)"'),
     '"version": "{version}"'),
    ("desktop/src-tauri/tauri.conf.json", "desktop/src-tauri/tauri.conf.json",
     re.compile(r'"version": "(\d+\.\d+\.\d+)"'),
     '"version": "{version}"'),
    ("extension/teshi-bridge/manifest.json", "extension/teshi-bridge/manifest.json",
     re.compile(r'"version": "(\d+\.\d+\.\d+)"'),
     '"version": "{version}"'),
]

BREAKING_PATTERN = re.compile(r'!\s*:|BREAKING CHANGE', re.IGNORECASE)
FEAT_PATTERN = re.compile(r'^(feat|feature)', re.IGNORECASE)
FIX_PATTERN = re.compile(r'^(fix|perf|security|refactor)', re.IGNORECASE)
IGNORE_PATTERN = re.compile(
    r'^(docs|style|test|chore|ci|build)', re.IGNORECASE,
)


def git(*args: str) -> str:
    result = subprocess.run(
        ["git", *args],
        capture_output=True, text=True, check=True, cwd=REPO_ROOT,
    )
    return result.stdout.strip()


def get_last_tag() -> str | None:
    try:
        return git("describe", "--tags", "--abbrev=0", "--match", "v*")
    except subprocess.CalledProcessError:
        return None


def get_commits_since(tag: str) -> list[dict]:
    log = git("log", f"{tag}..HEAD", "--format=%H|%s")
    commits = []
    for line in log.splitlines():
        if "|" in line:
            h, _, msg = line.partition("|")
            commits.append({"hash": h, "message": msg.strip()})
    return commits


def analyze_commits(commits: list[dict]) -> dict:
    result = {
        "total": len(commits),
        "breaking": [],
        "features": [],
        "fixes": [],
        "ignored": [],
    }
    for c in commits:
        msg = c["message"]
        first_line = msg.split("\n")[0]
        if BREAKING_PATTERN.search(first_line) or "BREAKING CHANGE" in msg:
            result["breaking"].append(c)
        elif FEAT_PATTERN.match(first_line):
            result["features"].append(c)
        elif FIX_PATTERN.match(first_line):
            result["fixes"].append(c)
        elif IGNORE_PATTERN.match(first_line):
            result["ignored"].append(c)
        else:
            # Commits that don't match known patterns — treat as fix
            result["fixes"].append(c)
    return result


def recommend_bump(analysis: dict) -> str:
    if analysis["breaking"]:
        return "major"
    if analysis["features"]:
        return "minor"
    if analysis["fixes"]:
        return "patch"
    return "none"


def bump_version(version: str, bump: str) -> str:
    major, minor, patch = map(int, version.split("."))
    # HARD RULE: version stays locked in ALLOWED_MAJOR.ALLOWED_MINOR.x
    if bump == "major" or bump == "minor":
        print(f"  ⛔ {bump} bump blocked by {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x lock — forcing patch bump instead.")
        bump = "patch"
    if major != ALLOWED_MAJOR or minor != ALLOWED_MINOR:
        print(f"  ⛔ Version {version} outside {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x — forcing to {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x.")
        major, minor = ALLOWED_MAJOR, ALLOWED_MINOR
        patch = 0
    if bump == "patch":
        return f"{major}.{minor}.{patch + 1}"
    return version


def get_current_versions() -> dict[str, str]:
    versions = {}
    for label, file_path, pattern, _ in VERSION_FILES:
        path = REPO_ROOT / file_path
        m = pattern.search(path.read_text(encoding="utf-8"))
        versions[label] = m.group(1) if m else "???"
    return versions


def update_version_files(new_version: str, dry_run: bool = True) -> list[str]:
    updated = []
    for label, file_path, pattern, replacement in VERSION_FILES:
        path = REPO_ROOT / file_path
        text = path.read_text(encoding="utf-8")
        new_text = pattern.sub(replacement.format(version=new_version), text)
        if new_text != text:
            if not dry_run:
                path.write_text(new_text, encoding="utf-8")
            updated.append(label)
    return updated


def git_commit_and_tag(version: str) -> None:
    # Stage all modified version files
    files = [file_path for _, file_path, _, _ in VERSION_FILES]
    git("add", *files)
    # Also stage Cargo.lock if it changed
    lock = REPO_ROOT / "Cargo.lock"
    if lock.exists():
        git("add", str(lock))

    git("commit", "-m", f"chore: bump version to v{version}")
    git("tag", f"v{version}")
    print(f"  ✓ Committed and tagged v{version}")
    print(f"  → To push: git push origin main && git push origin v{version}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Prepare a new release")
    parser.add_argument("--apply", action="store_true",
                        help="Apply version bumps (default: dry-run)")
    parser.add_argument("--tag", action="store_true",
                        help="Apply bumps, commit, and tag (implies --apply)")
    args = parser.parse_args()

    apply = args.apply or args.tag

    # Phase 1: Analyze
    last_tag = get_last_tag()
    if not last_tag:
        print("!  No previous tag found. Use --version VERSION to set manually.")
        sys.exit(1)

    print(f"Last tag:  {last_tag}")
    commits = get_commits_since(last_tag)

    if not commits:
        print("!  No commits since last tag. Nothing to release.")
        sys.exit(0)

    analysis = analyze_commits(commits)
    bump = recommend_bump(analysis)

    # Phase 2: Determine current & next versions
    versions = get_current_versions()
    unique_versions = set(v for v in versions.values() if v != "???")
    if len(unique_versions) > 1:
        print("⚠  Version mismatch across files!")
        for label, ver in versions.items():
            print(f"   {label}: {ver}")
        print()
        # Use the root Cargo.toml version as authoritative
        root_ver = versions.get("Cargo.toml (root)", "")
        if not root_ver or root_ver == "???":
            print("!  Cannot determine authoritative version.")
            sys.exit(1)
        print(f"   Using Cargo.toml version: {root_ver}")
        current = root_ver
    else:
        current = next(v for v in unique_versions)

    new_version = bump_version(current, bump)

    # Phase 3: Summary
    print()
    print(f"Commits since {last_tag}:")
    print(f"  {analysis['total']} total")
    if analysis["breaking"]:
        print(f"  {len(analysis['breaking'])} breaking")
    if analysis["features"]:
        print(f"  {len(analysis['features'])} features")
    if analysis["fixes"]:
        print(f"  {len(analysis['fixes'])} fixes/patches")
    if analysis["ignored"]:
        print(f"  {len(analysis['ignored'])} ignored (docs/chore/...)")
    print()
    print(f"Current version:  {current}")
    print(f"Recommended bump: {bump}")
    print(f"New version:      v{new_version}")

    if bump == "none":
        print("\n!  No functional changes detected. Consider if you really want a release.")
        return

    # Phase 4: Show what commits map to what
    print("\n── Commits by category ──")
    for c in analysis["features"]:
        print(f"  feat   {c['message'][:72]}")
    for c in analysis["fixes"]:
        print(f"  fix    {c['message'][:72]}")
    for c in analysis["breaking"]:
        print(f"  BREAK  {c['message'][:72]}")
    for c in analysis["ignored"]:
        print(f"  ·      {c['message'][:72]}")

    # Phase 5: Update files
    print("\n── Version file updates ──")
    updated = update_version_files(new_version, dry_run=not apply)
    for f in updated:
        print(f"  {'✓' if apply else '·'} {f}")

    if apply:
        # Post-update validation: enforce 0.7.x across all files
        versions = get_current_versions()
        for label, ver in versions.items():
            if ver == "???":
                continue
            parts = ver.split(".")
            if int(parts[0]) != ALLOWED_MAJOR or int(parts[1]) != ALLOWED_MINOR:
                print(f"\n  ❌ {label} version {ver} violates {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x lock!")
                print(f"     Reverting changes.")
                # Revert by restoring from git
                subprocess.run(
                    ["git", "checkout", "--"]
                    + [REPO_ROOT / f[1] for f in VERSION_FILES],
                    cwd=REPO_ROOT,
                )
                sys.exit(1)
        print(f"\n  ✅ All files locked to {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x — good.")

    if not apply:
        print()
        print(f"Dry-run. Run with --apply to write changes, or --tag to also commit+tag.")
        return

    if args.tag:
        print()
        git_commit_and_tag(new_version)
    else:
        print()
        print("  Files updated. Review with 'git diff', then commit & tag:")
        print(f"  git add -A && git commit -m 'chore: bump version to v{new_version}'")
        print(f"  git tag v{new_version}")


if __name__ == "__main__":
    main()
