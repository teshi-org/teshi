#!/usr/bin/env python3
"""One-command teshi web UI self-bootstrap with a live status dashboard.

Starts the full dev stack (teshi-desktop + teshi web + Vite + optional embedded
sidecar), then refreshes a Rich table with version, health, and duplicate-instance
warnings for each component.

Usage:
    pip install -r scripts/requirements-dev.txt
    python scripts/bootstrap_dev.py --project .
    py scripts/bootstrap_dev.py .
"""

from __future__ import annotations

import argparse
import atexit
import json
import os
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, field
from datetime import datetime
from enum import Enum
from pathlib import Path
from typing import Any

import httpx
import psutil
from rich.console import Console, Group
from rich.live import Live
from rich.table import Table
from rich.text import Text

REPO_ROOT = Path(__file__).resolve().parent.parent


class ItemStatus(str, Enum):
    STOPPED = "stopped"
    STARTING = "starting"
    HEALTHY = "healthy"
    UNHEALTHY = "unhealthy"
    STALE = "stale"
    DUPLICATE = "duplicate"
    NOT_STARTED = "not_started"
    WARN = "warn"


@dataclass
class ItemProbe:
    name: str
    version: str = "—"
    status: ItemStatus = ItemStatus.STOPPED
    pids: list[int] = field(default_factory=list)
    instances: int = 0
    uptime: str = "—"
    detail: str = ""


@dataclass
class BootstrapConfig:
    repo_root: Path
    project: Path
    mode: str
    api_port: int
    ui_port: int
    build: bool
    embedded: bool
    refresh_interval: float
    stop_existing: bool

    @property
    def teshi_bin(self) -> Path:
        name = "teshi.exe" if os.name == "nt" else "teshi"
        return self.repo_root / "target" / "debug" / name

    @property
    def desktop_bin(self) -> Path:
        name = "teshi-desktop.exe" if os.name == "nt" else "teshi-desktop"
        return self.repo_root / "target" / "debug" / name

    @property
    def sut_url(self) -> str:
        return f"http://127.0.0.1:{self.ui_port}/?e2e=1"

    @property
    def api_base(self) -> str:
        return f"http://127.0.0.1:{self.api_port}"


@dataclass
class ManagedProcess:
    label: str
    popen: subprocess.Popen[Any]
    log_path: Path | None = None


class ProcessSupervisor:
    """Spawns bootstrap child processes and tears down the process tree on exit."""

    def __init__(self, log_dir: Path) -> None:
        self.log_dir = log_dir
        self.processes: list[ManagedProcess] = []
        self.start_times: dict[int, float] = {}
        self._log_handles: list[Any] = []
        self._cleaned = False
        self.log_dir.mkdir(parents=True, exist_ok=True)

    @property
    def managed_pids(self) -> set[int]:
        return {
            mp.popen.pid for mp in self.processes if mp.popen.pid is not None
        }

    def spawn(
        self,
        args: list[str],
        *,
        label: str,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
    ) -> subprocess.Popen[Any]:
        merged = os.environ.copy()
        if env:
            merged.update(env)
        creationflags = 0
        if os.name == "nt":
            creationflags = subprocess.CREATE_NEW_PROCESS_GROUP  # type: ignore[attr-defined]

        log_path = self.log_dir / f"bootstrap-{label}.log"
        log_handle = log_path.open("a", encoding="utf-8")
        log_handle.write(f"\n--- spawn {datetime.now().isoformat()} {' '.join(args)} ---\n")
        log_handle.flush()
        self._log_handles.append(log_handle)

        proc = subprocess.Popen(
            args,
            cwd=str(cwd or REPO_ROOT),
            env=merged,
            stdout=log_handle,
            stderr=subprocess.STDOUT,
            creationflags=creationflags,
        )
        self.processes.append(ManagedProcess(label=label, popen=proc, log_path=log_path))
        if proc.pid is not None:
            self.start_times[proc.pid] = time.time()
        return proc

    def dead_processes(self) -> list[tuple[str, int, str]]:
        """Return exited children as (label, exit_code, log_tail)."""
        dead: list[tuple[str, int, str]] = []
        for mp in self.processes:
            code = mp.popen.poll()
            if code is None:
                continue
            tail = ""
            if mp.log_path and mp.log_path.is_file():
                try:
                    text = mp.log_path.read_text(encoding="utf-8", errors="replace")
                    tail = text[-400:].strip()
                except OSError:
                    pass
            dead.append((mp.label, code, tail))
        return dead

    def cleanup(self) -> None:
        if self._cleaned:
            return
        self._cleaned = True
        for mp in reversed(self.processes):
            proc = mp.popen
            if proc.poll() is None and proc.pid is not None:
                _kill_process_tree(proc.pid)
        self.processes.clear()
        for handle in self._log_handles:
            try:
                handle.close()
            except OSError:
                pass
        self._log_handles.clear()

    def uptime_for_pids(self, pids: list[int]) -> str:
        if not pids:
            return "—"
        earliest = None
        for pid in pids:
            t = self.start_times.get(pid)
            if t is None:
                try:
                    t = psutil.Process(pid).create_time()
                except (psutil.NoSuchProcess, psutil.AccessDenied):
                    continue
            if earliest is None or t < earliest:
                earliest = t
        if earliest is None:
            return "—"
        secs = int(time.time() - earliest)
        if secs < 60:
            return f"{secs}s"
        return f"{secs // 60}m{secs % 60}s"


