#!/usr/bin/env python3
"""Compare complete same-SHA Windows temporary-directory experiments."""

import argparse
import json
from pathlib import Path
import statistics


def compare(root):
    environments = {mode: json.loads((root / mode / "environment/temp-environment.json").read_text(encoding="utf-8-sig"))
                    for mode in ("default", "runner")}
    shas = {env["source_sha"] for env in environments.values()}
    if len(shas) != 1 or not next(iter(shas)):
        raise ValueError("experiment commits differ")
    images = {env["runner"]["image_version"] for env in environments.values()}
    if len(images) != 1 or not next(iter(images)):
        raise ValueError("runner image versions differ")
    lines = [f"Source: `{next(iter(shas))}`", "",
             "| Workload | Default median | RUNNER_TEMP median | Default / RUNNER_TEMP |",
             "|---|---:|---:|---:|"]
    for workload in ("directory", "management", "terminal"):
        samples = {}
        for mode in environments:
            samples[mode] = []
            for repeat in range(1, 4):
                result = json.loads((root / mode / f"repeat-{repeat}/{workload}/timings.json").read_text())
                if result["timed_out"] or result["returncode"] != 0 or result["cleanup_error"]:
                    raise ValueError(f"{mode}/{repeat}/{workload} did not pass completely")
                samples[mode].append(result["elapsed_seconds"])
        baseline, candidate = (statistics.median(samples[mode]) for mode in ("default", "runner"))
        lines.append(f"| {workload} | {baseline:.3f}s | {candidate:.3f}s | {baseline / candidate:.3f}x |")
    lines.extend(["", "Three sequential repeats per mode on separate hosted runners. Build and cache steps are outside these timings.",
                  "Runner variation and cold/warm effects remain; inspect individual samples and phase timings before adopting the setting."])
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = compare(args.root)
    args.output.write_text(report, encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
