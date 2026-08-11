#!/usr/bin/env python3
"""Smoke-test the browser-agent package from an isolated installed layout."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import shutil
import sys
import tempfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PACKAGE_SOURCE = REPO_ROOT / "agent-packages" / "teshi-browser-testing"
EXTENSION_SOURCE = REPO_ROOT / "extension" / "teshi-bridge"
BROKER_SOURCE = REPO_ROOT / "resources" / "browser_agent_broker.py"
BROWSER_SERVICE_SOURCE = REPO_ROOT / "resources" / "browser_service.py"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def cargo_version() -> str:
    text = (REPO_ROOT / "apps" / "teshi-cli" / "Cargo.toml").read_text(
        encoding="utf-8"
    )
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    require(match is not None, "teshi CLI version is missing")
    return match.group(1)


def discover_skill(skill_root: Path) -> tuple[str, str]:
    skill_file = skill_root / "SKILL.md"
    text = skill_file.read_text(encoding="utf-8")
    frontmatter = re.match(r"^---\n(.*?)\n---\n", text, re.DOTALL)
    require(frontmatter is not None, f"invalid skill frontmatter: {skill_file}")
    name = re.search(r"^name:\s*(.+)$", frontmatter.group(1), re.MULTILINE)
    description = re.search(
        r"^description:\s*(.+)$", frontmatter.group(1), re.MULTILINE
    )
    require(name is not None and description is not None, "skill metadata is incomplete")
    require(name.group(1).strip() == skill_root.name, "skill name/folder mismatch")
    require("playwright" in description.group(1).lower(), "skill trigger is not focused")
    return name.group(1).strip(), text


def assert_local_references_resolve(skill_root: Path, skill_text: str) -> None:
    for raw_target in re.findall(r"\[[^]]+\]\(([^)]+)\)", skill_text):
        target = raw_target.split("#", 1)[0]
        if not target or "://" in target:
            continue
        require((skill_root / target).is_file(), f"unresolved skill reference: {target}")


def load_staged_broker(path: Path):
    spec = importlib.util.spec_from_file_location("staged_browser_agent_broker", path)
    require(spec is not None and spec.loader is not None, "cannot load staged broker")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def fake_heartbeat(instance_id: str, label: str) -> dict:
    return {
        "extension_instance_id": instance_id,
        "profile_label": label,
        "extension_version": cargo_version(),
        "protocol_version": 1,
        "browser": {"name": "Chromium", "version": "140", "platform": "Linux"},
        "active_window_id": 7,
        "active_tab_id": 42,
        "url": f"https://{instance_id}.example.test/",
        "title": label,
        "windows": [
            {
                "id": 7,
                "focused": True,
                "tabs": [
                    {
                        "id": 42,
                        "window_id": 7,
                        "title": label,
                        "url": f"https://{instance_id}.example.test/",
                        "active": True,
                        "debuggable": True,
                    }
                ],
            }
        ],
    }


def smoke_test() -> None:
    with tempfile.TemporaryDirectory(prefix="teshi-browser-agent-package-") as temp:
        isolated = Path(temp)
        package_root = isolated / "teshi-browser-testing"
        shutil.copytree(PACKAGE_SOURCE, package_root)
        shutil.copytree(
            EXTENSION_SOURCE,
            package_root / "extension" / "teshi-bridge",
        )
        runtime = package_root / "runtime"
        runtime.mkdir()
        shutil.copy2(BROKER_SOURCE, runtime / BROKER_SOURCE.name)
        shutil.copy2(BROWSER_SERVICE_SOURCE, runtime / BROWSER_SERVICE_SOURCE.name)

        previous_cwd = Path.cwd()
        os.chdir(isolated)
        try:
            manifest = load_json(package_root / ".codex-plugin" / "plugin.json")
            require(manifest["name"] == package_root.name, "plugin folder/name mismatch")
            require((package_root / manifest["skills"]).is_dir(), "plugin skills path missing")
            require(
                (package_root / manifest["mcpServers"]).is_file(),
                "plugin MCP metadata missing",
            )

            skill_root = package_root / "skills" / "playwright-locator"
            skill_name, skill_text = discover_skill(skill_root)
            assert_local_references_resolve(skill_root, skill_text)
            require((skill_root / "agents" / "openai.yaml").is_file(), "UI metadata missing")

            consumer_skill = isolated / "consumer" / ".agents" / "skills" / skill_name
            consumer_skill.parent.mkdir(parents=True)
            shutil.copytree(skill_root, consumer_skill)
            discovered_name, consumer_text = discover_skill(consumer_skill)
            require(discovered_name == skill_name, "vendored skill was not discoverable")
            assert_local_references_resolve(consumer_skill, consumer_text)

            compatibility = load_json(package_root / "compatibility.json")
            extension = load_json(package_root / "extension" / "teshi-bridge" / "manifest.json")
            version = cargo_version()
            require(manifest["version"] == version, "plugin/CLI version drift")
            require(compatibility["package_version"] == version, "compatibility version drift")
            require(extension["version"] == version, "extension/CLI version drift")
            require(compatibility["broker_protocol"] == 1, "broker protocol drift")
            require(compatibility["browser_agent_schema"] == 1, "agent schema drift")

            mcp = load_json(package_root / ".mcp.json")["mcpServers"]
            server = mcp["teshi-browser-agent"]
            require(server["command"] == "teshi", "MCP command must use installed CLI")
            require(
                server["args"] == ["mcp", "serve", "--stdio"],
                "MCP STDIO arguments drifted",
            )

            broker_module = load_staged_broker(runtime / "browser_agent_broker.py")
            broker = broker_module.BrowserSessionBroker()
            record_a = broker.register_heartbeat(fake_heartbeat("profile-a", "Agent A"))
            record_b = broker.register_heartbeat(fake_heartbeat("profile-b", "Agent B"))
            require(broker.heartbeat_response(record_a)["compatible"], "profile A rejected")
            require(broker.heartbeat_response(record_b)["compatible"], "profile B rejected")
            sessions = broker.list_sessions()
            require(len(sessions) == 2, "fake extension sessions were not both discovered")
            lease_a = broker.acquire_lease("profile-a", "smoke-agent-a", 30)
            lease_b = broker.acquire_lease("profile-b", "smoke-agent-b", 30)
            target_a = {
                "extension_instance_id": "profile-a",
                "window_id": 7,
                "tab_id": 42,
            }
            target_b = {
                "extension_instance_id": "profile-b",
                "window_id": 7,
                "tab_id": 42,
            }
            require(broker.resolve_target(target_a)[1] == target_a, "profile A misrouted")
            require(broker.resolve_target(target_b)[1] == target_b, "profile B misrouted")
            broker.release_lease("profile-a", lease_a["lease_token"])
            broker.release_lease("profile-b", lease_b["lease_token"])
        finally:
            os.chdir(previous_cwd)

    print("browser-agent package smoke test passed")


if __name__ == "__main__":
    smoke_test()
