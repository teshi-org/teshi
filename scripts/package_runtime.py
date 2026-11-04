"""Resolve shipped Python resources from their live local imports.

Embedded imports the shared session/lease/locator implementation from
browser_agent_broker.py. The Chrome transport oracle lives under resources/tests
and is excluded from shipped runtime resources.
"""

from __future__ import annotations

import ast
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import urllib.request


RESOURCES = Path(__file__).resolve().parents[1] / "resources"


def runtime_resources(entry_points: tuple[str, ...]) -> list[Path]:
    """Include entry points and the transitive imports owned by resources/."""
    pending = list(entry_points)
    result: dict[str, Path] = {}
    while pending:
        name = pending.pop()
        if name in result:
            continue
        if Path(name).name != name or not name.endswith(".py"):
            raise ValueError(f"Runtime entry point must be a resource filename: {name}")
        path = RESOURCES / name
        if not path.is_file():
            raise ValueError(f"Runtime entry point is missing: {name}")
        result[name] = path
        for node in ast.walk(ast.parse(path.read_text(encoding="utf-8"))):
            modules = []
            if isinstance(node, ast.Import):
                modules = [alias.name for alias in node.names]
            elif isinstance(node, ast.ImportFrom) and node.level == 0 and node.module:
                modules = [node.module]
            for module in modules:
                dependency = module.split(".", 1)[0] + ".py"
                if (RESOURCES / dependency).is_file():
                    pending.append(dependency)
    return [result[name] for name in sorted(result)]


def copy_runtime(destination: Path, entry_points: tuple[str, ...]) -> None:
    """Copy the dependency closure without checkout tests, caches, or venvs."""
    sources = runtime_resources(entry_points)
    destination.mkdir(parents=True, exist_ok=True)
    for source in sources:
        shutil.copy2(source, destination / source.name)


def probe_rust_broker(cli: Path) -> None:
    """Start the packaged broker without Python on PATH in private test state.

    An ephemeral listener never touches the user's default port or pairing.
    The Popen process handle owns cleanup, including on failed assertions.
    """
    with tempfile.TemporaryDirectory(prefix="teshi-package-broker-") as directory:
        temp = Path(directory)
        environment = os.environ.copy()
        environment["PATH"] = str(cli.parent)
        environment.pop("PYTHONPATH", None)
        environment.pop("VIRTUAL_ENV", None)
        with (temp / "stdout.log").open("w", encoding="utf-8") as output, (
            temp / "stderr.log"
        ).open("w", encoding="utf-8") as errors:
            process = subprocess.Popen(
                [str(cli), "--browser-broker-internal", "--state-dir", str(temp / "state"),
                 "--discovery-port", "0", "--enable-p0-control", "--enable-p1-observability"],
                cwd=temp, env=environment, stdout=output, stderr=errors,
            )
            try:
                deadline = time.monotonic() + 15
                endpoint = None
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError("Packaged Rust broker exited before readiness")
                    line = (temp / "stdout.log").read_text(encoding="utf-8").strip()
                    if line.startswith("BROWSER_BROKER_READY "):
                        endpoint = json.loads(line.split(" ", 1)[1])
                        break
                    time.sleep(0.02)
                if endpoint is None:
                    raise AssertionError("Packaged Rust broker did not report readiness")
                # Ignore ambient proxies for this explicitly loopback-only probe.
                opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
                with opener.open(endpoint["discovery_url"], timeout=5) as response:
                    discovery = json.load(response)
                if (endpoint["bridge"] != "rust" or discovery["protocol_version"] != 1
                        or discovery["broker_pid"] != process.pid
                        or discovery["broker_start_id"] != endpoint["broker_start_id"]
                        or "p0.control" not in discovery["broker_features"]
                        or "p1.filtered_network_capture" not in discovery["broker_features"]):
                    raise AssertionError("Packaged Rust broker identity/features mismatch")
                if "token=" in json.dumps(discovery) or "project_root" in discovery:
                    raise AssertionError("Public package probe discovery exposes private state")
            finally:
                if process.poll() is None:
                    process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--entry-point", action="append", required=True)
    args = parser.parse_args()
    copy_runtime(args.destination, tuple(args.entry_point))
