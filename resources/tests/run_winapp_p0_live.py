"""Owned WinForms/real UIA replay acceptance, with no physical input injection."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--managed-runtime", type=Path, help="Exercise real daemon-owned startup with an isolated runtime")
    args = parser.parse_args()
    root = args.out.resolve()
    root.mkdir(parents=True, exist_ok=False)
    cli = args.cli.resolve()
    report = {"cli_sha256": hashlib.sha256(cli.read_bytes()).hexdigest(), "checks": [], "status": "running"}
    server = fixture = None
    token = secrets.token_urlsafe(24)
    service = Path(__file__).resolve().parents[1] / "winapp_service.py"
    log = (root / "service.log").open("w", encoding="utf-8")
    environment = dict(os.environ)
    if args.managed_runtime:
        environment["TESHI_WINAPP_RUNTIME_DIR"] = str(args.managed_runtime.resolve())
        environment["TESHI_APP_DATA_DIR"] = str(root / "app-data")
        report["managed_runtime"] = str(args.managed_runtime.resolve())

    def call(*arguments, success=True):
        result = subprocess.run([str(cli), *arguments], cwd=root, capture_output=True,
                                text=True, encoding="utf-8", timeout=25, env=environment)
        report["checks"].append({"args": arguments, "exit": result.returncode,
                                 "stdout": result.stdout, "stderr": result.stderr})
        (root / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
        assert (result.returncode == 0) == success, (arguments, result.stdout, result.stderr)
        return result

    def start_service():
        if args.managed_runtime:
            process = subprocess.Popen([str(cli), "--daemon-internal", "--project-root", str(root),
                                        "--port", "0", "--host", "127.0.0.1"],
                                       stdout=log, stderr=log, env=environment,
                                       cwd=root,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
            for _ in range(200):
                if (root / ".teshi/daemon.json").exists():
                    return process
                if process.poll() is not None:
                    raise RuntimeError("Owned daemon exited before readiness")
                time.sleep(0.05)
            process.terminate()
            process.wait(timeout=5)
            raise RuntimeError("Owned daemon readiness timed out")
        process = subprocess.Popen([sys.executable, "-u", str(service), "--port", "0",
                                    "--project-root", str(root), "--auth-token", token],
                                   stdout=subprocess.DEVNULL, stderr=log,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        for _ in range(200):
            if (root / ".teshi/cdp-endpoint.json").exists():
                return process
            if process.poll() is not None:
                raise RuntimeError("Isolated sidecar exited before readiness")
            time.sleep(0.05)
        process.terminate()
        process.wait(timeout=5)
        raise RuntimeError("Isolated sidecar readiness timed out")

    def stop_service():
        if args.managed_runtime:
            manifest = json.loads((root / ".teshi/daemon.json").read_text())
            assert manifest["pid"] == server.pid, "Refuse to stop a daemon not owned by this fixture"
            import urllib.request
            request = urllib.request.Request(f"http://127.0.0.1:{manifest['port']}/api/v1/daemon/shutdown", data=b"", method="POST")
            with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=3):
                pass
        else:
            server.terminate()
        server.wait(timeout=5)

    try:
        fixture = subprocess.Popen(["powershell", "-NoProfile", "-WindowStyle", "Hidden", "-File",
                                    str(Path(__file__).with_name("winapp_shell_fixture.ps1")), "-StateDir", str(root)],
                                   stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
        server = start_service()
        if args.managed_runtime:
            call("winapp", "health")
        endpoint_path = root / ".teshi/cdp-endpoint.json"
        endpoint_text = endpoint_path.read_text(encoding="utf-8")
        endpoint = json.loads(endpoint_text)
        endpoint["ws_url"] = endpoint["ws_url"].split("?", 1)[0] + "?token=invalid-test-credential"
        endpoint_path.write_text(json.dumps(endpoint), encoding="utf-8")
        try:
            denied = call("winapp", "health", success=False)
            assert "authentication_failed" in denied.stdout
        finally:
            endpoint_path.write_text(endpoint_text, encoding="utf-8")
        call("winapp", "wait-window", "--pid", str(fixture.pid), "--visible", "--timeout-ms", "15000")
        call("winapp", "attach", "--pid", str(fixture.pid))
        snapshot = call("winapp", "snapshot")
        assert "Teshi visibility marker" in snapshot.stdout
        selector = "uia:name=Teshi visibility marker"
        call("winapp", "execute", "--selector", selector, "--action", "assert_visible", "--mode", "background")
        source = "Feature: Native replay outcome\n  Scenario: Read marker\n    Then the marker is visible\n"
        (root / "native.feature").write_text(source, encoding="utf-8")
        call("check", "--feature", "native.feature", "--json")

        def bind(action):
            call("steps", "select", "--feature", "native.feature", "--line", "3")
            call("steps", "propose", "--line", "3", "--strategy", "uia", "--value", selector,
                 "--action", action, "--confidence", "1", "--rationale", "Verified owned fixture marker")
            call("steps", "confirm", "--rank", "1")

        bind("assert_visible")
        passed = call("winapp", "replay", "--feature", "native.feature", "--yes", "--mode", "background")
        assert '"status": "passed"' in passed.stdout
        # Failure injection deliberately asserts the opposite of the observed
        # native state; retain this failing run rather than overwriting evidence.
        bind("assert_not_exists")
        failed = call("winapp", "replay", "--feature", "native.feature", "--yes", "--mode", "background", success=False)
        assert '"status": "failed"' in failed.stdout
        health = json.loads(call("winapp", "health").stdout)
        assert health["target_valid"]
        stop_service()
        (root / ".teshi/cdp-endpoint.json").unlink(missing_ok=True)
        server = start_service()
        call("winapp", "execute", "--selector", selector, "--action", "assert_visible", "--mode", "background")
        (root / "command.txt").write_text("quit", encoding="utf-8")
        fixture.wait(timeout=5)
        invalid = call("winapp", "execute", "--selector", selector, "--action", "assert_visible", success=False)
        assert "target_invalid" in invalid.stdout
        report["status"] = "passed"
    except Exception as error:
        report["status"] = "failed"
        report["error"] = str(error)
        raise
    finally:
        if server and server.poll() is None:
            stop_service()
        if fixture and fixture.poll() is None:
            (root / "command.txt").write_text("quit", encoding="utf-8")
            try:
                fixture.wait(timeout=5)
            except subprocess.TimeoutExpired:
                fixture.terminate()
                fixture.wait(timeout=5)
        log.close()
        (root / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"status": report["status"], "checks": len(report["checks"]), "report": str(root / "report.json")}))


if __name__ == "__main__":
    main()
