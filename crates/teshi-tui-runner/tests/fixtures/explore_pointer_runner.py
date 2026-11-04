"""Owned deterministic runner for pointer-to-run/details acceptance."""
import json
import os
from pathlib import Path
import sys

request = json.loads(sys.stdin.readline())
Path(sys.argv[1]).write_text(json.dumps({"request": request, "cwd": os.getcwd()}), encoding="utf-8")
for case in request["cases"]:
    print(json.dumps({"type": "case_failed", "case_id": case["id"], "duration_ms": 1,
                      "error": {"message": "POINTER_FAILURE_EVIDENCE"}}), flush=True)
print(json.dumps({"type": "end_run", "passed": 0, "failed": len(request["cases"]), "skipped": 0}), flush=True)
raise SystemExit(1)
