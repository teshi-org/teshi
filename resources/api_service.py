"""Loopback WebSocket sidecar for Teshi HTTP API BDD.

Mirrors the browser/WinApp bridge shape: clients send one JSON command and
receive a typed ``response``. ``http_exchange`` / step events are included in
the response ``events`` array (and optionally NDJSON on stdout).
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import sys
from pathlib import Path
from typing import Any

try:
    import websockets
except ImportError as exc:  # pragma: no cover - startup preflight catches this
    print(f"websockets import failed: {exc}", file=sys.stderr)
    raise

from teshi_api import Session, configure


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Teshi API BDD sidecar")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--project", "--project-root", dest="project_root", default=".")
    return parser.parse_args()


def handle_command(session: Session, data: dict[str, Any]) -> dict[str, Any]:
    """Dispatch one sidecar command against the shared session."""
    cmd = str(data.get("cmd") or "")
    if cmd == "ping":
        return {"ok": True}
    if cmd == "doctor":
        steps_dir = session.project_root / "features" / "steps"
        templates = session._template_roots[0] if session._template_roots else None
        return {
            "ok": True,
            "project_root": str(session.project_root),
            "templates": str(templates) if templates else None,
            "steps_dir": str(steps_dir),
            "steps_dir_exists": steps_dir.is_dir(),
            "step_defs": len(session._step_defs),
        }
    if cmd == "begin_scenario":
        session.begin_scenario()
        if data.get("case_id"):
            session.set_step_context(str(data["case_id"]), None)
        return {"ok": True, "vars": dict(session.vars)}
    if cmd == "execute_step":
        text = str(data.get("text") or "")
        result = session.execute_step(
            text,
            case_id=data.get("case_id"),
            step_id=data.get("step_id"),
        )
        result.setdefault("ok", True)
        return result
    if cmd == "call":
        template = str(data.get("template") or "")
        raw = session.call(template)
        return {"ok": True, "exchange": session.get_exchange(raw["exchange_id"], redact=True)}
    if cmd == "get_exchange":
        exchange_id = str(data.get("exchange_id") or data.get("id") or "")
        redact = data.get("redact", True)
        return {"ok": True, "exchange": session.get_exchange(exchange_id, redact=bool(redact))}
    return {"ok": False, "error": f"unknown cmd {cmd}"}


async def run_server(host: str, port: int, project_root: Path) -> None:
    session = configure(project_root)
    session.load_step_modules()

    async def handler(websocket: Any) -> None:
        async for raw in websocket:
            request_id = ""
            try:
                data = json.loads(raw)
                request_id = str(data.get("request_id") or "")
                payload = handle_command(session, data)
            except Exception as exc:
                payload = {"ok": False, "error": str(exc)}
            await websocket.send(
                json.dumps({"type": "response", "request_id": request_id, **payload}, ensure_ascii=False)
            )

    listener = await websockets.serve(handler, host, port)
    sockets = getattr(listener, "sockets", None) or []
    actual_port = sockets[0].getsockname()[1] if sockets else port
    endpoint = {
        "ws_url": f"ws://{host}:{actual_port}",
        "pid": os.getpid(),
        "project_root": str(project_root.resolve()),
    }
    teshi_dir = project_root / ".teshi"
    teshi_dir.mkdir(parents=True, exist_ok=True)
    (teshi_dir / "api-endpoint.json").write_text(json.dumps(endpoint, indent=2), encoding="utf-8")
    print(json.dumps({"ok": True, **endpoint}), flush=True)
    await asyncio.Future()


def main() -> None:
    args = parse_args()
    project_root = Path(args.project_root).resolve()
    resources = Path(__file__).resolve().parent
    if str(resources) not in sys.path:
        sys.path.insert(0, str(resources))
    asyncio.run(run_server(args.host, args.port, project_root))


if __name__ == "__main__":
    main()
