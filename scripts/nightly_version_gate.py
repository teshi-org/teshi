#!/usr/bin/env python3
"""Enforce SemVer ordering and tag guards before publishing a Nightly build.

Nightly builds are pre-releases of the *next* unreleased stable version
(``v0.7.14 < v0.7.15-nightly.YYYYMMDD.sha < v0.7.15``). Publishing a nightly
whose base SemVer is less than or equal to an already-published stable tag
violates SemVer ordering and updater channel-switch guarantees.

Exit codes and stdout contract:
- Exit 0 with ``publish:<reason>`` when the commit is eligible for Nightly.
- Exit 0 with ``skip:<reason>`` when the commit already has a nightly or
  stable release tag (expected no-op on unchanged ``main``).
- Exit 1 with a diagnostic on stderr when ``workspace_version`` is invalid or
  not strictly greater than the latest stable ``vX.Y.Z`` tag (fail closed).
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from typing import Iterable

SEMVER_RE = re.compile(r"^(\d+)\.(\d+)\.(\d+)$")
STABLE_TAG_RE = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
NIGHTLY_TAG_RE = re.compile(r"^v\d+\.\d+\.\d+-nightly\.")


def parse_semver(value: str) -> tuple[int, int, int] | None:
    """Parse a strict ``X.Y.Z`` version string."""
    match = SEMVER_RE.fullmatch(value.strip())
    if not match:
        return None
    return tuple(int(part) for part in match.groups())


def parse_stable_tag(tag: str) -> tuple[int, int, int] | None:
    """Parse a stable ``vX.Y.Z`` release tag, ignoring prereleases and legacy 4-part tags."""
    match = STABLE_TAG_RE.fullmatch(tag.strip())
    if not match:
        return None
    return tuple(int(part) for part in match.groups())


def latest_stable_tag(tags: Iterable[str]) -> tuple[tuple[int, int, int], str] | None:
    """Return ``((major, minor, patch), tag)`` for the highest stable ``vX.Y.Z`` tag."""
    best: tuple[tuple[int, int, int], str] | None = None
    for raw in tags:
        tag = raw.strip()
        parsed = parse_stable_tag(tag)
        if parsed is None:
            continue
        if best is None or parsed > best[0]:
            best = (parsed, tag)
    return best


def evaluate(
    workspace_version: str,
    all_tags: Iterable[str],
    head_tags: Iterable[str],
) -> tuple[str, str]:
    """Return ``(decision, detail)`` where decision is ``publish``, ``skip``, or ``error``."""
    parsed_workspace = parse_semver(workspace_version)
    if parsed_workspace is None:
        return ("error", f"Invalid workspace version {workspace_version!r}; expected X.Y.Z.")

    head_list = [tag.strip() for tag in head_tags if tag.strip()]

    existing_nightly = sorted(tag for tag in head_list if NIGHTLY_TAG_RE.match(tag))
    if existing_nightly:
        joined = " ".join(existing_nightly)
        return ("skip", f"Commit already published as nightly ({joined}); skipping.")

    existing_stable = sorted(tag for tag in head_list if parse_stable_tag(tag) is not None)
    if existing_stable:
        joined = " ".join(existing_stable)
        return ("skip", f"Commit already published as stable release ({joined}); skipping.")

    latest = latest_stable_tag(all_tags)
    if latest is not None:
        latest_ver, latest_tag = latest
        if parsed_workspace <= latest_ver:
            next_patch = f"{latest_ver[0]}.{latest_ver[1]}.{latest_ver[2] + 1}"
            return (
                "error",
                f"Workspace version {workspace_version} must be greater than latest stable "
                f"release {latest_tag}; bump Cargo.toml to the next version (for example "
                f"{next_patch}) before publishing Nightly.",
            )
        return (
            "publish",
            f"Workspace version {workspace_version} is ahead of latest stable release {latest_tag}.",
        )

    return ("publish", f"Workspace version {workspace_version} has no prior stable release tags.")


def _git_lines(*args: str) -> list[str]:
    completed = subprocess.run(
        ["git", *args],
        check=True,
        capture_output=True,
        encoding="utf-8",
    )
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--version", required=True, help="workspace SemVer from Cargo.toml")
    parser.add_argument("--head", required=True, help="full commit SHA being considered for Nightly")
    args = parser.parse_args(argv)
    try:
        all_tags = _git_lines("tag", "--list", "v*")
        head_tags = _git_lines("tag", "--list", "v*", "--points-at", args.head)
    except subprocess.CalledProcessError as error:
        print(f"git tag lookup failed: {error.stderr.strip()}", file=sys.stderr)
        return 1

    decision, detail = evaluate(args.version, all_tags, head_tags)
    if decision == "error":
        print(detail, file=sys.stderr)
        return 1
    print(f"{decision}:{detail}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
