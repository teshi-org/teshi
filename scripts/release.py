#!/usr/bin/env python3
"""
Release preparation script.

Analyzes commits since the last tag, suggests the next version,
and updates version fields across all component files.

Usage:
    python scripts/release.py                   # dry-run (show what would happen)
    python scripts/release.py --apply           # actually bump files for release
    python scripts/release.py --tag             # bump + commit + tag
    python scripts/release.py --bump-next       # dry-run post-release dev bump
    python scripts/release.py --bump-next --apply  # bump to next dev version for Nightly

Requirements: Python 3.8+, git available on PATH.
"""

import argparse
import re
import subprocess
import sys
from datetime import date
from pathlib import Path

# Optional: validate release.yml YAML syntax (fail early before CI).
try:
    import yaml as _yaml
except ImportError:
    _yaml = None

REPO_ROOT = Path(__file__).resolve().parent.parent

# HARD RULE: teshi is locked to 0.7.x until the lock is intentionally removed.
ALLOWED_MAJOR = 0
ALLOWED_MINOR = 7

VERSION_FILES: list[tuple[str, str, re.Pattern, str]] = [
    # (label, file_path, regex, replacement)
    ("Cargo.toml (root)", "Cargo.toml",
     re.compile(r'^version = "(\d+\.\d+\.\d+)"', re.MULTILINE),
     'version = "{version}"'),
    ("extension/teshi-bridge/manifest.json", "extension/teshi-bridge/manifest.json",
     re.compile(r'"version": "(\d+\.\d+\.\d+)"'),
     '"version": "{version}"'),
    ("browser-testing plugin", "agent-packages/teshi-browser-testing/.codex-plugin/plugin.json",
     re.compile(r'"version": "(\d+\.\d+\.\d+)"'),
     '"version": "{version}"'),
    ("browser-testing compatibility package", "agent-packages/teshi-browser-testing/compatibility.json",
     re.compile(r'"package_version": "(\d+\.\d+\.\d+)"'),
     '"package_version": "{version}"'),
    ("browser-testing compatibility CLI", "agent-packages/teshi-browser-testing/compatibility.json",
     re.compile(r'"teshi_cli": ">=(\d+\.\d+\.\d+) <0\.8\.0"'),
     '"teshi_cli": ">={version} <0.8.0"'),
    ("browser-testing compatibility extension", "agent-packages/teshi-browser-testing/compatibility.json",
     re.compile(r'"extension": ">=(\d+\.\d+\.\d+) <0\.8\.0"'),
     '"extension": ">={version} <0.8.0"'),
    ("playwright-locator CLI compatibility", "agent-packages/teshi-browser-testing/skills/playwright-locator/references/compatibility.md",
     re.compile(r'\| Teshi CLI \| `>=(\d+\.\d+\.\d+) <0\.8\.0` \|'),
     '| Teshi CLI | `>={version} <0.8.0` |'),
    ("playwright-locator extension compatibility", "agent-packages/teshi-browser-testing/skills/playwright-locator/references/compatibility.md",
     re.compile(r'\| teshi-bridge extension \| `>=(\d+\.\d+\.\d+) <0\.8\.0` \|'),
     '| teshi-bridge extension | `>={version} <0.8.0` |'),
]

CHANGELOG_PATH = "CHANGELOG.md"
CHANGELOG_GENERATOR = REPO_ROOT / "scripts" / "changelog_generator.py"

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
    """Return the newest stable ``vX.Y.Z`` tag reachable from HEAD.

    Nightly and other prerelease tags (for example ``v0.7.9-nightly.…``) are
    ignored so changelog and bump analysis cover the last published stable
    release rather than the most recent pre-release tag on ``dev``.
    """
    try:
        tags = git("tag", "--list", "v*", "--sort=-v:refname", "--merged", "HEAD")
    except subprocess.CalledProcessError:
        return None
    stable = re.compile(r"^v\d+\.\d+\.\d+$")
    for tag in tags.splitlines():
        tag = tag.strip()
        if stable.fullmatch(tag):
            return tag
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


def parse_version(version: str) -> tuple[int, int, int]:
    """Parse a strict ``X.Y.Z`` SemVer string into a comparable tuple."""
    major, minor, patch = map(int, version.lstrip("v").split("."))
    return (major, minor, patch)


def bump_version(version: str, bump: str) -> str:
    major, minor, patch = parse_version(version)
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


