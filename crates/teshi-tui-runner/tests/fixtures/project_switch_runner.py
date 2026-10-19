"""Owned runner tree with persistent write evidence for project switching."""

import json
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import time


def main():
    role, project_name, evidence_dir = sys.argv[1:]
    evidence = Path(evidence_dir)
    evidence.mkdir(parents=True, exist_ok=True)
    prefix = evidence / f"{project_name}-{role}"
    # Use the verifier's exact OS probe, so record and inspection compare the
    # same creation-time representation without copying platform-specific code.
    spec = importlib.util.spec_from_file_location(
        "teshi_runner_identity", Path(__file__).resolve().parents[4] / "scripts/run-tui-sidecar-feature.py"
    )
    adapter = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(adapter)
    identity = adapter.process_identity(os.getpid())
    if identity is None or not identity["running"]:
        raise RuntimeError("runner could not record its own live process creation identity")
    prefix.with_suffix(".json").write_text(json.dumps({
        "pid": os.getpid(), "cwd": os.getcwd(), "project": project_name,
        "process_identity": identity,
    }), encoding="utf-8")
    if role == "runner":
        request = json.loads(sys.stdin.readline())
        if project_name == "project-b":
            for case in request["cases"]:
                print(json.dumps({"type": "case_passed", "case_id": case["id"]}), flush=True)
            print(json.dumps({"type": "end_run", "passed": len(request["cases"]),
                              "failed": 0, "skipped": 0}), flush=True)
            return
        # Inherited pipes expose a parent-only kill: the child keeps the runner
        # reader alive and continues writing even after its parent is gone.
        child = subprocess.Popen([sys.executable, __file__, "child", project_name, str(evidence)])
        child_prefix = evidence / f"{project_name}-child"
        ready_deadline = time.monotonic() + 10
        while not all(child_prefix.with_suffix(suffix).exists() for suffix in (".json", ".writes")):
            if child.poll() is not None or time.monotonic() >= ready_deadline:
                raise RuntimeError("runner child did not publish its process and write evidence")
            time.sleep(0.01)
        prefix.with_suffix(".writes").write_text("ready\n", encoding="utf-8")
        # The UI's running indicator precedes actual process startup. Publish
        # readiness only after both processes have verifiable write evidence.
        (evidence / f"{project_name}.ready").write_text("ready\n", encoding="utf-8")
    deadline = time.monotonic() + 45
    counter = 0
    while time.monotonic() < deadline and not (evidence / "cleanup").exists():
        with prefix.with_suffix(".writes").open("a", encoding="utf-8") as stream:
            stream.write(f"{counter}\n")
        counter += 1
        time.sleep(0.025)


if __name__ == "__main__":
    main()
