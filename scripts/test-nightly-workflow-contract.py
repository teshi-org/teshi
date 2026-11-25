#!/usr/bin/env python3
"""Contract tests for the fail-closed Nightly publication gate."""

from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = (ROOT / ".github" / "workflows" / "nightly.yml").read_text(encoding="utf-8")
RELEASE = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
CI = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
GATE = (ROOT / "scripts" / "nightly_ci_gate.py").read_text(encoding="utf-8")
VERSION_GATE = (ROOT / "scripts" / "nightly_version_gate.py").read_text(encoding="utf-8")


def require(needle: str, label: str, text: str = WORKFLOW) -> None:
    if needle not in text:
        raise SystemExit(f"missing {label}: {needle}")


def nightly_decision(
    ci_state: str,
    nightly_tags: list[str],
    trigger: str = "schedule",
    *,
    stable_tags_at_head: list[str] | None = None,
    workspace_version: tuple[int, int, int] = (0, 7, 15),
    latest_stable: tuple[int, int, int] = (0, 7, 14),
) -> str:
    """Mirror the workflow gate for table-driven, offline contract tests."""
    if trigger not in {"schedule", "workflow_dispatch"}:
        return "skip"
    if nightly_tags or stable_tags_at_head:
        return "skip"
    if workspace_version <= latest_stable:
        return "error"
    if not ci_state.startswith("success:"):
        return "skip"
    return "publish"


def main() -> int:
    require("schedule:", "schedule trigger")
    require('cron: "0 6 * * *"', "daily schedule")
    require("workflow_dispatch:", "manual trigger")
    if "  push:" in WORKFLOW:
        raise SystemExit("Nightly must not have a push trigger")
    require("actions: read", "CI status permission")
    require("pull-requests: read", "merged pull request lookup permission")
    require(
        'python scripts/nightly_version_gate.py --version "${version}" --head "${full_sha}"',
        "nightly version gate invocation",
    )
    require('[[ "${version_state}" == skip:* ]]', "version gate skip branch")
    require('[[ "${version_state}" != publish:* ]]', "version gate fail-closed branch")
    require('python scripts/nightly_ci_gate.py --repo "${GITHUB_REPOSITORY}" --sha "${full_sha}" --tree "${tree}"', "CI gate invocation")
    require('tree=$(git rev-parse "${full_sha}^{tree}")', "published tree identity")
    require('[[ "${ci_state}" != success:* ]]', "success-only gate")
    require("fail closed", "API-error fail-closed path")
    require("?head_sha={sha}&per_page=100", "exact-SHA CI query", GATE)
    require('run.get("event") == "merge_group"', "merge-queue evidence", GATE)
    require("success:merge-group", "merge-queue verdict", GATE)
    require("commits/{sha}/pulls", "merged pull request lookup", GATE)
    require("artifacts?name={marker}", "tested tree marker lookup", GATE)
    require("git rev-parse 'HEAD^{tree}'", "tested tree record", CI)
    require("name: tested-tree-${{ steps.tested-tree.outputs.tree }}", "tested tree marker upload", CI)
    require("CI is not green", "pending/failure diagnostic")
    require("cancel-in-progress: false", "non-cancelling publication concurrency")
    require("source_ref: ${{ needs.prepare.outputs.source_ref }}", "immutable release source")
    require("NIGHTLY_TAG_RE", "duplicate SHA nightly tag guard", VERSION_GATE)
    require("parsed_workspace <= latest_ver", "strictly-greater-than-stable guard", VERSION_GATE)
    require("Commit already published as stable release", "stable release commit skip guard", VERSION_GATE)
    if "target_commitish: ${{ needs.resolve.outputs.source_ref }}" not in RELEASE:
        raise SystemExit("release workflow must anchor the GitHub tag to source_ref")
    require("python scripts/update_manifest.py identity", "compiled release identity", RELEASE)
    require("TESHI_NIGHTLY_RELEASE_NUMBER", "nightly display number", RELEASE)
    require("Nightly #{1}", "numbered release title", RELEASE)

    cases = [
        ("success:push", [], "schedule", {}, "publish"),
        ("success:pull-request #36 run 1", [], "workflow_dispatch", {}, "publish"),
        ("not-green:completed/failure", [], "schedule", {}, "skip"),
        ("not-green:in_progress/None", [], "schedule", {}, "skip"),
        ("missing", [], "workflow_dispatch", {}, "skip"),
        ("success:push", ["v0.7.15-nightly.20261125.aaaaaaa"], "schedule", {}, "skip"),
        ("success:push", [], "schedule", {"stable_tags_at_head": ["v0.7.14"], "workspace_version": (0, 7, 14)}, "skip"),
        ("success:push", [], "schedule", {"workspace_version": (0, 7, 14), "latest_stable": (0, 7, 14)}, "error"),
        ("success:push", [], "schedule", {"workspace_version": (0, 7, 13), "latest_stable": (0, 7, 14)}, "error"),
        ("success:push", [], "push", {}, "skip"),
    ]
    for ci_state, tags, trigger, extra, expected in cases:
        actual = nightly_decision(ci_state, tags, trigger, **extra)
        if actual != expected:
            raise SystemExit(
                f"Nightly gate case {(ci_state, tags, trigger, extra)!r} expected {expected}, got {actual}"
            )

    print("nightly workflow contract passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
