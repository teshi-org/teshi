#!/usr/bin/env python3
"""Execute every product Feature and persist Feature/Scenario evidence.

The matrix is deliberately fail-closed: a missing binding is a failed Feature,
while an unavailable external target (browser or WinApp) is recorded as
blocked. Neither state is converted into a pass.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
FEATURES = ROOT / "features"
RUNNER_TAGS = {"@api", "@cli", "@validation-e2e", "@web-ui", "@winapp", "@tui"}


def tags(source: str) -> set[str]:
    return {
        token
        for line in source.splitlines()
        if line.strip().startswith("@")
        for token in line.split()
        if token.startswith("@")
    }


def scenarios(source: str) -> list[dict[str, object]]:
    """Return the scenario identities used by the stable Teshi case IDs."""
    result: list[dict[str, object]] = []
    pattern = re.compile(
        r"^\s*(?:Scenario(?: Outline)?|场景(?:大纲)?):\s*(.+?)\s*$"
    )
    for line_number, line in enumerate(source.splitlines(), 1):
        match = pattern.match(line)
        if match:
            result.append(
                {
                    "case_id": f"f0:s{len(result)}",
                    "scenario": match.group(1),
                    "line": line_number,
                }
            )
    return result


def safe_stem(path: Path) -> str:
    value = path.relative_to(ROOT).with_suffix("").as_posix()
    # Keep Unicode so same-stem localized Features do not overwrite one another.
    # Only characters that are invalid in a Windows filename are normalized.
    return re.sub(r'[<>:"/\\|?*]+', "_", value).strip(" ._")


def sibling(teshi: Path, name: str) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    candidate = teshi.with_name(name + suffix)
    return candidate


def resolve_runner(teshi: Path, name: str, default: str | None = None) -> Path:
    override = os.environ.get(name)
    if override:
        return Path(override).resolve()
    return sibling(
        teshi,
        default or name.removeprefix("TESHI_RUNNER_").lower().replace("_", "-"),
    )


def tui_runner(teshi: Path, feature: Path) -> tuple[str | None, str]:
    name = feature.name
    if name in {"tui_acp_model_persistence.feature", "ACP模型选择持久化.feature"}:
        return str(resolve_runner(teshi, "TESHI_RUNNER_ACP_MODEL_FEATURE", "teshi-acp-model-feature-runner")), "confirmed-semantic"
    if name in {"tui_acp_authoring.feature", "ACP功能生成.feature"}:
        return str(resolve_runner(teshi, "TESHI_RUNNER_ACP_AUTHORING_FEATURE", "teshi-acp-authoring-feature-runner")), "confirmed-semantic"
    if name in {
        "tui_agent_activity.feature",
        "Agent运行过程展示.feature",
    }:
        return str(resolve_runner(teshi, "TESHI_RUNNER_AGENT_FEATURE", "teshi-agent-feature-runner")), "confirmed-semantic"
    if name in {
        "tui_conversation_history.feature",
        "tui_agent_selection_memory.feature",
        "Agent对话持久化.feature",
        "记住上次选择的Agent.feature",
    }:
        return str(resolve_runner(teshi, "TESHI_RUNNER_AGENT_FEATURE", "teshi-agent-feature-runner")), "confirmed-semantic"
    # The generic runner is still an executable binding: it will fail closed
    # when the product Feature has no source-bound .tui.json sidecar.
    return sys.executable, "generated-tui"


def base_report(feature: Path, route: str, binding: str, status: str, reason: str) -> dict:
    source = feature.read_text(encoding="utf-8")
    cases = [
        {
            "case_id": item["case_id"],
            "feature_path": feature.relative_to(ROOT).as_posix(),
            "scenario": item["scenario"],
            "status": status,
            "duration_ms": None,
            "error": reason,
            "steps": [],
            "attachments": [],
        }
        for item in scenarios(source)
    ]
    return {
        "schema_version": 1,
        "feature": feature.relative_to(ROOT).as_posix(),
        "runner": route,
        "status": status,
        "binding": {"status": binding, "route": route},
        "started_at_unix_ms": int(time.time() * 1000),
        "finished_at_unix_ms": int(time.time() * 1000),
        "summary": {
            "total": len(cases),
            "passed": len(cases) if status == "passed" else 0,
            "failed": len(cases) if status == "failed" else 0,
            "skipped": 0,
            "blocked": len(cases) if status == "blocked" else 0,
        },
        "cases": cases,
        "runner_error": reason,
        "runner_exit": None,
    }


def enrich_report(path: Path, feature: Path, route: str, binding: str) -> dict:
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return base_report(feature, route, binding, "failed", "runner did not produce a valid report")
    report["binding"] = {"status": binding, "route": route}
    report["feature"] = feature.relative_to(ROOT).as_posix()
    artifact_root = path.parent / "artifacts" / safe_stem(feature)
    for case in report.get("cases", []):
        case_dir = artifact_root / re.sub(r"[^A-Za-z0-9_-]", "_", case["case_id"])
        case["attachments"] = case.get("attachments", []) + [
            {"kind": "file", "path": str(artifact)}
            for artifact in sorted(case_dir.rglob("*")) if artifact.is_file()
        ]
    path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return report


def is_blocked_output(text: str) -> bool:
    lowered = text.lower()
    return any(
        marker in lowered
        for marker in (
            "no eligible browser",
            "no browser session",
            "browser target unavailable",
            "ready profile",
            "implicit browser targeting",
            "cdp endpoint",
            "no winapp",
            "winapp sidecar",
            "winapp service did not respond",
            "no winui3 window",
            "no attached",
            "attached window",
            "windows-only",
            "not supported on this platform",
        )
    )


def run_one(
    teshi: Path,
    feature: Path,
    output_dir: Path,
    verbose: bool,
    scenario_filter: str | None = None,
) -> dict:
    source = feature.read_text(encoding="utf-8")
    feature_tags = tags(source)
    owner = sorted(feature_tags & RUNNER_TAGS)
    if len(owner) != 1 and not ("@cli" in feature_tags and "@validation-e2e" in feature_tags):
        report = base_report(feature, "catalog", "invalid-route", "failed", "Feature must have exactly one execution owner")
        report_path = output_dir / f"{safe_stem(feature)}.json"
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return report

    report_path = output_dir / f"{safe_stem(feature)}.json"
    log_path = output_dir / f"{safe_stem(feature)}.log"
    runner_args: list[str] = []
    if "@validation-e2e" in feature_tags:
        route = "validation-cli"
        runner = resolve_runner(teshi, "TESHI_RUNNER_VALIDATION_CLI", "teshi-validation-cli-runner")
        binding = "confirmed-semantic"
    elif "@cli" in feature_tags:
        route = "requirement-cli"
        runner = resolve_runner(teshi, "TESHI_RUNNER_REQUIREMENT_CLI", "teshi-requirement-cli-runner")
        binding = "confirmed-semantic"
    elif "@tui" in feature_tags:
        runner_text, binding = tui_runner(teshi, feature)
        route = "tui"
        runner = Path(runner_text) if runner_text else None
        runner_args = (
            [str(ROOT / "scripts" / "run-tui-sidecar-feature.py")]
            if binding == "generated-tui"
            else []
        )
    elif "@web-ui" in feature_tags:
        route = "browser-replay"
        binding = "confirmed-sidecar"
        command = [
            str(teshi),
            "browser",
            "replay",
            "--feature",
            feature.relative_to(ROOT).as_posix(),
            "--non-interactive",
        ]
        return run_browser_cases(feature, command, output_dir, report_path, log_path, verbose, scenario_filter)
    elif "@winapp" in feature_tags:
        route = "winapp-replay"
        binding = "confirmed-sidecar"
        command = [
            str(teshi),
            "winapp",
            "replay",
            "--feature",
            feature.relative_to(ROOT).as_posix(),
            "--yes",
        ]
        return run_external_target(feature, route, binding, command, output_dir, report_path, log_path, verbose)
    else:
        report = base_report(feature, "catalog", "invalid-route", "failed", "no executable route")
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return report

    if runner is None or not runner.is_file():
        report = base_report(feature, route, binding, "failed", f"runner binary is missing: {runner}")
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        return report

    command = [
        str(teshi),
        "run",
        str(feature),
        "--runner-cmd",
        str(runner),
        "--report",
        str(report_path),
    ]
    for runner_arg in runner_args:
        command.extend(["--runner-arg", runner_arg])
    if scenario_filter:
        command.extend(["--scenario", scenario_filter])
    return run_external_target(feature, route, binding, command, output_dir, report_path, log_path, verbose)


def run_browser_cases(feature, command, output_dir, report_path, log_path, verbose, scenario_filter):
    """Replay each scenario with its own background and retain actual outcomes."""
    report = base_report(feature, "browser-replay", "confirmed-sidecar", "failed", "not executed")
    # Diagnose the explicitly configured local GPUI test page without retaining
    # its bearer-token URL. A missing adapter prevents every canvas assertion.
    if os.environ.get("TESHI_BROWSER_PROJECT"):
        probe = subprocess.run(
            [command[0], "browser", "snapshot"], cwd=os.environ["TESHI_BROWSER_PROJECT"],
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=30,
        )
        try:
            snapshot = json.loads(probe.stdout)
        except json.JSONDecodeError:
            snapshot = {}
        gpu_errors = snapshot.get("gpu_errors", [])
        if "teshi" in snapshot.get("title", "").lower() and any("webgpu found no adapters" in error.lower() for error in gpu_errors):
            report = base_report(feature, "browser-replay", "confirmed-sidecar", "blocked", "WebGPU adapter unavailable; GPUI window could not initialize")
            evidence = {key:snapshot.get(key) for key in ("title", "accessibility_tree", "gpu_errors")}
            log_path.write_text(json.dumps(evidence, ensure_ascii=False, indent=2), encoding="utf-8")
            for case in report["cases"]:
                case["attachments"] = [{"kind":"file", "path":str(log_path)}]
            report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
            return report
    if scenario_filter:
        report["cases"] = [case for case in report["cases"] if case["scenario"] == scenario_filter]
        if not report["cases"]:
            raise ValueError(f"scenario not found: {scenario_filter}")
    logs = []
    for case in report["cases"]:
        started = time.monotonic()
        try:
            completed = subprocess.run(
                command + ["--scenario", case["scenario"]],
                cwd=os.environ.get("TESHI_BROWSER_PROJECT", str(ROOT)),
                env=os.environ,
                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=180,
            )
            combined = completed.stdout + "\n" + completed.stderr
            passed = completed.returncode == 0 and '"event":"replay_complete"' in combined
            case["status"] = "passed" if passed else ("blocked" if is_blocked_output(combined) else "failed")
            case["error"] = None if passed else combined[-4000:]
        except (OSError, subprocess.TimeoutExpired) as error:
            combined = str(error)
            case["status"] = "failed"
            case["error"] = combined
        case["duration_ms"] = int((time.monotonic() - started) * 1000)
        case_log = output_dir / (safe_stem(feature) + "-" + case["case_id"].replace(":", "_") + ".log")
        case_log.write_text(combined, encoding="utf-8")
        case["attachments"] = [{"kind":"file", "path":str(case_log)}]
        logs.append(combined)
        if verbose:
            print(combined, flush=True)
    report["summary"] = {"total":len(report["cases"]), **{
        state:sum(case["status"] == state for case in report["cases"])
        for state in ("passed", "failed", "blocked", "skipped")
    }}
    report["status"] = "failed" if report["summary"]["failed"] else ("blocked" if report["summary"]["blocked"] else "passed")
    report["finished_at_unix_ms"] = int(time.time() * 1000)
    report["runner_error"] = None
    log_path.write_text("\n".join(logs), encoding="utf-8")
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return report


def run_external_target(
    feature: Path,
    route: str,
    binding: str,
    command: list[str],
    output_dir: Path,
    report_path: Path,
    log_path: Path,
    verbose: bool,
) -> dict:
    started = time.time()
    # Never let a previous invocation masquerade as the current run's result.
    # The matrix reports are generated artifacts and are always rebuilt here.
    report_path.unlink(missing_ok=True)
    log_path.unlink(missing_ok=True)
    try:
        runner_env = {
            **os.environ,
            "TESHI_BIN": command[0],
            "TESHI_SELF_TEST_BIN": command[0],
            "TESHI_FEATURE_TEMP": os.environ.get("TESHI_FEATURE_TEMP", str(output_dir)),
            "TESHI_FEATURE_ARTIFACTS": str(output_dir / "artifacts" / safe_stem(feature)),
        }
        generic_runner = Path(command[0]).with_name(
            "teshi-tui-runner" + (".exe" if os.name == "nt" else "")
        )
        if generic_runner.is_file():
            runner_env["TESHI_GENERIC_TUI_RUNNER"] = str(generic_runner)
        completed = subprocess.run(
            command,
            cwd=os.environ.get("TESHI_BROWSER_PROJECT", str(ROOT)) if route == "browser-replay" else ROOT,
            env=runner_env,
            text=True,
            encoding="utf-8",
            errors="replace",
            capture_output=True,
            timeout=900,
        )
        combined = completed.stdout + "\n" + completed.stderr
        log_path.write_text(combined, encoding="utf-8")
    except (OSError, subprocess.TimeoutExpired) as error:
        completed = None
        combined = str(error)
        log_path.write_text(combined, encoding="utf-8")

    if report_path.is_file():
        report = enrich_report(report_path, feature, route, binding)
        report["duration_ms"] = int((time.time() - started) * 1000)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    else:
        blocked = is_blocked_output(combined)
        native_complete = (
            route == "winapp-replay" and completed is not None and completed.returncode == 0
            and '"event":"replay_complete"' in combined and len(scenarios(feature.read_text(encoding="utf-8"))) == 1
        )
        status = "passed" if native_complete else ("blocked" if blocked else "failed")
        reason = None if native_complete else ("external target unavailable" if blocked else "runner exited without producing a report")
        report = base_report(feature, route, binding, status, reason)
        report["runner_error"] = combined[-4000:]
        report["runner_exit"] = {"code": completed.returncode, "success": completed.returncode == 0} if completed else None
        report["duration_ms"] = int((time.time() - started) * 1000)
        report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    if verbose and combined:
        print(combined, end="" if combined.endswith("\n") else "\n")
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--teshi", type=Path, default=None, help="teshi CLI binary")
    parser.add_argument("--output-dir", type=Path, default=None, help="new result directory (defaults to a unique run directory)")
    parser.add_argument("--feature", action="append", type=Path, help="run only this Feature (repeatable)")
    parser.add_argument("--verbose", action="store_true")
    parser.add_argument("--scenario", default=None, help="run only this scenario in every selected Feature")
    args = parser.parse_args()

    teshi = (args.teshi or Path(os.environ.get("TESHI_BIN", ""))).resolve()
    if not teshi.is_file():
        candidates = [ROOT / "target" / "debug" / ("teshi.exe" if os.name == "nt" else "teshi")]
        teshi = next((candidate for candidate in candidates if candidate.is_file()), teshi)
    if not teshi.is_file():
        parser.error("teshi binary not found; pass --teshi or set TESHI_BIN")

    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ") + f"-{os.getpid()}"
    output_dir = (args.output_dir or ROOT / ".teshi" / "feature-results" / "runs" / run_id).resolve()
    if output_dir.exists() and any(output_dir.iterdir()):
        parser.error("result directory is not empty; use a new directory to preserve previous evidence")
    output_dir.mkdir(parents=True, exist_ok=True)
    feature_paths = args.feature or sorted(FEATURES.rglob("*.feature"))
    selected_features = [
        (raw_path if raw_path.is_absolute() else (ROOT / raw_path)).resolve()
        for raw_path in feature_paths
    ]
    for feature in selected_features:
        if not feature.is_file():
            parser.error(f"missing Feature: {feature}")
        if args.scenario and not any(item["scenario"] == args.scenario for item in scenarios(feature.read_text(encoding="utf-8"))):
            parser.error(f"scenario not found in {feature}: {args.scenario}")
    reports = []
    for raw_path in feature_paths:
        feature = raw_path if raw_path.is_absolute() else (ROOT / raw_path)
        feature = feature.resolve()
        if not feature.is_file():
            print(f"missing Feature: {feature}", file=sys.stderr)
            continue
        report = run_one(teshi, feature, output_dir, args.verbose, args.scenario)
        reports.append(report)
        summary = report.get("summary", {})
        print(
            f"{report.get('status', 'failed'):7} {feature.relative_to(ROOT)} "
            f"passed={summary.get('passed', 0)} failed={summary.get('failed', 0)} "
            f"blocked={summary.get('blocked', 0)}"
        )

    aggregate = {
        "schema_version": 1,
        "run_id": run_id,
        "teshi": str(teshi),
        "generated_at_unix_ms": int(time.time() * 1000),
        "feature_count": len(reports),
        "features": reports,
        "summary": {
            "passed": sum(report.get("status") == "passed" for report in reports),
            "failed": sum(report.get("status") == "failed" for report in reports),
            "blocked": sum(report.get("status") == "blocked" for report in reports),
        },
    }
    aggregate_path = output_dir / "matrix.json"
    aggregate_path.write_text(json.dumps(aggregate, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"result report: {aggregate_path}")
    return 1 if aggregate["summary"]["failed"] or aggregate["summary"]["blocked"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
