#!/usr/bin/env python3
"""Prepare once, execute isolated CI shards, and verify their exact inventories."""

import argparse
import os
from pathlib import Path

import ci_suite as suite


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["check", "prepare", "backend", "directory", "terminal-a", "terminal-b", "pack", "unpack", "gate"])
    parser.add_argument("--artifacts", type=Path, default=Path(os.environ.get("RUNNER_TEMP", ".teshi/artifacts")) / "teshi-ci")
    parser.add_argument("--inventory", type=Path)
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    if args.action == "check":
        print(f"All {len(suite.ownership())} test modules belong to exactly one terminal shard.")
        return
    if args.action == "prepare":
        suite.prepare(args.artifacts)
        return
    inventory = suite.read(args.inventory)
    if args.action == "pack":
        suite.write(args.artifacts / "pack-timings.json", suite.pack(inventory, args.archive))
    elif args.action == "unpack":
        suite.write(args.artifacts / "unpack-timings.json", suite.unpack(inventory, args.archive))
    elif args.action == "gate":
        try:
            result = suite.gate(inventory, args.artifacts)
        except Exception as error:
            suite.write(args.artifacts / "platform-gate.json", dict(status="failed", error=str(error)))
            raise
        suite.write(args.artifacts / "platform-gate.json", result)
        print("All required platform tests, expanded Examples and capture files passed.")
        print(f"Manual external-service acceptances not executed: {len(result['manual_acceptance'])}")
    else:
        suite.execute(inventory, args.action, args.artifacts)


if __name__ == "__main__":
    main()
