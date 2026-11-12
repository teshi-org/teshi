"""Offline regressions for the Nightly CI verification gate."""

import io
from pathlib import Path
import subprocess
import sys
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import nightly_ci_gate as gate

REPO = "teshi-org/teshi"
SHA = "c" * 40
TREE = "e" * 40
PR_HEAD = "a" * 40
RUNS = f"repos/{REPO}/actions/workflows/ci.yml/runs"
PUSH_RUNS = f"{RUNS}?branch=main&head_sha={SHA}&per_page=100"
PULLS = f"repos/{REPO}/commits/{SHA}/pulls"
PR_RUNS = f"{RUNS}?event=pull_request&head_sha={PR_HEAD}&per_page=100"
ARTIFACTS = f"repos/{REPO}/actions/runs/7/artifacts?name=tested-tree-{TREE}"


def run(head_sha=SHA, status="completed", conclusion="success", repo=REPO, run_id=7):
    return {"id": run_id, "head_sha": head_sha, "status": status, "conclusion": conclusion, "head_repository": {"full_name": repo}}


def pull(base="main", merged=True, repo=REPO):
    return {
        "number": 36,
        "base": {"ref": base},
        "merged_at": "2026-11-11T08:53:30Z" if merged else None,
        "head": {"sha": PR_HEAD, "repo": {"full_name": repo} if repo else None},
    }


def fake_fetch(responses):
    calls = []

    def fetch(path):
        calls.append(path)
        if path not in responses:
            raise AssertionError(f"unexpected API request: {path}")
        return responses[path]

    return fetch, calls


def pr_responses(pulls=None, pr_runs=None, marker_tree=TREE, push_runs=()):
    return {
        PUSH_RUNS: {"workflow_runs": list(push_runs)},
        PULLS: [pull()] if pulls is None else pulls,
        PR_RUNS: {"workflow_runs": [run(PR_HEAD)] if pr_runs is None else pr_runs},
        ARTIFACTS: {"artifacts": [{"name": f"tested-tree-{marker_tree}"}] if marker_tree else []},
    }


class NightlyCiGateTest(unittest.TestCase):
    def test_green_push_run_for_exact_commit_publishes_without_pr_lookup(self):
        fetch, calls = fake_fetch({PUSH_RUNS: {"workflow_runs": [run()]}})
        self.assertEqual(gate.decide(fetch, REPO, SHA, TREE), "success:push")
        self.assertEqual(calls, [PUSH_RUNS], "exact-SHA success must not query pull requests")

    def test_pr_run_gate_cases(self):
        cases = [
            ("identical tree from merged PR", pr_responses(), "success:pull-request #36 run 7"),
            ("PR tested against an older main", pr_responses(marker_tree="f" * 40), "missing"),
            ("PR run without a marker", pr_responses(marker_tree=None), "missing"),
            ("failed PR run", pr_responses(pr_runs=[run(PR_HEAD, conclusion="failure")]), "missing"),
            ("pending PR run", pr_responses(pr_runs=[run(PR_HEAD, status="in_progress", conclusion=None)]), "missing"),
            ("PR run for another head", pr_responses(pr_runs=[run("b" * 40)]), "missing"),
            ("PR run from a fork repository", pr_responses(pr_runs=[run(PR_HEAD, repo="someone/teshi")]), "missing"),
            ("fork pull request", pr_responses(pulls=[pull(repo="someone/teshi")]), "missing"),
            ("deleted fork pull request", pr_responses(pulls=[pull(repo=None)]), "missing"),
            ("unmerged pull request", pr_responses(pulls=[pull(merged=False)]), "missing"),
            ("pull request into another base", pr_responses(pulls=[pull(base="release")]), "missing"),
            (
                "pending push run with no PR evidence",
                pr_responses(pulls=[], push_runs=[run(status="in_progress", conclusion=None)]),
                "not-green:in_progress/None",
            ),
            (
                "failed push run is rescued by an identical PR tree",
                pr_responses(push_runs=[run(conclusion="failure", run_id=8)]),
                "success:pull-request #36 run 7",
            ),
        ]
        for label, responses, expected in cases:
            with self.subTest(label):
                fetch, _ = fake_fetch(responses)
                self.assertEqual(gate.decide(fetch, REPO, SHA, TREE), expected, label)

    def test_api_failure_fails_closed_with_nonzero_exit(self):
        error = subprocess.CalledProcessError(1, ["gh", "api"], stderr="HTTP 403: Resource not accessible")
        stdout, stderr = io.StringIO(), io.StringIO()
        with patch.object(gate, "gh_api", side_effect=error), redirect_stdout(stdout), redirect_stderr(stderr):
            code = gate.main(["--repo", REPO, "--sha", SHA, "--tree", TREE])
        self.assertEqual(code, 1, "API errors must not print a verdict")
        self.assertEqual(stdout.getvalue(), "", "API errors must not print a verdict")
        self.assertIn("HTTP 403", stderr.getvalue())

    def test_malformed_response_fails_closed_with_nonzero_exit(self):
        with patch.object(gate, "gh_api", return_value={"unexpected": []}), redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(gate.main(["--repo", REPO, "--sha", SHA, "--tree", TREE]), 1)


if __name__ == "__main__":
    unittest.main()
