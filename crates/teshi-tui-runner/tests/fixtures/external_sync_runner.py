"""A local runner that holds an actual Feature run until the test releases it."""
import json
from pathlib import Path
import sys
import time

root = Path(sys.argv[1])
request = json.loads(sys.stdin.readline())
(root / "run-request.json").write_text(json.dumps(request), encoding="utf-8")
print(json.dumps({"type": "start_run", "total": len(request["cases"])}), flush=True)
(root / "runner-ready").write_text("ready", encoding="utf-8")
deadline = time.monotonic() + 30
while not (root / "runner-release").exists():
    if time.monotonic() >= deadline:
        raise TimeoutError("test did not release the runner")
    time.sleep(0.02)
for case in request["cases"]:
    print(json.dumps({"type": "case_passed", "case_id": case["id"]}), flush=True)
print(json.dumps({"type": "end_run", "passed": len(request["cases"]), "failed": 0, "skipped": 0}), flush=True)
