#!/usr/bin/env python3
"""Decide whether CI has verified the exact source that Nightly would publish.

A green CI push run for the exact main commit qualifies. So does a green
pull_request run of the merged pull request when its checkout had the same Git
tree: merging creates a new main commit, but the tested files are identical,
so waiting for a second CI run on main would only repeat the verification.
A merge queue verifies the merged tree directly; its run qualifies for the
same reason (`success:merge-group`).

Prints ``success:<evidence>``, ``not-green:<states>`` or ``missing`` and exits
0. Any API or parsing failure exits 1 so the workflow can fail closed.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from typing import Any, Callable

TREE_ARTIFACT_PREFIX = "tested-tree-"

Fetch = Callable[[str], Any]


def tree_artifact_name(tree: str) -> str:
    """Name of the marker artifact CI uploads for the tree it checked out."""
    return f"{TREE_ARTIFACT_PREFIX}{tree}"


def is_green(run: dict) -> bool:
    return run.get("status") == "completed" and run.get("conclusion") == "success"


def same_repo(owner: dict | None, repo: str) -> bool:
    return (owner or {}).get("full_name") == repo


def decide(fetch: Fetch, repo: str, sha: str, tree: str) -> str:
    """Return the CI verdict for main commit ``sha`` whose Git tree is ``tree``."""
    ci_runs = f"repos/{repo}/actions/workflows/ci.yml/runs"
    # A merge-queue run carries the merged commit but belongs to the queue
    # branch, so query by SHA and verify the repository below.
    exact = [
        run
        for run in fetch(f"{ci_runs}?head_sha={sha}&per_page=100")["workflow_runs"]
        if run.get("head_sha") == sha and same_repo(run.get("head_repository"), repo)
    ]
    marker = tree_artifact_name(tree)
    for run in exact:
        # The marker proves the queue checked out the tree the merged commit has.
        if is_green(run) and run.get("event") == "merge_group" and has_marker(fetch, repo, run, marker):
            return f"success:merge-group run {run['id']}"
    if any(is_green(run) and run.get("event") != "merge_group" for run in exact):
        return "success:push"

    for pull in fetch(f"repos/{repo}/commits/{sha}/pulls"):
        # Fork pull_request runs execute the fork's own workflow file, so their
        # markers prove nothing about what this repository's CI verified.
        if pull["base"]["ref"] != "main" or not pull.get("merged_at") or not same_repo(pull["head"]["repo"], repo):
            continue
        head_sha = pull["head"]["sha"]
        for run in fetch(f"{ci_runs}?event=pull_request&head_sha={head_sha}&per_page=100")["workflow_runs"]:
            if not is_green(run) or run.get("head_sha") != head_sha or not same_repo(run.get("head_repository"), repo):
                continue
            # The marker binds the run to its tested merge checkout; a PR run
            # against an older main has a different tree and cannot qualify.
            if has_marker(fetch, repo, run, marker):
                return f"success:pull-request #{pull['number']} run {run['id']}"

    states = sorted({f"{run.get('status')}/{run.get('conclusion')}" for run in exact if not is_green(run)})
    if states:
        return "not-green:" + ",".join(states)
    return "missing"


def has_marker(fetch: Fetch, repo: str, run: dict, marker: str) -> bool:
    """Whether ``run`` uploaded the tested-tree marker for ``marker``."""
    artifacts = fetch(f"repos/{repo}/actions/runs/{run['id']}/artifacts?name={marker}")["artifacts"]
    return any(artifact.get("name") == marker for artifact in artifacts)


def gh_api(path: str) -> Any:
    completed = subprocess.run(
        ["gh", "api", "--header", "Accept: application/vnd.github+json", "--header", "X-GitHub-Api-Version: 2022-11-28", path],
        check=True,
        capture_output=True,
        encoding="utf-8",
    )
    return json.loads(completed.stdout)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True, help="owner/name of the repository")
    parser.add_argument("--sha", required=True, help="full main commit SHA to publish")
    parser.add_argument("--tree", required=True, help="Git tree SHA of that commit")
    args = parser.parse_args(argv)
    try:
        print(decide(gh_api, args.repo, args.sha, args.tree))
    except subprocess.CalledProcessError as error:
        print(f"GitHub API request failed: {error.stderr.strip()}", file=sys.stderr)
        return 1
    except (json.JSONDecodeError, KeyError, TypeError) as error:
        print(f"Unexpected GitHub API response: {error!r}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