def determine_release_version(last_tag: str, current: str, bump: str) -> str:
    """Choose the next stable release version without double-bumping.

    When ``main`` has already been bumped ahead of ``last_tag`` (for example
    ``0.7.15`` after ``v0.7.14`` so Nightly publishes ``v0.7.15-nightly.*``),
    the upcoming stable release uses ``current`` directly instead of skipping
    a version number.
    """
    if bump == "none":
        return current
    last_version = last_tag.lstrip("v")
    current_tuple = parse_version(current)
    last_tuple = parse_version(last_version)
    if (
        current_tuple > last_tuple
        and current_tuple[0] == ALLOWED_MAJOR
        and current_tuple[1] == ALLOWED_MINOR
    ):
        return current
    return bump_version(last_version, bump)


def next_dev_version(last_tag: str | None, current: str) -> tuple[str, bool]:
    """Return ``(version, needs_bump)`` for advancing ``main`` past ``last_tag``.

    Nightly builds require ``Cargo.toml`` to be strictly greater than the
    latest stable release tag. When ``current`` is already ahead of
    ``last_tag``, no further bump is needed.
    """
    if last_tag is not None:
        last_version = last_tag.lstrip("v")
        if parse_version(current) > parse_version(last_version):
            return (current, False)
        return (bump_version(last_version, "patch"), True)
    return (bump_version(current, "patch"), True)


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
    if updated and not dry_run:
        # Keep both lockfiles in sync without `cargo generate-lockfile`, which
        # rewrites transitive crates.io pins and creates unrelated churn.
        # The terminal runner is its own workspace; forgetting its lockfile is
        # what failed `--locked` CI after the v0.7.12 / v0.7.13 bumps.
        sync_workspace_lockfile_versions(new_version)
        sync_runner_lockfile_versions(new_version)
    return updated


def _rewrite_package_versions(lock_path: Path, package_names: set[str], new_version: str) -> bool:
    """Rewrite ``version = "…"`` for named ``[[package]]`` blocks in a lockfile.

    Returns True when the file changed.
    """
    if not lock_path.is_file():
        return False
    text = lock_path.read_text(encoding="utf-8")
    lines = text.splitlines(keepends=True)
    out: list[str] = []
    current: str | None = None
    changed = False
    for line in lines:
        name_match = re.match(r'^name = "([^"]+)"\s*$', line)
        if name_match:
            current = name_match.group(1)
            out.append(line)
            continue
        if current in package_names:
            version_match = re.match(r'^version = "(\d+\.\d+\.\d+)"\s*$', line)
            if version_match and version_match.group(1) != new_version:
                indent = line[: line.index("version")]
                out.append(f'{indent}version = "{new_version}"\n')
                changed = True
                current = None
                continue
        out.append(line)
    if changed:
        lock_path.write_text("".join(out), encoding="utf-8")
    return changed


def sync_workspace_lockfile_versions(new_version: str) -> None:
    """Bump path/workspace package versions in the root ``Cargo.lock``."""
    # Only rewrite packages that inherit ``[workspace.package] version``.
    # Leave 0.1.0 / unpublished path crates alone.
    packages = {
        "teshi-agent-runtime",
        "teshi-browser-broker",
        "teshi-cli",
        "teshi-core",
        "teshi-tui",
        "teshi-update",
        "teshi-update-helper",
        "teshi-update-ui",
    }
    if _rewrite_package_versions(REPO_ROOT / "Cargo.lock", packages, new_version):
        print("  ✓ Cargo.lock (workspace package versions)")


def sync_runner_lockfile_versions(new_version: str) -> None:
    """Bump path dependency versions in ``crates/teshi-tui-runner/Cargo.lock``."""
    lock = REPO_ROOT / "crates" / "teshi-tui-runner" / "Cargo.lock"
    # The runner workspace only locks ``teshi-core`` at the workspace version;
    # other path deps stay on their own package versions (e.g. 0.1.0).
    if _rewrite_package_versions(lock, {"teshi-core"}, new_version):
        print("  ✓ crates/teshi-tui-runner/Cargo.lock (teshi-core)")


