#!/usr/bin/env python3
"""Contract tests for the fail-closed Nightly publication gate."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github" / "workflows" / "nightly.yml").read_text(encoding="utf-8")
RELEASE = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")


def require(needle: str, label: str, text: str = WORKFLOW) -> None:
    if needle not in text:
        raise SystemExit(f"missing {label}: {needle}")


def nightly_decision(ci_runs: list[dict], head_sha: str, nightly_tags: list[str], trigger: str = "schedule") -> str:
    """Mirror the workflow gate for table-driven, offline contract tests."""
    if trigger not in {"schedule", "workflow_dispatch"}:
        return "skip"
    matching = [run for run in ci_runs if run.get("head_sha") == head_sha]
    if not any(run.get("status") == "completed" and run.get("conclusion") == "success" for run in matching):
        return "skip"
    if nightly_tags:
        return "skip"
    return "publish"


def main() -> int:
    require("schedule:", "schedule trigger")
    require('cron: "0 6 * * *"', "daily schedule")
    require("workflow_dispatch:", "manual trigger")
    if "  push:" in WORKFLOW:
        raise SystemExit("Nightly must not have a push trigger")
    require("actions: read", "CI status permission")
    require("actions/workflows/ci.yml/runs?branch=dev&head_sha=${full_sha}", "exact-SHA CI query")
    require("status", "CI run status inspection")
    require("conclusion", "CI run conclusion inspection")
    require("CI is not green", "pending/failure diagnostic")
    require("cancel-in-progress: false", "non-cancelling publication concurrency")
    require("source_ref: ${{ needs.prepare.outputs.source_ref }}", "immutable release source")
    require("git tag --list 'v*-nightly.*' --points-at", "duplicate SHA tag guard")
    if "target_commitish: ${{ needs.resolve.outputs.source_ref }}" not in RELEASE:
        raise SystemExit("release workflow must anchor the GitHub tag to source_ref")
    require("python scripts/update_manifest.py identity", "compiled release identity", RELEASE)

    sha = "a" * 40
    cases = [
        ([{"head_sha": sha, "status": "completed", "conclusion": "success"}], [], "schedule", "publish"),
        ([{"head_sha": sha, "status": "completed", "conclusion": "failure"}], [], "schedule", "skip"),
        ([{"head_sha": sha, "status": "in_progress", "conclusion": None}], [], "schedule", "skip"),
        ([{"head_sha": sha, "status": "completed", "conclusion": "success"}], ["v0.7.10-nightly.20260916.aaaaaaa"], "schedule", "skip"),
        ([{"head_sha": sha, "status": "completed", "conclusion": "success"}], [], "workflow_dispatch", "publish"),
        ([], [], "workflow_dispatch", "skip"),
    ]
    for runs, tags, trigger, expected in cases:
        actual = nightly_decision(runs, sha, tags, trigger)
        if actual != expected:
            raise SystemExit(f"Nightly gate case expected {expected}, got {actual}")

    print("nightly workflow contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