def _kill_process_tree(pid: int) -> None:
    try:
        parent = psutil.Process(pid)
    except psutil.NoSuchProcess:
        return
    children = parent.children(recursive=True)
    for child in children:
        try:
            child.terminate()
        except psutil.NoSuchProcess:
            pass
    try:
        parent.terminate()
    except psutil.NoSuchProcess:
        return
    gone, alive = psutil.wait_procs(children + [parent], timeout=3)
    for proc in alive:
        try:
            proc.kill()
        except psutil.NoSuchProcess:
            pass


def _read_toml_version(path: Path) -> str | None:
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8")
    match = re.search(r'^version\s*=\s*"([^"]+)"', text, re.MULTILINE)
    return match.group(1) if match else None


def _read_json_version(path: Path, key: str = "version") -> str | None:
    if not path.is_file():
        return None
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        val = data.get(key)
        return str(val) if val is not None else None
    except (json.JSONDecodeError, OSError):
        return None


def _run_version(binary: Path) -> str | None:
    if not binary.is_file():
        return None
    try:
        out = subprocess.run(
            [str(binary), "--version"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
        line = (out.stdout or out.stderr or "").strip().splitlines()
        if not line:
            return None
        parts = line[0].split()
        return parts[-1] if parts else None
    except (subprocess.SubprocessError, OSError):
        return None


def _newest_mtime(root: Path) -> float:
    latest = 0.0
    if not root.is_dir():
        return latest
    for path in root.rglob("*.rs"):
        try:
            latest = max(latest, path.stat().st_mtime)
        except OSError:
            continue
    return latest


def _pids_listening_on(port: int) -> list[int]:
    pids: list[int] = []
    try:
        for conn in psutil.net_connections(kind="inet"):
            if conn.laddr and conn.laddr.port == port and conn.status == psutil.CONN_LISTEN:
                if conn.pid and conn.pid not in pids:
                    pids.append(conn.pid)
    except (psutil.Error, PermissionError):
        pass
    return pids


def _pids_by_name(name: str) -> list[int]:
    pids: list[int] = []
    name_lower = name.lower()
    for proc in psutil.process_iter(["pid", "name"]):
        try:
            pname = (proc.info.get("name") or "").lower()
            if pname == name_lower and proc.pid not in pids:
                pids.append(proc.pid)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return pids


def _pids_by_cmdline(fragment: str) -> list[int]:
    pids: list[int] = []
    needle = fragment.lower()
    for proc in psutil.process_iter(["pid", "cmdline"]):
        try:
            cmdline = proc.info.get("cmdline") or []
            joined = " ".join(cmdline).lower()
            if needle in joined and proc.pid not in pids:
                pids.append(proc.pid)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return pids


def _serve_embedded_pid(supervisor: ProcessSupervisor) -> int | None:
    """PID of the managed `teshi browser serve-embedded` child, if running."""
    for mp in supervisor.processes:
        if mp.label != "serve-embedded":
            continue
        if mp.popen.poll() is None and mp.popen.pid is not None:
            return mp.popen.pid
    return None


def _browser_service_pids_for_parent(parent_pid: int) -> list[int]:
    """Return browser_service.py PIDs in the process tree rooted at `parent_pid`."""
    try:
        parent = psutil.Process(parent_pid)
    except psutil.NoSuchProcess:
        return []
    pids: list[int] = []
    for child in parent.children(recursive=True):
        try:
            cmdline = " ".join(child.cmdline()).lower()
            if "browser_service.py" in cmdline and child.pid not in pids:
                pids.append(child.pid)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            continue
    return pids


def _stop_all_browser_services(timeout_secs: float = 10.0) -> None:
    """Stop every browser_service.py and wait until none remain."""
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        pids = _pids_by_cmdline("browser_service.py")
        if not pids:
            time.sleep(0.3)
            if not _pids_by_cmdline("browser_service.py"):
                return
        for pid in pids:
            _kill_process_tree(pid)
        time.sleep(0.5)
    for pid in _pids_by_cmdline("browser_service.py"):
        _kill_process_tree(pid)


def _reconcile_bootstrap_sidecars(supervisor: ProcessSupervisor) -> int:
    """Keep the serve-embedded sidecar only; kill extras (e.g. from teshi-desktop)."""
    serve_pid = _serve_embedded_pid(supervisor)
    if serve_pid is None:
        return 0
    managed = set(_browser_service_pids_for_parent(serve_pid))
    killed = 0
    for pid in _pids_by_cmdline("browser_service.py"):
        if pid in managed:
            continue
        _kill_process_tree(pid)
        killed += 1
    return killed


def _http_ok(url: str, timeout: float = 1.5, *, require_200: bool = False) -> tuple[bool, str]:
    try:
        with httpx.Client(timeout=timeout) as client:
            r = client.get(url)
            if require_200:
                ok = r.status_code == 200
            else:
                ok = r.status_code < 500
            if ok:
                return True, f"HTTP {r.status_code}"
            return False, f"HTTP {r.status_code}"
    except httpx.HTTPError as exc:
        return False, str(exc)


def _external_teshi_pids(cfg: BootstrapConfig) -> list[tuple[int, str]]:
    """PIDs for teshi.exe not built from this repo's target/debug."""
    try:
        debug_dir = cfg.teshi_bin.resolve().parent.as_posix().casefold()
    except OSError:
        return []
    found: list[tuple[int, str]] = []
    for proc in psutil.process_iter(["pid", "name", "exe"]):
        try:
            name = (proc.info.get("name") or "").lower()
            if name not in ("teshi.exe", "teshi"):
                continue
            exe = proc.info.get("exe")
            if not exe:
                continue
            parent = Path(exe).resolve().parent.as_posix().casefold()
            if parent != debug_dir:
                found.append((proc.pid, exe))
        except (psutil.NoSuchProcess, psutil.AccessDenied, OSError):
            continue
    return found


def _probe_repo_versions(cfg: BootstrapConfig) -> ItemProbe:
    cargo_v = _read_toml_version(cfg.repo_root / "Cargo.toml")
    pkg_v = _read_json_version(cfg.repo_root / "desktop" / "package.json")
    ext_v = _read_json_version(cfg.repo_root / "extension" / "teshi-bridge" / "manifest.json")
    versions = {"Cargo.toml": cargo_v, "package.json": pkg_v, "manifest.json": ext_v}
    unique = {v for v in versions.values() if v}
    if len(unique) <= 1:
        status = ItemStatus.HEALTHY
        detail = f"aligned: {cargo_v or '?'}"
    else:
        status = ItemStatus.WARN
        detail = ", ".join(f"{k}={v}" for k, v in versions.items())
    return ItemProbe(
        name="repo_version",
        version=cargo_v or "—",
        status=status,
        detail=detail,
    )


def _expected_debug_teshi_count(supervisor: ProcessSupervisor) -> int:
    """Managed `teshi.exe` roles: one for web, one for serve-embedded."""
    labels = {"teshi-web", "serve-embedded"}
    return sum(1 for mp in supervisor.processes if mp.label in labels)


def _probe_teshi_cli(cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> ItemProbe:
    binary = cfg.teshi_bin
    repo_v = _read_toml_version(cfg.repo_root / "Cargo.toml")
    run_v = _run_version(binary)
    debug_pids = _pids_running_binary(binary)
    external_installs = _external_teshi_pids(cfg)

    status = ItemStatus.STOPPED
    detail = ""
    if not binary.is_file():
        detail = f"missing {binary}"
    elif run_v is None:
        detail = "binary exists but --version failed"
        status = ItemStatus.UNHEALTHY
    else:
        src_mtime = _newest_mtime(cfg.repo_root / "src")
        bin_mtime = binary.stat().st_mtime
        if repo_v and run_v != repo_v:
            status = ItemStatus.STALE
            detail = f"built {run_v}, repo {repo_v}; run cargo build -p teshi"
        elif src_mtime > bin_mtime + 1:
            status = ItemStatus.STALE
            detail = "debug binary older than src/; run cargo build -p teshi"
        else:
            status = ItemStatus.HEALTHY
            detail = str(binary)

    if external_installs:
        ext_detail = ", ".join(f"PID {pid}" for pid, _ in external_installs)
        if status == ItemStatus.HEALTHY:
            status = ItemStatus.WARN
        detail = f"{detail}; WinGet/PATH teshi running ({ext_detail}) — stop if unused"

    expected = _expected_debug_teshi_count(supervisor)
    if expected > 0 and len(debug_pids) == expected:
        if status in (ItemStatus.HEALTHY, ItemStatus.STOPPED):
            status = ItemStatus.HEALTHY
        roles = [mp.label for mp in supervisor.processes if mp.label in ("teshi-web", "serve-embedded")]
        detail = f"{expected} expected ({', '.join(roles)})"
    elif expected > 0 and len(debug_pids) > expected:
        status = ItemStatus.DUPLICATE
        detail = f"{len(debug_pids)} debug teshi, expected {expected}: {debug_pids}"
    elif len(debug_pids) > 2:
        status = ItemStatus.DUPLICATE
        detail = f"{len(debug_pids)} unexpected debug teshi process(es): {debug_pids}"

    managed = [p for p in debug_pids if p in supervisor.managed_pids]
    all_pids = debug_pids + [pid for pid, _ in external_installs]
    return ItemProbe(
        name="teshi_cli",
        version=run_v or "—",
        status=status,
        pids=all_pids,
        instances=len(all_pids),
        uptime=supervisor.uptime_for_pids(managed or debug_pids),
        detail=detail,
    )


def _probe_teshi_desktop(
    cfg: BootstrapConfig, supervisor: ProcessSupervisor
) -> ItemProbe:
    binary = cfg.desktop_bin
    run_v = _run_version(binary)
    repo_v = _read_toml_version(cfg.repo_root / "desktop" / "src-tauri" / "Cargo.toml")
    debug_pids = _pids_running_binary(binary)

    status = ItemStatus.STOPPED
    detail = ""
    all_pids = debug_pids
    if debug_pids:
        status = ItemStatus.HEALTHY if len(debug_pids) == 1 else ItemStatus.DUPLICATE
        detail = f"{len(debug_pids)} running (tauri child)"
    elif cfg.mode == "tauri-dev":
        vite_mp = next(
            (mp for mp in supervisor.processes if mp.label == "vite"), None
        )
        if vite_mp is not None:
            code = vite_mp.popen.poll()
            if code is None:
                status = ItemStatus.STARTING
                detail = "waiting for desktop (start after Vite)"
                all_pids = []
            else:
                status = ItemStatus.UNHEALTHY
                detail = f"vite exited ({code}); see bootstrap-vite.log"
        else:
            tauri_pids = _pids_by_cmdline("tauri dev") or _pids_by_cmdline(
                "run tauri dev"
            )
            if tauri_pids:
                status = ItemStatus.STARTING
                detail = "npm run tauri dev"
                all_pids = tauri_pids

    if run_v and repo_v and run_v != repo_v and not debug_pids:
        status = ItemStatus.STALE
        detail = f"built {run_v}, repo {repo_v}; tauri dev will rebuild"

    return ItemProbe(
        name="teshi_desktop",
        version=run_v or "—",
        status=status,
        pids=all_pids,
        instances=len(all_pids),
        uptime=supervisor.uptime_for_pids(all_pids),
        detail=detail or (str(binary) if binary.is_file() else "not built"),
    )


def _probe_teshi_web(cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> ItemProbe:
    listeners = _pids_listening_on(cfg.api_port)
    url = f"{cfg.api_base}/api/v1/settings/recent"
    ok, msg = _http_ok(url)
    run_v = _run_version(cfg.teshi_bin)

    status = ItemStatus.STOPPED
    if len(listeners) > 1:
        status = ItemStatus.DUPLICATE
    elif ok:
        status = ItemStatus.HEALTHY
    elif listeners:
        status = ItemStatus.STARTING
    else:
        status = ItemStatus.UNHEALTHY if supervisor.managed_pids else ItemStatus.STOPPED

    external = [p for p in listeners if p not in supervisor.managed_pids]
    detail = msg
    if external:
        detail += f"; external listeners: {external}"

    return ItemProbe(
        name="teshi_web",
        version=run_v or "—",
        status=status,
        pids=listeners,
        instances=len(listeners),
        uptime=supervisor.uptime_for_pids([p for p in listeners if p in supervisor.managed_pids]),
        detail=detail,
    )


def _probe_vite_dev(cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> ItemProbe:
    listeners = _pids_listening_on(cfg.ui_port)
    ok, msg = _http_ok(f"http://127.0.0.1:{cfg.ui_port}/", require_200=True)
    pkg_v = _read_json_version(cfg.repo_root / "desktop" / "package.json")

    status = ItemStatus.STOPPED
    if len(listeners) > 1:
        status = ItemStatus.DUPLICATE
    elif ok:
        status = ItemStatus.HEALTHY
    elif listeners:
        status = ItemStatus.STARTING
        detail = msg
    else:
        vite_mp = next(
            (mp for mp in supervisor.processes if mp.label == "vite"), None
        )
        if vite_mp and vite_mp.popen.poll() is None:
            status = ItemStatus.STARTING
            detail = "waiting for Vite"
        elif msg.startswith("HTTP"):
            status = ItemStatus.UNHEALTHY
            detail = msg
        else:
            status = ItemStatus.UNHEALTHY if supervisor.managed_pids else ItemStatus.STOPPED
            detail = msg or "not listening"

    vite_pids = _pids_by_cmdline("vite.js") or _pids_by_cmdline("vite/bin")
    if len(listeners) == 1 and ok:
        status = ItemStatus.HEALTHY
        detail = msg
    elif len(vite_pids) > 2 and status != ItemStatus.DUPLICATE:
        status = ItemStatus.WARN
        detail = f"{len(vite_pids)} node/vite workers (normal: npm + vite); {msg}"

    if not ok and status == ItemStatus.STOPPED and detail == "":
        detail = msg

    return ItemProbe(
        name="vite_dev",
        version=pkg_v or "—",
        status=status,
        pids=listeners or vite_pids,
        instances=max(len(listeners), len(vite_pids)),
        uptime=supervisor.uptime_for_pids(listeners),
        detail=detail,
    )


def _probe_sidecar(cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> ItemProbe:
    endpoint = cfg.project / ".teshi" / "cdp-endpoint.json"
    serve_pid = _serve_embedded_pid(supervisor)
    managed_bs = (
        _browser_service_pids_for_parent(serve_pid) if serve_pid is not None else []
    )
    browser_pids = _pids_by_cmdline("browser_service.py")
    bridge_port_pids = _pids_listening_on(17373)

    if not endpoint.is_file():
        return ItemProbe(
            name="sidecar_embedded",
            version="—",
            status=ItemStatus.NOT_STARTED,
            pids=browser_pids,
            instances=len(browser_pids),
            detail="no cdp-endpoint.json; serve-embedded not running",
        )

    status = ItemStatus.STARTING
    detail = ""
    try:
        proc = subprocess.run(
            [str(cfg.teshi_bin), "browser", "doctor"],
            cwd=str(cfg.project),
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
        raw = (proc.stdout or proc.stderr or "").strip()
        if raw:
            try:
                report = json.loads(raw)
            except json.JSONDecodeError:
                report = None
            if isinstance(report, dict):
                mode = report.get("mode", "?")
                detail = f"mode={mode}, page={report.get('page_url') or '—'}"
                if report.get("ok"):
                    status = ItemStatus.HEALTHY
                else:
                    status = ItemStatus.UNHEALTHY
                    err = report.get("error") or "doctor failed"
                    detail += f"; {err}"
            else:
                status = ItemStatus.UNHEALTHY
                detail = raw[:200]
        else:
            status = ItemStatus.UNHEALTHY
            detail = "doctor produced no output"
    except (subprocess.SubprocessError, json.JSONDecodeError, OSError) as exc:
        status = ItemStatus.UNHEALTHY
        detail = str(exc)

    if len(browser_pids) > 1:
        status = ItemStatus.DUPLICATE
        extra = [p for p in browser_pids if p not in managed_bs]
        if managed_bs and extra:
            detail = (
                f"{len(browser_pids)} browser_service.py "
                f"(bootstrap keeps {managed_bs[0]}, extra {extra}); {detail}"
            )
        else:
            detail = f"{len(browser_pids)} browser_service.py (expected 1); {detail}"
    elif (
        len(browser_pids) == 1
        and managed_bs
        and browser_pids[0] not in managed_bs
        and status == ItemStatus.HEALTHY
    ):
        status = ItemStatus.WARN
        detail = (
            f"sidecar pid {browser_pids[0]} not under serve-embedded; {detail}"
        )
    elif len(browser_pids) == 1 and status == ItemStatus.HEALTHY:
        detail = f"1 sidecar; {detail}"

    return ItemProbe(
        name="sidecar_embedded",
        version="python",
        status=status,
        pids=browser_pids or bridge_port_pids,
        instances=len(browser_pids),
        uptime=supervisor.uptime_for_pids(managed_bs or browser_pids),
        detail=detail,
    )


def _probe_python_venv(cfg: BootstrapConfig) -> ItemProbe:
    venv = cfg.project / ".venv"
    if not venv.is_dir():
        venv = cfg.project / "venv"
    if not venv.is_dir():
        return ItemProbe(
            name="python_venv",
            version="—",
            status=ItemStatus.UNHEALTHY,
            detail="no .venv or venv; create and pip install -r python/requirements.txt",
        )

    py = venv / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
    if not py.is_file():
        return ItemProbe(
            name="python_venv",
            version="—",
            status=ItemStatus.UNHEALTHY,
            detail=f"missing interpreter {py}",
        )

    try:
        out = subprocess.run(
            [str(py), "-c", "import playwright, websockets; print('ok')"],
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )
        if out.returncode == 0:
            return ItemProbe(
                name="python_venv",
                version="ok",
                status=ItemStatus.HEALTHY,
                detail=str(venv),
            )
        detail = (out.stderr or out.stdout or "import failed").strip()[:200]
    except (subprocess.SubprocessError, OSError) as exc:
        detail = str(exc)

    return ItemProbe(
        name="python_venv",
        version="—",
        status=ItemStatus.UNHEALTHY,
        detail=detail,
    )


class ProbeRegistry:
    def __init__(self, cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> None:
        self.cfg = cfg
        self.supervisor = supervisor

    def collect(self) -> list[ItemProbe]:
        return [
            _probe_repo_versions(self.cfg),
            _probe_teshi_cli(self.cfg, self.supervisor),
            _probe_teshi_desktop(self.cfg, self.supervisor),
            _probe_teshi_web(self.cfg, self.supervisor),
            _probe_vite_dev(self.cfg, self.supervisor),
            _probe_sidecar(self.cfg, self.supervisor),
            _probe_python_venv(self.cfg),
        ]


def _status_style(status: ItemStatus) -> str:
    return {
        ItemStatus.HEALTHY: "green",
        ItemStatus.STARTING: "yellow",
        ItemStatus.STOPPED: "dim",
        ItemStatus.NOT_STARTED: "dim",
        ItemStatus.UNHEALTHY: "red",
        ItemStatus.STALE: "magenta",
        ItemStatus.DUPLICATE: "bold red",
        ItemStatus.WARN: "yellow",
    }.get(status, "white")


def _build_table(items: list[ItemProbe]) -> Table:
    table = Table(title="teshi bootstrap dev", expand=True)
    table.add_column("Item", style="cyan", no_wrap=True)
    table.add_column("Version")
    table.add_column("Status")
    table.add_column("Inst", justify="right")
    table.add_column("PID(s)")
    table.add_column("Uptime")
    table.add_column("Detail", overflow="fold")

    for item in items:
        status_text = Text(item.status.value, style=_status_style(item.status))
        pids = ", ".join(str(p) for p in item.pids[:6])
        if len(item.pids) > 6:
            pids += f" +{len(item.pids) - 6}"
        table.add_row(
            item.name,
            item.version,
            status_text,
            str(item.instances) if item.instances else "0",
            pids or "—",
            item.uptime,
            item.detail[:120] if item.detail else "—",
        )

    return table


def _build_header(cfg: BootstrapConfig, items: list[ItemProbe]) -> Text:
    dup = any(i.status == ItemStatus.DUPLICATE for i in items)
    warn = any(i.status in (ItemStatus.STALE, ItemStatus.WARN, ItemStatus.UNHEALTHY) for i in items)
    line = (
        f"project={cfg.project}  mode={cfg.mode}  "
        f"SUT={cfg.sut_url}  API={cfg.api_base}  "
        f"teshi={cfg.teshi_bin.name}"
    )
    header = Text(line + "\n")
    if dup:
        header.append("DUPLICATE INSTANCES DETECTED — check Inst/PID columns\n", style="bold red")
    elif warn:
        header.append("Some items need attention (stale/unhealthy)\n", style="yellow")
    else:
        header.append("All monitored items OK\n", style="green")
    header.append(
        f"Updated {datetime.now().strftime('%H:%M:%S')}  Ctrl+C to stop all\n",
        style="dim",
    )
    return header


def _npm_cmd() -> str:
    return "npm.cmd" if os.name == "nt" else "npm"


def _preflight(cfg: BootstrapConfig) -> None:
    """Verify tools required for the selected launch mode."""
    console = Console(stderr=True)
    if cfg.mode == "tauri-dev":
        desktop = cfg.repo_root / "desktop"
        if not (desktop / "node_modules").is_dir():
            console.print("[dim]Installing desktop npm dependencies...[/dim]")
            subprocess.run(
                [_npm_cmd(), "install"],
                cwd=str(desktop),
                check=True,
            )
    if cfg.mode == "separate" and not cfg.desktop_bin.is_file():
        raise SystemExit(
            f"teshi-desktop not found at {cfg.desktop_bin}; pass --build"
        )


def _wait_for_http(url: str, timeout_secs: float = 60.0) -> bool:
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        ok, _ = _http_ok(url, timeout=2.0, require_200=True)
        if ok:
            return True
        time.sleep(0.5)
    return False


def _pids_running_binary(binary: Path) -> list[int]:
    """Return PIDs whose main executable matches `binary` (exact resolved path)."""
    if not binary.is_file():
        return []
    try:
        target = binary.resolve().as_posix().casefold()
    except OSError:
        return []
    pids: list[int] = []
    for proc in psutil.process_iter(["pid", "exe"]):
        try:
            exe = proc.info.get("exe")
            if not exe:
                continue
            if Path(exe).resolve().as_posix().casefold() == target:
                if proc.pid not in pids:
                    pids.append(proc.pid)
        except (psutil.NoSuchProcess, psutil.AccessDenied, OSError):
            continue
    return pids


def _binaries_blocking_build(cfg: BootstrapConfig) -> dict[Path, list[int]]:
    blocked: dict[Path, list[int]] = {}
    for binary in (cfg.teshi_bin, cfg.desktop_bin):
        pids = _pids_running_binary(binary)
        if pids:
            blocked[binary] = pids
    return blocked


def _cleanup_before_start(cfg: BootstrapConfig, *, aggressive: bool) -> list[int]:
    """Free bootstrap ports and orphan sidecars before spawning a fresh stack."""
    stopped: list[int] = []
    seen: set[int] = set()

    def stop_pid(pid: int) -> None:
        if pid in seen or pid == os.getpid():
            return
        seen.add(pid)
        _kill_process_tree(pid)
        stopped.append(pid)

    for pid in _pids_by_cmdline("browser_service.py"):
        stop_pid(pid)

    for port in (cfg.api_port, cfg.ui_port, 17373):
        for pid in _pids_listening_on(port):
            stop_pid(pid)

    if aggressive:
        for binary in (cfg.teshi_bin, cfg.desktop_bin):
            for pid in _pids_running_binary(binary):
                stop_pid(pid)
        for pid in _pids_by_cmdline("tauri dev"):
            stop_pid(pid)
        for pid in _pids_by_cmdline("run tauri dev"):
            stop_pid(pid)

    if stopped:
        time.sleep(0.8)
    return stopped


def _stop_existing_stack(cfg: BootstrapConfig) -> list[int]:
    """Stop prior bootstrap/dev processes that block rebuild or port bind."""
    return _cleanup_before_start(cfg, aggressive=True)


def _can_replace_binary(binary: Path) -> bool:
    """True when the file is absent or opened for write (not locked by a running process)."""
    if not binary.is_file():
        return True
    try:
        with binary.open("r+b"):
            pass
        return True
    except OSError:
        return False


def _release_teshi_exe_lock(cfg: BootstrapConfig) -> list[int]:
    """Stop bootstrap-related processes and wait until teshi.exe can be rebuilt on Windows."""
    console = Console(stderr=True)
    stopped: list[int] = []
    seen: set[int] = set()

    def stop_pid(pid: int) -> None:
        if pid in seen or pid == os.getpid():
            return
        seen.add(pid)
        _kill_process_tree(pid)
        stopped.append(pid)

    if cfg.stop_existing:
        for pid in _cleanup_before_start(cfg, aggressive=True):
            if pid not in seen:
                seen.add(pid)
                stopped.append(pid)

    for attempt in range(20):
        for binary in (cfg.desktop_bin, cfg.teshi_bin):
            for pid in _pids_running_binary(binary):
                stop_pid(pid)
        for name in ("teshi.exe", "teshi-desktop.exe", "teshi"):
            for pid in _pids_by_name(name):
                stop_pid(pid)
        for port in (cfg.api_port, cfg.ui_port, 17373):
            for pid in _pids_listening_on(port):
                stop_pid(pid)
        for pid in _pids_by_cmdline("browser_service.py"):
            stop_pid(pid)
        for pid in _pids_by_cmdline("bootstrap_dev.py"):
            if pid != os.getpid():
                stop_pid(pid)

        remaining = _pids_running_binary(cfg.teshi_bin)
        if not remaining and _can_replace_binary(cfg.teshi_bin):
            time.sleep(0.5)
            if _can_replace_binary(cfg.teshi_bin):
                return stopped

        if attempt == 0 and stopped:
            console.print(
                f"[yellow]Stopped {len(stopped)} process(es), waiting for teshi.exe unlock...[/yellow]"
            )
        elif attempt % 3 == 0:
            console.print(
                f"[dim]Waiting for teshi.exe unlock ({attempt + 1}/20, "
                f"PIDs: {remaining or 'none'})[/dim]"
            )
        time.sleep(1.0)

    remaining = _pids_running_binary(cfg.teshi_bin)
    raise SystemExit(
        f"Cannot unlock {cfg.teshi_bin} for rebuild.\n"
        f"Remaining teshi PIDs: {remaining or 'unknown (handle held)'}\n"
        "Close teshi-desktop and any terminal running teshi, then retry:\n"
        "  python scripts/bootstrap_dev.py --project . --stop-existing\n"
        "Do not run Stop-Process -Name teshi before bootstrap; --stop-existing is enough."
    )


def _run_cargo_build(cfg: BootstrapConfig, package: str) -> None:
    """Run cargo build with retries when Windows temporarily locks the output exe."""
    last_err: subprocess.CalledProcessError | None = None
    for attempt in range(5):
        try:
            subprocess.run(
                ["cargo", "build", "-p", package],
                cwd=str(cfg.repo_root),
                check=True,
            )
            return
        except subprocess.CalledProcessError as err:
            last_err = err
            if attempt >= 4:
                break
            Console(stderr=True).print(
                f"[yellow]cargo build -p {package} failed (attempt {attempt + 1}), "
                "retrying after unlock...[/yellow]"
            )
            _release_teshi_exe_lock(cfg)
    assert last_err is not None
    raise last_err


def _build_workspace(cfg: BootstrapConfig) -> None:
    """Build teshi + desktop while teshi.exe is not locked."""
    console = Console(stderr=True)
    stopped = _release_teshi_exe_lock(cfg)
    if stopped:
        console.print(f"[yellow]Released lock ({len(stopped)} process(es) stopped)[/yellow]")
    console.print("[dim]Building teshi CLI and desktop...[/dim]")
    _run_cargo_build(cfg, "teshi")
    _run_cargo_build(cfg, "teshi-desktop")


def _ensure_built(cfg: BootstrapConfig) -> None:
    console = Console(stderr=True)
    if cfg.mode == "tauri-dev":
        _build_workspace(cfg)
        return
    _release_teshi_exe_lock(cfg)
    console.print("[dim]Building teshi CLI...[/dim]")
    _run_cargo_build(cfg, "teshi")
    if cfg.mode == "separate":
        console.print("[dim]Building teshi-desktop...[/dim]")
        _run_cargo_build(cfg, "teshi-desktop")


def _build_spawn_panel(supervisor: ProcessSupervisor) -> Text:
    panel = Text("Managed processes:\n", style="bold")
    for mp in supervisor.processes:
        proc = mp.popen
        pid = proc.pid or "?"
        code = proc.poll()
        state = f"exit {code}" if code is not None else "running"
        log_hint = f"  log: {mp.log_path}" if mp.log_path else ""
        panel.append(f"  {mp.label}: PID {pid} ({state}){log_hint}\n", style="dim")
    for label, code, tail in supervisor.dead_processes():
        panel.append(f"  FAILED {label} (exit {code})\n", style="bold red")
        if tail:
            for line in tail.splitlines()[-3:]:
                panel.append(f"    {line}\n", style="red")
    return panel


def _start_stack(cfg: BootstrapConfig, supervisor: ProcessSupervisor) -> None:
    """Spawn child processes without blocking; dashboard shows startup progress."""
    console = Console(stderr=True)
    stopped = _cleanup_before_start(cfg, aggressive=cfg.stop_existing)
    if stopped:
        console.print(
            f"[yellow]Cleaned {len(stopped)} prior process(es): {stopped}[/yellow]"
        )

    teshi = str(cfg.teshi_bin)
    project = str(cfg.project.resolve())

    if cfg.mode != "tauri-dev" and not cfg.teshi_bin.is_file():
        raise SystemExit(f"teshi binary not found at {cfg.teshi_bin}; pass --build")

    if cfg.mode == "tauri-dev":
        # Vite only — skip npm `predev` (cargo build) to avoid locking teshi.exe on Windows.
        supervisor.spawn(
            [_npm_cmd(), "run", "dev", "--ignore-scripts"],
            cwd=cfg.repo_root / "desktop",
            label="vite",
        )
    elif cfg.mode == "separate":
        if not cfg.teshi_bin.is_file():
            raise SystemExit(f"teshi binary not found at {cfg.teshi_bin}; pass --build")
        supervisor.spawn(
            [_npm_cmd(), "run", "dev"],
            cwd=cfg.repo_root / "desktop",
            label="vite",
        )
        supervisor.spawn(
            [str(cfg.desktop_bin), "--project", project],
            cwd=cfg.repo_root,
            label="teshi-desktop",
        )
        console.print(f"[dim]Starting teshi web on :{cfg.api_port}...[/dim]")
        supervisor.spawn(
            [teshi, "web", "--project", project, "--port", str(cfg.api_port), "--no-open"],
            cwd=cfg.repo_root,
            label="teshi-web",
        )


def _maybe_start_desktop(
    cfg: BootstrapConfig,
    supervisor: ProcessSupervisor,
    *,
    desktop_started: bool,
) -> bool:
    if desktop_started or cfg.mode != "tauri-dev":
        return desktop_started
    if any(mp.label == "teshi-desktop" for mp in supervisor.processes):
        return True
    ui_ok, _ = _http_ok(f"http://127.0.0.1:{cfg.ui_port}/", require_200=True)
    if not ui_ok or not cfg.desktop_bin.is_file():
        return False
    project = str(cfg.project.resolve())
    supervisor.spawn(
        [str(cfg.desktop_bin), "--project", project],
        cwd=cfg.repo_root,
        label="teshi-desktop",
    )
    return True


def _maybe_start_web(
    cfg: BootstrapConfig,
    supervisor: ProcessSupervisor,
    *,
    web_started: bool,
) -> bool:
    """Start teshi web after tauri predev + Vite are ready (avoids Windows exe lock)."""
    if web_started or cfg.mode != "tauri-dev":
        return web_started
    if any(mp.label == "teshi-web" for mp in supervisor.processes):
        return True
    ui_ok, _ = _http_ok(f"http://127.0.0.1:{cfg.ui_port}/", require_200=True)
    if not ui_ok:
        return False
    if not cfg.teshi_bin.is_file():
        return False
    teshi = str(cfg.teshi_bin)
    project = str(cfg.project.resolve())
    supervisor.spawn(
        [teshi, "web", "--project", project, "--port", str(cfg.api_port), "--no-open"],
        cwd=cfg.repo_root,
        label="teshi-web",
    )
    return True


def _maybe_start_embedded(
    cfg: BootstrapConfig,
    supervisor: ProcessSupervisor,
    *,
    embedded_started: bool,
) -> bool:
    if embedded_started or not cfg.embedded:
        return embedded_started
    if any(mp.label == "serve-embedded" for mp in supervisor.processes):
        return True
    api_ok, _ = _http_ok(
        f"{cfg.api_base}/api/v1/settings/recent", require_200=True
    )
    ui_ok, _ = _http_ok(
        f"http://127.0.0.1:{cfg.ui_port}/", require_200=True
    )
    if not (api_ok and ui_ok):
        return False
    if cfg.mode == "tauri-dev" and not any(
        mp.label == "teshi-web" for mp in supervisor.processes
    ):
        return False
    if cfg.mode == "tauri-dev" and not _pids_by_name(cfg.desktop_bin.name):
        return False
    _stop_all_browser_services()
    teshi = str(cfg.teshi_bin)
    project = str(cfg.project.resolve())
    supervisor.spawn(
        [
            teshi,
            "browser",
            "serve-embedded",
            "--project",
            project,
            "--navigate",
            cfg.sut_url,
        ],
        cwd=cfg.project,
        label="serve-embedded",
        env={
            # Avoid nested `teshi browser serve-embedded` spawns during navigate/doctor.
            "TESHI_BROWSER_AUTO_RECONNECT": "0",
        },
    )
    return True


def _parse_args(argv: list[str] | None = None) -> BootstrapConfig:
    parser = argparse.ArgumentParser(
        description="One-command teshi web UI self-bootstrap with live status dashboard.",
    )
    parser.add_argument(
        "project",
        nargs="?",
        default=".",
        help="BDD project directory (default: current directory)",
    )
    parser.add_argument(
        "--project",
        dest="project_flag",
        default=None,
        help="Project directory (overrides positional)",
    )
    parser.add_argument(
        "--mode",
        choices=("tauri-dev", "separate"),
        default="tauri-dev",
        help="tauri-dev: vite + teshi-desktop (no npm predev); separate: desktop exe + npm run dev",
    )
    parser.add_argument("--api-port", type=int, default=1421, help="teshi web API port")
    parser.add_argument("--ui-port", type=int, default=1420, help="Vite dev / SUT UI port")
    parser.add_argument("--build", action="store_true", help="Run cargo build before starting")
    parser.add_argument(
        "--stop-existing",
        action="store_true",
        help="Stop repo debug teshi processes and bootstrap ports before build/start",
    )
    parser.add_argument(
        "--no-embedded",
        action="store_true",
        help="Do not auto-start serve-embedded sidecar",
    )
    parser.add_argument(
        "--refresh-interval",
        type=float,
        default=1.0,
        help="Dashboard refresh interval in seconds",
    )
    args = parser.parse_args(argv)

    project_raw = args.project_flag if args.project_flag is not None else args.project
    project = Path(project_raw).resolve()
    if not project.is_dir():
        raise SystemExit(f"project directory not found: {project}")

    return BootstrapConfig(
        repo_root=REPO_ROOT,
        project=project,
        mode=args.mode,
        api_port=args.api_port,
        ui_port=args.ui_port,
        build=args.build,
        embedded=not args.no_embedded,
        refresh_interval=args.refresh_interval,
        stop_existing=args.stop_existing,
    )


def main(argv: list[str] | None = None) -> int:
    cfg = _parse_args(argv)
    log_dir = cfg.project / ".teshi" / "logs"
    supervisor = ProcessSupervisor(log_dir)
    atexit.register(supervisor.cleanup)

    def _handle_signal(signum: int, _frame: Any) -> None:
        supervisor.cleanup()
        raise SystemExit(0)

    signal.signal(signal.SIGINT, _handle_signal)
    if hasattr(signal, "SIGTERM"):
        signal.signal(signal.SIGTERM, _handle_signal)

    _preflight(cfg)
    if cfg.mode == "tauri-dev" or cfg.build:
        _ensure_built(cfg)

    _start_stack(cfg, supervisor)

    registry = ProbeRegistry(cfg, supervisor)
    console = Console(force_terminal=True)
    embedded_started = False
    web_started = cfg.mode != "tauri-dev"
    desktop_started = cfg.mode != "tauri-dev"

    def render() -> Group:
        nonlocal embedded_started, web_started, desktop_started
        desktop_started = _maybe_start_desktop(
            cfg, supervisor, desktop_started=desktop_started
        )
        web_started = _maybe_start_web(
            cfg, supervisor, web_started=web_started
        )
        embedded_started = _maybe_start_embedded(
            cfg, supervisor, embedded_started=embedded_started
        )
        if embedded_started:
            _reconcile_bootstrap_sidecars(supervisor)
        items = registry.collect()
        return Group(
            _build_header(cfg, items),
            _build_table(items),
            _build_spawn_panel(supervisor),
        )

    # Show the live dashboard immediately while services start in the background.
    with Live(render(), console=console, refresh_per_second=4, screen=False) as live:
        while True:
            time.sleep(cfg.refresh_interval)
            live.update(render())

    return 0


if __name__ == "__main__":
    sys.exit(main())