def validate_release_artifacts() -> bool:
    """Quick pre-flight checks before committing a release tag.

    Catches common issues that would otherwise fail the CI workflow.
    """
    ok = True

    # 1. Validate release.yml and nightly.yml YAML syntax.
    workflows = [
        REPO_ROOT / ".github" / "workflows" / "release.yml",
        REPO_ROOT / ".github" / "workflows" / "nightly.yml",
    ]
    if _yaml is not None:
        for workflow in workflows:
            if not workflow.is_file():
                continue
            try:
                _yaml.safe_load(workflow.read_text(encoding="utf-8"))
            except _yaml.YAMLError as e:
                print(f"  ❌ {workflow.name} has invalid YAML:\n     {e}")
                ok = False
    elif any(workflow.is_file() for workflow in workflows):
        print("  ⚠  PyYAML not installed — skipping workflow YAML syntax check.")

    # 2. Verify Cargo.lock is parseable (minimal check).
    lock = REPO_ROOT / "Cargo.lock"
    if lock.is_file():
        try:
            text = lock.read_text(encoding="utf-8")
            # Ensure the distributable CLI package is present and non-empty.
            if 'name = "teshi-cli"' not in text:
                print("  ❌ Cargo.lock missing teshi-cli package entry.")
                ok = False
        except Exception as e:
            print(f"  ❌ Cargo.lock read error: {e}")
            ok = False

    # 3. Prove the agent package works after being copied outside the checkout.
    smoke = subprocess.run(
        [sys.executable, str(REPO_ROOT / "scripts" / "test-browser-agent-package.py")],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    if smoke.returncode != 0:
        print(f"  ❌ browser-agent package smoke test failed:\n{smoke.stdout}{smoke.stderr}")
        ok = False

    return ok


def git_commit_and_tag(version: str) -> None:
    # Stage all modified version files
    files = [file_path for _, file_path, _, _ in VERSION_FILES]
    git("add", *files)
    # Also stage lockfiles if they changed
    for lock in (
        REPO_ROOT / "Cargo.lock",
        REPO_ROOT / "crates" / "teshi-tui-runner" / "Cargo.lock",
    ):
        if lock.exists():
            git("add", str(lock))
    # Stage CHANGELOG.md if it was updated
    changelog = REPO_ROOT / CHANGELOG_PATH
    if changelog.exists():
        git("add", str(changelog))

    diff = subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=REPO_ROOT)
    if diff.returncode != 0:
        git("commit", "-m", f"chore: release v{version}")
    git("tag", f"v{version}")
    print(f"  ✓ Tagged v{version}")
    print(f"  → To push: git push origin main && git push origin v{version}")
    print("  → After releasing, advance main for Nightly: python scripts/release.py --bump-next --apply")


def update_changelog(version: str, last_tag: str) -> bool:
    """Generate and prepend changelog entry for the new version.

    Delegates to scripts/changelog_generator.py (Keep a Changelog format).
    Returns True if CHANGELOG.md was updated.
    """
    generator = CHANGELOG_GENERATOR
    if not generator.is_file():
        print(f"  ⚠  changelog generator not found at {generator}")
        return False

    # Get commit log since last tag
    log_result = subprocess.run(
        ["git", "log", "--format=%h %s", f"{last_tag}..HEAD"],
        capture_output=True, text=True, cwd=REPO_ROOT,
    )
    if not log_result.stdout.strip():
        print("  ⚠  No commits since last tag — skipping changelog.")
        return False

    # Run changelog generator
    result = subprocess.run(
        [sys.executable, str(generator),
         "--version", version,
         "--base-url", "https://github.com/teshi-org/teshi"],
        input=log_result.stdout,
        capture_output=True, text=True, cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        print(f"  ⚠  changelog generator failed:\n{result.stderr}")
        return False

    new_entry = result.stdout.strip()
    if not new_entry:
        return False

    changelog_path = REPO_ROOT / CHANGELOG_PATH
    existing = changelog_path.read_text(encoding="utf-8") if changelog_path.exists() else ""

    # Keep a Changelog header if file is new
    if not existing.strip():
        existing = (
            "# Changelog\n\n"
            "All notable changes to this project will be documented in this file.\n\n"
        )

    # Prepend new entry before existing content
    content = new_entry + "\n\n" + existing
    changelog_path.write_text(content, encoding="utf-8")
    print(f"  ✓ Updated {CHANGELOG_PATH}")
    return True


def _resolve_current_version() -> str:
    versions = get_current_versions()
    unique_versions = set(v for v in versions.values() if v != "???")
    if len(unique_versions) > 1:
        print("⚠  Version mismatch across files!")
        for label, ver in versions.items():
            print(f"   {label}: {ver}")
        print()
        root_ver = versions.get("Cargo.toml (root)", "")
        if not root_ver or root_ver == "???":
            print("!  Cannot determine authoritative version.")
            sys.exit(1)
        print(f"   Using Cargo.toml version: {root_ver}")
        return root_ver
    return next(v for v in unique_versions)


def _verify_applied_versions() -> None:
    versions = get_current_versions()
    for label, ver in versions.items():
        if ver == "???":
            continue
        parts = ver.split(".")
        if int(parts[0]) != ALLOWED_MAJOR or int(parts[1]) != ALLOWED_MINOR:
            print(f"\n  ❌ {label} version {ver} violates {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x lock!")
            print("     Reverting changes.")
            subprocess.run(
                ["git", "checkout", "--"]
                + [REPO_ROOT / f[1] for f in VERSION_FILES]
                + ([REPO_ROOT / CHANGELOG_PATH] if (REPO_ROOT / CHANGELOG_PATH).exists() else []),
                cwd=REPO_ROOT,
            )
            sys.exit(1)
    print(f"\n  ✅ All files locked to {ALLOWED_MAJOR}.{ALLOWED_MINOR}.x — good.")

    print("\n── Pre-flight CI validation ──")
    if not validate_release_artifacts():
        print("\n  ❌ Pre-flight checks failed. Aborting.")
        print("     Fix the issues above and re-run.")
        sys.exit(1)
    print("  ✅ Pre-flight checks passed.")


def main() -> None:
    parser = argparse.ArgumentParser(description="Prepare a new release")
    parser.add_argument("--apply", action="store_true",
                        help="Apply version bumps (default: dry-run)")
    parser.add_argument("--tag", action="store_true",
                        help="Apply bumps, commit, and tag (implies --apply)")
    parser.add_argument("--bump-next", action="store_true",
                        help="Advance workspace version past the latest stable tag for Nightly builds")
    args = parser.parse_args()

    if args.bump_next and args.tag:
        print("!  --bump-next cannot be combined with --tag.")
        sys.exit(1)

    apply = args.apply or args.tag

    # Phase 1: Analyze
    last_tag = get_last_tag()
    if not last_tag:
        print("!  No previous tag found. Use --version VERSION to set manually.")
        sys.exit(1)

    print(f"Last tag:  {last_tag}")
    current = _resolve_current_version()

    if args.bump_next:
        next_ver, needs_bump = next_dev_version(last_tag, current)
        print(f"Current version:  {current}")
        if not needs_bump:
            print(f"✓ Workspace version {current} is already ahead of {last_tag}; no bump needed.")
            return
        print(f"Next dev version: {next_ver}")
        print("\n── Version file updates ──")
        updated = update_version_files(next_ver, dry_run=not apply)
        for f in updated:
            print(f"  {'✓' if apply else '·'} {f}")
        if apply:
            _verify_applied_versions()
            print()
            print("  Files updated for next Nightly cycle. Review with 'git diff', then commit:")
            print(f"  git commit -m 'chore: bump dev version to {next_ver}'")
        else:
            print()
            print("Dry-run. Run with --bump-next --apply to write changes.")
        return

    commits = get_commits_since(last_tag)

    if not commits:
        print("!  No commits since last tag. Nothing to release.")
        sys.exit(0)

    analysis = analyze_commits(commits)
    bump = recommend_bump(analysis)

    # Phase 2: Determine next release version (without double-bumping pre-bumped dev versions)
    new_version = determine_release_version(last_tag, current, bump)

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
        _verify_applied_versions()

    if not apply:
        print()
        print("Dry-run. Run with --apply to write changes, or --tag to also commit+tag.")
        return

    if args.tag:
        # Generate changelog entry from commits since last tag
        print("\n── Changelog ──")
        if last_tag:
            update_changelog(new_version, last_tag)
        else:
            print("  ⚠  No previous tag — skipping changelog.")

        print()
        git_commit_and_tag(new_version)
    else:
        print()
        print("  Files updated. Review with 'git diff', then commit & tag:")
        print(f"  git add -A && git commit -m 'chore: release v{new_version}'")
        print(f"  git tag v{new_version}")
        print("  # After releasing, advance main for Nightly:")
        print("  python scripts/release.py --bump-next --apply")


if __name__ == "__main__":
    main()
