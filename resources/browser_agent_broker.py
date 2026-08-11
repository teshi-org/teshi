"""Versioned multi-session broker contracts for Teshi browser agents."""

from __future__ import annotations

import asyncio
import json
import secrets
import time
from dataclasses import dataclass, field
from typing import Any

SCHEMA_VERSION = 1
PROTOCOL_VERSION = 1
LEGACY_PROTOCOL_VERSION = 0
LEGACY_INSTANCE_ID = "legacy-single-session"
DEFAULT_LEASE_TTL_SEC = 60
MIN_LEASE_TTL_SEC = 5
MAX_LEASE_TTL_SEC = 3600
DEFAULT_HEARTBEAT_TTL_SEC = 8.0
DISCONNECTED_RETENTION_SEC = 60.0
MAX_COMMAND_QUEUE = 64
MAX_PENDING_REQUESTS = 128

MUTATING_COMMANDS = {
    "get_page_snapshot",
    "resolve_playwright_locator",
    "verify_playwright_locator",
    "verify_playwright_locators",
    "capture_browser_evidence",
    "highlight_selector",
    "clear_highlight",
    "execute_locator",
    "heal_execute_locator",
    "enhance_locator",
    "navigate",
    "activate_tab",
    "open_project",
}


class BrokerError(Exception):
    """Stable broker failure with non-sensitive recovery metadata."""

    def __init__(
        self,
        code: str,
        message: str,
        recovery: dict[str, Any] | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.recovery = recovery or {}

    def response(
        self,
        request_id: str = "",
        operation: str = "",
    ) -> dict[str, Any]:
        """Render the shared machine-readable error envelope."""
        response: dict[str, Any] = {
            "type": "response",
            "schema_version": SCHEMA_VERSION,
            "request_id": request_id,
            "ok": False,
            "code": self.code,
            "error": self.message,
        }
        if operation:
            response["operation"] = operation
        if self.recovery:
            response["recovery"] = self.recovery
        return response


@dataclass
class LeaseState:
    """Exclusive browser-session lease held by one external caller."""

    token: str
    owner_label: str
    acquired_wall_time: float
    expires_monotonic: float
    expires_wall_time: float

    def expired(self, now: float) -> bool:
        """Return whether the monotonic expiry has elapsed."""
        return now >= self.expires_monotonic

    def public_summary(self) -> dict[str, Any]:
        """Return lease metadata without exposing the secret token."""
        return {
            "owner_label": self.owner_label,
            "expires_at_ms": int(self.expires_wall_time * 1000),
        }

    def private_result(self, extension_instance_id: str) -> dict[str, Any]:
        """Return the full lease contract to its owner."""
        return {
            "extension_instance_id": extension_instance_id,
            "lease_token": self.token,
            "owner_label": self.owner_label,
            "acquired_at_ms": int(self.acquired_wall_time * 1000),
            "expires_at_ms": int(self.expires_wall_time * 1000),
        }


@dataclass
class SessionRecord:
    """Mutable broker state for one extension installation."""

    extension_instance_id: str
    profile_label: str = ""
    extension_version: str = "legacy"
    protocol_version: int = LEGACY_PROTOCOL_VERSION
    browser: dict[str, Any] = field(default_factory=dict)
    windows: list[dict[str, Any]] = field(default_factory=list)
    active_window_id: int | None = None
    active_tab_id: int | None = None
    page_url: str = ""
    page_title: str = ""
    last_heartbeat: float = 0.0
    last_frame_at: float | None = None
    last_frame_error: str = ""
    command_queue: list[dict[str, Any]] = field(default_factory=list)
    lease: LeaseState | None = None
    latest_frame: dict[str, Any] | None = None

    def is_legacy(self) -> bool:
        """Return whether this record came from the pre-versioned heartbeat."""
        return self.protocol_version == LEGACY_PROTOCOL_VERSION

    def compatible(self) -> bool:
        """Return whether this extension may accept protocol-v1 commands."""
        return self.is_legacy() or self.protocol_version == PROTOCOL_VERSION

    def alive(self, now: float, heartbeat_ttl: float) -> bool:
        """Return whether the latest heartbeat is within the live window."""
        return now - self.last_heartbeat < heartbeat_ttl

    def health(self, now: float, heartbeat_ttl: float) -> str:
        """Return the stable public health state."""
        if not self.compatible():
            return "incompatible"
        if not self.alive(now, heartbeat_ttl):
            return "disconnected"
        if self.last_frame_error and "debugger" in self.last_frame_error.lower():
            return "debugger_conflict"
        if now - self.last_heartbeat > heartbeat_ttl * 0.75:
            return "stale"
        return "ready"

    def iter_tabs(self) -> list[dict[str, Any]]:
        """Flatten the normalized window inventory while preserving window IDs."""
        tabs: list[dict[str, Any]] = []
        for window in self.windows:
            window_id = _as_int(window.get("id"))
            for raw_tab in window.get("tabs", []):
                if not isinstance(raw_tab, dict):
                    continue
                tab = dict(raw_tab)
                tab["window_id"] = _as_int(tab.get("window_id")) or window_id or 0
                tabs.append(tab)
        return tabs

    def active_target(self) -> dict[str, Any] | None:
        """Return this session's current active debuggable target."""
        active: dict[str, Any] | None = None
        for tab in self.iter_tabs():
            tab_id = _as_int(tab.get("id"))
            if tab_id is None or tab.get("debuggable") is False:
                continue
            if tab_id == self.active_tab_id or tab.get("active"):
                active = tab
                if tab_id == self.active_tab_id:
                    break
        if active is None:
            return None
        return {
            "extension_instance_id": self.extension_instance_id,
            "window_id": _as_int(active.get("window_id")) or self.active_window_id or 0,
            "tab_id": int(active["id"]),
        }

    def public_contract(self, now: float, heartbeat_ttl: float) -> dict[str, Any]:
        """Return non-sensitive session discovery metadata."""
        contract: dict[str, Any] = {
            "schema_version": SCHEMA_VERSION,
            "identity": {
                "extension_instance_id": self.extension_instance_id,
                "profile_label": self.profile_label or None,
                "extension_version": self.extension_version,
                "protocol_version": self.protocol_version,
            },
            "browser": {
                "name": str(self.browser.get("name", "Chromium")),
                "version": str(self.browser.get("version", "")),
                "platform": self.browser.get("platform"),
            },
            "health": self.health(now, heartbeat_ttl),
            "last_heartbeat_age_ms": max(
                0, int((now - self.last_heartbeat) * 1000)
            ),
            "windows": self.windows,
        }
        if self.lease is not None and not self.lease.expired(now):
            contract["lease"] = self.lease.public_summary()
        return contract


@dataclass
class PendingRequest:
    """Correlation record for a command queued to one extension session."""

    request_id: str
    operation: str
    target: dict[str, Any]
    extension_instance_id: str
    future: asyncio.Future[dict[str, Any]]
    ephemeral_lease_token: str | None = None


class BrowserSessionBroker:
    """Instance-indexed registry, router, lease manager, and response correlator."""

    def __init__(self, heartbeat_ttl: float = DEFAULT_HEARTBEAT_TTL_SEC) -> None:
        self.heartbeat_ttl = heartbeat_ttl
        self.sessions: dict[str, SessionRecord] = {}
        self.pending: dict[str, PendingRequest] = {}
        self.quarantined_responses: list[dict[str, Any]] = []

    def register_heartbeat(self, payload: dict[str, Any]) -> SessionRecord:
        """Register or refresh one extension session from a heartbeat."""
        now = time.monotonic()
        self.expire_stale(now)
        instance_id = _clean_text(payload.get("extension_instance_id"))
        protocol_version = _as_int(payload.get("protocol_version"))
        if not instance_id:
            instance_id = LEGACY_INSTANCE_ID
            protocol_version = LEGACY_PROTOCOL_VERSION
        record = self.sessions.get(instance_id)
        if record is None:
            record = SessionRecord(extension_instance_id=instance_id)
            self.sessions[instance_id] = record
        record.profile_label = _clean_text(payload.get("profile_label"))[:120]
        record.extension_version = (
            _clean_text(payload.get("extension_version")) or "legacy"
        )[:64]
        record.protocol_version = (
            protocol_version
            if protocol_version is not None
            else LEGACY_PROTOCOL_VERSION
        )
        raw_browser = payload.get("browser")
        record.browser = (
            _sanitize_browser_metadata(raw_browser)
            if isinstance(raw_browser, dict)
            else {"name": "Chromium", "version": "", "platform": None}
        )
        record.windows = _normalize_windows(payload)
        record.active_window_id = _as_int(payload.get("active_window_id"))
        record.active_tab_id = _as_int(payload.get("active_tab_id"))
        record.page_url = _clean_text(payload.get("url"))[:4096]
        record.page_title = _clean_text(payload.get("title"))[:500]
        record.last_heartbeat = now
        frame_error = _clean_text(payload.get("frame_error"))
        if frame_error:
            record.last_frame_error = frame_error[:1000]
        self._release_expired_lease(record, now)
        return record

    def heartbeat_response(self, record: SessionRecord) -> dict[str, Any]:
        """Pop only the command queue belonging to the heartbeating session."""
        command = record.command_queue.pop(0) if record.command_queue else None
        return {
            "ok": True,
            "schema_version": SCHEMA_VERSION,
            "protocol_version": PROTOCOL_VERSION,
            "extension_instance_id": record.extension_instance_id,
            "compatible": record.compatible(),
            "required_protocol_version": PROTOCOL_VERSION,
            "cmd": command,
        }

    def list_sessions(self, include_disconnected: bool = True) -> list[dict[str, Any]]:
        """Return stable, non-sensitive session discovery contracts."""
        now = time.monotonic()
        self.expire_stale(now)
        sessions = []
        for record in sorted(
            self.sessions.values(), key=lambda item: item.extension_instance_id
        ):
            if not include_disconnected and not record.alive(now, self.heartbeat_ttl):
                continue
            sessions.append(record.public_contract(now, self.heartbeat_ttl))
        return sessions

    def list_tabs(self, extension_instance_id: str) -> dict[str, Any]:
        """Return windows and tabs for one explicit extension session."""
        record = self.require_session(extension_instance_id)
        return {
            "extension_instance_id": record.extension_instance_id,
            "windows": record.windows,
        }

    def require_session(self, extension_instance_id: str) -> SessionRecord:
        """Resolve a live and compatible session or raise a stable failure."""
        now = time.monotonic()
        self.expire_stale(now)
        record = self.sessions.get(extension_instance_id)
        if record is None:
            raise BrokerError(
                "browser_target_not_found",
                f"browser session not found: {extension_instance_id}",
            )
        if not record.compatible():
            raise BrokerError(
                "incompatible_browser_session",
                "browser extension protocol is incompatible with this broker",
                {
                    "detected_protocol_version": record.protocol_version,
                    "required_protocol_version": PROTOCOL_VERSION,
                },
            )
        if not record.alive(now, self.heartbeat_ttl):
            raise BrokerError(
                "browser_session_disconnected",
                "browser extension session is disconnected; reload the extension or browser profile",
                {"extension_instance_id": extension_instance_id},
            )
        return record

    def resolve_target(
        self,
        raw_target: Any,
    ) -> tuple[SessionRecord, dict[str, Any], bool]:
        """Resolve and validate an explicit target, or the sole active target."""
        explicit = isinstance(raw_target, dict)
        if explicit:
            instance_id = _clean_text(raw_target.get("extension_instance_id"))
            window_id = _as_int(raw_target.get("window_id"))
            tab_id = _as_int(raw_target.get("tab_id"))
            if not instance_id or window_id is None or tab_id is None:
                raise BrokerError(
                    "invalid_browser_operation",
                    "target requires extension_instance_id, window_id, and tab_id",
                )
            record = self.require_session(instance_id)
            for tab in record.iter_tabs():
                if (
                    _as_int(tab.get("id")) == tab_id
                    and _as_int(tab.get("window_id")) == window_id
                ):
                    if tab.get("debuggable") is False:
                        raise BrokerError(
                            "browser_target_not_found",
                            "selected tab cannot be debugged; choose an http(s) or file page",
                        )
                    return record, {
                        "extension_instance_id": instance_id,
                        "window_id": window_id,
                        "tab_id": tab_id,
                    }, True
            raise BrokerError(
                "browser_target_not_found",
                "selected browser window/tab is no longer available",
                {"extension_instance_id": instance_id},
            )

        candidates: list[tuple[SessionRecord, dict[str, Any]]] = []
        now = time.monotonic()
        self.expire_stale(now)
        for record in self.sessions.values():
            if not record.alive(now, self.heartbeat_ttl) or not record.compatible():
                continue
            target = record.active_target()
            if target is not None:
                candidates.append((record, target))
        if not candidates:
            raise BrokerError(
                "browser_unavailable",
                "no live debuggable browser extension target is available",
                {"hint": "install or reload teshi-bridge and open an http(s) tab"},
            )
        if len(candidates) != 1:
            raise BrokerError(
                "ambiguous_browser_target",
                "multiple browser profiles are available; select an explicit session and tab",
                {
                    "candidates": [
                        {
                            "extension_instance_id": target[
                                "extension_instance_id"
                            ],
                            "profile_label": record.profile_label or None,
                            "window_id": target["window_id"],
                            "tab_id": target["tab_id"],
                        }
                        for record, target in candidates
                    ]
                },
            )
        record, target = candidates[0]
        return record, target, False

    def acquire_lease(
        self,
        extension_instance_id: str,
        owner_label: str,
        ttl_secs: Any = DEFAULT_LEASE_TTL_SEC,
    ) -> dict[str, Any]:
        """Acquire an exclusive, renewable session lease."""
        record = self.require_session(extension_instance_id)
        now = time.monotonic()
        self._release_expired_lease(record, now)
        if record.lease is not None:
            raise BrokerError(
                "browser_session_busy",
                "browser session is already leased by another caller",
                record.lease.public_summary(),
            )
        owner = _clean_text(owner_label)[:120] or "external-agent"
        ttl = _bounded_ttl(ttl_secs)
        wall_now = time.time()
        lease = LeaseState(
            token=f"lease_{secrets.token_urlsafe(32)}",
            owner_label=owner,
            acquired_wall_time=wall_now,
            expires_monotonic=now + ttl,
            expires_wall_time=wall_now + ttl,
        )
        record.lease = lease
        return lease.private_result(extension_instance_id)

    def renew_lease(
        self,
        extension_instance_id: str,
        lease_token: str,
        ttl_secs: Any = DEFAULT_LEASE_TTL_SEC,
    ) -> dict[str, Any]:
        """Renew a matching live lease without changing ownership."""
        record = self.require_session(extension_instance_id)
        lease = self.validate_lease(record, lease_token)
        ttl = _bounded_ttl(ttl_secs)
        now = time.monotonic()
        wall_now = time.time()
        lease.expires_monotonic = now + ttl
        lease.expires_wall_time = wall_now + ttl
        return lease.private_result(extension_instance_id)

    def release_lease(
        self,
        extension_instance_id: str,
        lease_token: str,
    ) -> dict[str, Any]:
        """Release a matching lease and make the session immediately available."""
        record = self.require_session(extension_instance_id)
        self.validate_lease(record, lease_token)
        record.lease = None
        return {"extension_instance_id": extension_instance_id, "released": True}

    def validate_lease(self, record: SessionRecord, lease_token: Any) -> LeaseState:
        """Validate the secret token and bounded lifetime for one session."""
        now = time.monotonic()
        lease = record.lease
        if lease is not None and lease.expired(now):
            record.lease = None
            raise BrokerError(
                "expired_browser_lease",
                "browser session lease expired; acquire a new lease",
                {"extension_instance_id": record.extension_instance_id},
            )
        if lease is None or not secrets.compare_digest(
            lease.token, _clean_text(lease_token)
        ):
            raise BrokerError(
                "invalid_browser_lease",
                "a valid lease_token is required for this browser operation",
                {"extension_instance_id": record.extension_instance_id},
            )
        return lease

    def authorize_command(
        self,
        data: dict[str, Any],
        *,
        legacy_compatibility: bool = True,
    ) -> tuple[SessionRecord, dict[str, Any], str | None]:
        """Resolve a command target and enforce exclusive ownership."""
        operation = _clean_text(data.get("cmd"))
        record, target, explicit = self.resolve_target(data.get("target"))
        ephemeral_token: str | None = None
        if operation in MUTATING_COMMANDS:
            supplied = _clean_text(data.get("lease_token"))
            if supplied:
                self.validate_lease(record, supplied)
            elif not explicit and legacy_compatibility:
                now = time.monotonic()
                self._release_expired_lease(record, now)
                if record.lease is not None:
                    raise BrokerError(
                        "browser_session_busy",
                        "the sole browser session is leased; legacy implicit mutation is unavailable",
                        record.lease.public_summary(),
                    )
                wall_now = time.time()
                ephemeral_token = f"lease_{secrets.token_urlsafe(24)}"
                record.lease = LeaseState(
                    token=ephemeral_token,
                    owner_label="legacy-compatibility-adapter",
                    acquired_wall_time=wall_now,
                    expires_monotonic=now + DEFAULT_LEASE_TTL_SEC,
                    expires_wall_time=wall_now + DEFAULT_LEASE_TTL_SEC,
                )
            else:
                raise BrokerError(
                    "invalid_browser_lease",
                    "explicit browser mutation requires a valid lease_token",
                    {"extension_instance_id": record.extension_instance_id},
                )
        return record, target, ephemeral_token

    def queue_command(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        data: dict[str, Any],
        future: asyncio.Future[dict[str, Any]],
        *,
        ephemeral_lease_token: str | None = None,
        front: bool = False,
    ) -> dict[str, Any]:
        """Queue a correlated command only on the selected extension session."""
        request_id = _clean_text(data.get("request_id"))
        operation = _clean_text(data.get("cmd"))
        if not request_id:
            raise BrokerError(
                "invalid_browser_operation", "request_id is required"
            )
        if request_id in self.pending:
            raise BrokerError(
                "invalid_browser_operation", f"duplicate request_id: {request_id}"
            )
        if len(self.pending) >= MAX_PENDING_REQUESTS:
            raise BrokerError(
                "browser_session_busy",
                "browser broker has too many pending requests; retry later",
            )
        if len(record.command_queue) >= MAX_COMMAND_QUEUE:
            raise BrokerError(
                "browser_session_busy",
                "selected browser session command queue is full; retry later",
            )
        command = dict(data)
        command["type"] = "cmd"
        command["schema_version"] = SCHEMA_VERSION
        command["protocol_version"] = PROTOCOL_VERSION
        command["extension_instance_id"] = record.extension_instance_id
        command["target"] = target
        command.pop("lease_token", None)
        pending = PendingRequest(
            request_id=request_id,
            operation=operation,
            target=target,
            extension_instance_id=record.extension_instance_id,
            future=future,
            ephemeral_lease_token=ephemeral_lease_token,
        )
        self.pending[request_id] = pending
        if front:
            record.command_queue.insert(0, command)
        else:
            record.command_queue.append(command)
        return command

    def accept_response(self, payload: dict[str, Any]) -> PendingRequest | None:
        """Deliver only a response matching its request, session, and target."""
        request_id = _clean_text(payload.get("request_id"))
        if not request_id:
            return None
        pending = self.pending.get(request_id)
        if pending is None:
            self._quarantine(payload, "unknown_request_id")
            raise BrokerError(
                "mismatched_browser_response",
                f"no pending browser request matches response {request_id}",
            )
        response_instance = _clean_text(payload.get("extension_instance_id"))
        if not response_instance:
            response_instance = pending.extension_instance_id
        response_target = payload.get("target")
        if response_target is None:
            response_target = pending.target
        if (
            response_instance != pending.extension_instance_id
            or not _targets_equal(response_target, pending.target)
        ):
            self._quarantine(payload, "target_mismatch")
            raise BrokerError(
                "mismatched_browser_response",
                "browser response target does not match its pending request",
            )
        self.pending.pop(request_id, None)
        response = dict(payload)
        response["schema_version"] = SCHEMA_VERSION
        response["operation"] = pending.operation
        response["extension_instance_id"] = pending.extension_instance_id
        response["target"] = pending.target
        if not pending.future.done():
            pending.future.set_result(response)
        self._release_ephemeral_lease(pending)
        return pending

    def cancel_request(self, request_id: str, error: BrokerError) -> None:
        """Fail and remove one pending request and its queued command."""
        pending = self.pending.pop(request_id, None)
        if pending is None:
            return
        record = self.sessions.get(pending.extension_instance_id)
        if record is not None:
            record.command_queue = [
                command
                for command in record.command_queue
                if command.get("request_id") != request_id
            ]
        if not pending.future.done():
            pending.future.set_result(error.response(request_id, pending.operation))
        self._release_ephemeral_lease(pending)

    def expire_stale(self, now: float | None = None) -> None:
        """Expire sessions, pending requests, queues, and leases deterministically."""
        current = time.monotonic() if now is None else now
        for record in list(self.sessions.values()):
            self._release_expired_lease(record, current)
            if record.alive(current, self.heartbeat_ttl):
                continue
            record.command_queue.clear()
            record.lease = None
            for request_id, pending in list(self.pending.items()):
                if pending.extension_instance_id != record.extension_instance_id:
                    continue
                self.pending.pop(request_id, None)
                if not pending.future.done():
                    pending.future.set_result(
                        BrokerError(
                            "browser_session_disconnected",
                            "browser extension disconnected while the operation was pending",
                        ).response(request_id, pending.operation)
                    )
            if current - record.last_heartbeat > DISCONNECTED_RETENTION_SEC:
                self.sessions.pop(record.extension_instance_id, None)

    def update_frame(
        self,
        extension_instance_id: str,
        target: dict[str, Any],
        frame: dict[str, Any],
    ) -> SessionRecord:
        """Store a preview frame only in its originating session."""
        record = self.require_session(extension_instance_id)
        if target.get("extension_instance_id") != extension_instance_id:
            raise BrokerError(
                "mismatched_browser_response",
                "preview frame target does not match its extension session",
            )
        record.latest_frame = dict(frame)
        record.last_frame_at = time.monotonic()
        record.last_frame_error = ""
        record.page_url = _clean_text(frame.get("url")) or record.page_url
        record.active_window_id = _as_int(target.get("window_id"))
        record.active_tab_id = _as_int(target.get("tab_id"))
        return record

    def bridge_info(self) -> dict[str, Any]:
        """Return discovery metadata plus a safe legacy single-session projection."""
        now = time.monotonic()
        contracts = self.list_sessions(include_disconnected=True)
        live = [
            record
            for record in self.sessions.values()
            if record.alive(now, self.heartbeat_ttl) and record.compatible()
        ]
        info: dict[str, Any] = {
            "schema_version": SCHEMA_VERSION,
            "protocol_version": PROTOCOL_VERSION,
            "extension_connected": bool(live),
            "ambiguous_browser_target": len(live) > 1,
            "sessions": contracts,
        }
        if len(live) == 1:
            record = live[0]
            target = record.active_target()
            info.update(
                {
                    "selected_session_id": record.extension_instance_id,
                    "profile_label": record.profile_label or None,
                    "page_url": record.page_url,
                    "title": record.page_title,
                    "active_window_id": record.active_window_id,
                    "active_tab_id": record.active_tab_id,
                    "tabs": record.iter_tabs(),
                    "target": target,
                    "last_frame_error": record.last_frame_error,
                    "last_frame_age_ms": (
                        None
                        if record.last_frame_at is None
                        else int((now - record.last_frame_at) * 1000)
                    ),
                }
            )
        else:
            info.update(
                {
                    "selected_session_id": None,
                    "profile_label": None,
                    "page_url": "",
                    "title": "",
                    "active_window_id": None,
                    "active_tab_id": None,
                    "tabs": [],
                    "target": None,
                    "last_frame_error": "",
                    "last_frame_age_ms": None,
                }
            )
        return info

    def _release_expired_lease(self, record: SessionRecord, now: float) -> None:
        if record.lease is not None and record.lease.expired(now):
            record.lease = None

    def _release_ephemeral_lease(self, pending: PendingRequest) -> None:
        if pending.ephemeral_lease_token is None:
            return
        record = self.sessions.get(pending.extension_instance_id)
        if (
            record is not None
            and record.lease is not None
            and secrets.compare_digest(
                record.lease.token, pending.ephemeral_lease_token
            )
        ):
            record.lease = None

    def _quarantine(self, payload: dict[str, Any], reason: str) -> None:
        entry = {
            "reason": reason,
            "request_id": payload.get("request_id"),
            "extension_instance_id": payload.get("extension_instance_id"),
            "target": payload.get("target"),
        }
        self.quarantined_responses.append(entry)
        if len(self.quarantined_responses) > 32:
            del self.quarantined_responses[:-32]


def generate_playwright_candidates(
    snapshot: dict[str, Any],
    intent: dict[str, Any],
    test_id_attributes: list[str] | None = None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    """Select an intended element and rank stable Playwright locator candidates."""
    attributes = [
        item.strip()
        for item in (test_id_attributes or ["data-testid"])
        if isinstance(item, str) and item.strip()
    ] or ["data-testid"]
    raw_elements = snapshot.get("interactive_elements")
    if not isinstance(raw_elements, list):
        raw_elements = snapshot.get("elements")
    elements = [
        _normalize_element(index, value)
        for index, value in enumerate(raw_elements or [])
        if isinstance(value, dict)
    ]
    if not elements:
        raise BrokerError(
            "browser_target_not_found",
            "page snapshot contains no interactive elements for locator acquisition",
        )
    explicitly_matching = [
        element for element in elements if _matches_explicit_intent(element, intent)
    ]
    if not explicitly_matching:
        raise BrokerError(
            "browser_target_not_found",
            "locator role, text, or element reference did not match an interactive element in the selected page",
        )
    ranked_elements = sorted(
        ((score_intent(element, intent), element) for element in explicitly_matching),
        key=lambda pair: pair[0],
        reverse=True,
    )
    best_score, element = ranked_elements[0]
    has_intent = any(
        _clean_text(intent.get(field))
        for field in ("purpose", "text", "role", "element_ref", "gherkin_step")
    )
    if has_intent and best_score <= 0:
        raise BrokerError(
            "browser_target_not_found",
            "locator intent did not match an interactive element in the selected page",
        )

    candidates: list[dict[str, Any]] = []
    role = element.get("role") or _implicit_role(element)
    accessible_name = (
        element.get("accessible_name")
        or element.get("ariaLabel")
        or element.get("label")
        or element.get("text")
    )
    context = element.get("context") or {}
    if role and accessible_name:
        candidates.append(
            _candidate(
                "role",
                {"role": role, "name": accessible_name, "exact": True},
                f"page.getByRole({_js_string(role)}, {{ name: {_js_string(accessible_name)}, exact: true }})",
                100,
                "unique accessible role and name",
                context,
            )
        )
    label = element.get("label")
    if label:
        candidates.append(
            _candidate(
                "label",
                {"text": label, "exact": True},
                f"page.getByLabel({_js_string(label)}, {{ exact: true }})",
                90,
                "associated form label",
                context,
            )
        )
    placeholder = element.get("placeholder")
    if placeholder:
        candidates.append(
            _candidate(
                "placeholder",
                {"text": placeholder, "exact": True},
                f"page.getByPlaceholder({_js_string(placeholder)}, {{ exact: true }})",
                85,
                "stable placeholder text",
                context,
            )
        )
    element_attributes = element.get("attributes", {})
    for attribute in attributes:
        value = _clean_text(element_attributes.get(attribute))
        if not value:
            continue
        if attribute == "data-testid":
            expression = f"page.getByTestId({_js_string(value)})"
            kind = "test_id"
        else:
            selector = f"[{attribute}={json.dumps(value)}]"
            expression = f"page.locator({_js_string(selector)})"
            kind = "attribute"
        candidates.append(
            _candidate(
                kind,
                {"attribute": attribute, "value": value},
                expression,
                80,
                f"project-configured test-id attribute {attribute}",
                context,
            )
        )
    stable_attributes = ["id", "name", "aria-label", "title", "alt"]
    for attribute in stable_attributes:
        value = _clean_text(element_attributes.get(attribute))
        if not value:
            continue
        selector = f"[{attribute}={json.dumps(value)}]"
        candidates.append(
            _candidate(
                "attribute",
                {"attribute": attribute, "value": value},
                f"page.locator({_js_string(selector)})",
                70 if attribute == "id" else 65,
                f"stable {attribute} attribute fallback",
                context,
            )
        )
    short_selector = _clean_text(
        element.get("shortSelector") or element.get("short_selector")
    )
    if short_selector:
        warnings = _selector_warnings(short_selector)
        candidates.append(
            _candidate(
                "css",
                {"selector": short_selector},
                f"page.locator({_js_string(short_selector)})",
                55 - 8 * len(warnings),
                "CSS fallback derived from the current DOM",
                context,
                warnings,
            )
        )
    text = _clean_text(element.get("text"))
    if text:
        candidates.append(
            _candidate(
                "text",
                {"text": text, "exact": True},
                f"page.getByText({_js_string(text)}, {{ exact: true }})",
                45,
                "visible text fallback may change with copy or localization",
                context,
                ["text_content_may_change"],
            )
        )
    if not candidates:
        raise BrokerError(
            "browser_target_not_found",
            "the intended element has no supported stable locator attributes",
        )
    deduplicated: dict[str, dict[str, Any]] = {}
    for candidate in candidates:
        expression = candidate["expression"]
        existing = deduplicated.get(expression)
        if existing is None or candidate["score"] > existing["score"]:
            deduplicated[expression] = candidate
    ordered = sorted(
        deduplicated.values(), key=lambda candidate: candidate["score"], reverse=True
    )
    return element, ordered


def score_intent(element: dict[str, Any], intent: dict[str, Any]) -> int:
    """Score how well one normalized element matches caller-provided intent."""
    score = 0
    expected_ref = _clean_text(intent.get("element_ref"))
    if expected_ref:
        return 10_000 if element.get("element_ref") == expected_ref else -10_000
    expected_role = _clean_text(intent.get("role")).casefold()
    actual_role = _clean_text(element.get("role") or _implicit_role(element)).casefold()
    if expected_role:
        score += 120 if actual_role == expected_role else -60
    expected_text = _clean_text(intent.get("text")).casefold()
    haystack = " ".join(
        _clean_text(element.get(field))
        for field in (
            "accessible_name",
            "ariaLabel",
            "label",
            "placeholder",
            "text",
        )
    ).casefold()
    if expected_text:
        if expected_text == haystack.strip():
            score += 160
        elif expected_text in haystack:
            score += 100
        else:
            score -= 50
    context_words = _intent_words(
        " ".join(
            [
                _clean_text(intent.get("purpose")),
                _clean_text(intent.get("gherkin_step")),
            ]
        )
    )
    if context_words:
        element_words = _intent_words(f"{actual_role} {haystack}")
        score += 8 * len(context_words & element_words)
    if element.get("visible") is False:
        score -= 80
    return score


def _matches_explicit_intent(
    element: dict[str, Any], intent: dict[str, Any]
) -> bool:
    """Require every structured identity field supplied by the caller to match."""
    expected_ref = _clean_text(intent.get("element_ref"))
    if expected_ref and element.get("element_ref") != expected_ref:
        return False
    expected_role = _clean_text(intent.get("role")).casefold()
    actual_role = _clean_text(element.get("role") or _implicit_role(element)).casefold()
    if expected_role and actual_role != expected_role:
        return False
    expected_text = _clean_text(intent.get("text")).casefold()
    if expected_text:
        text_fields = (
            "accessible_name",
            "ariaLabel",
            "label",
            "placeholder",
            "text",
        )
        if not any(
            expected_text in _clean_text(element.get(field)).casefold()
            for field in text_fields
        ):
            return False
    return True


def apply_verification_results(
    candidates: list[dict[str, Any]],
    verification: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    """Merge browser-observed counts and actionability into ranked candidates."""
    by_expression = {
        _clean_text(item.get("expression")): item
        for item in verification
        if isinstance(item, dict)
    }
    merged: list[dict[str, Any]] = []
    for candidate in candidates:
        result = by_expression.get(candidate["expression"], {})
        count = max(0, _as_int(result.get("match_count")) or 0)
        visible = bool(result.get("visible"))
        enabled = bool(result.get("enabled"))
        if result.get("stale_page_context"):
            status = "stale_page_context"
        elif count == 0:
            status = "not_found"
        elif count > 1:
            status = "ambiguous"
        elif not visible or not enabled:
            status = "not_actionable"
        else:
            status = "verified"
        updated = dict(candidate)
        updated.update(
            {
                "match_count": count,
                "visible": visible,
                "enabled": enabled,
                "verification": status,
            }
        )
        merged.append(updated)
    return sorted(
        merged,
        key=lambda candidate: (
            candidate.get("verification") == "verified",
            candidate.get("score", 0),
        ),
        reverse=True,
    )


def operation_success(
    operation: str,
    request_id: str,
    **fields: Any,
) -> dict[str, Any]:
    """Build the shared successful operation envelope."""
    return {
        "type": "response",
        "schema_version": SCHEMA_VERSION,
        "operation": operation,
        "request_id": request_id,
        "ok": True,
        **fields,
    }


def _normalize_windows(payload: dict[str, Any]) -> list[dict[str, Any]]:
    raw_windows = payload.get("windows")
    normalized: list[dict[str, Any]] = []
    if isinstance(raw_windows, list):
        for raw_window in raw_windows:
            if not isinstance(raw_window, dict):
                continue
            window_id = _as_int(raw_window.get("id"))
            if window_id is None:
                continue
            normalized.append(
                {
                    "id": window_id,
                    "focused": bool(raw_window.get("focused")),
                    "tabs": _normalize_tabs(raw_window.get("tabs"), window_id),
                }
            )
    if normalized:
        return normalized
    legacy_window_id = _as_int(payload.get("active_window_id")) or 0
    tabs = _normalize_tabs(payload.get("tabs"), legacy_window_id)
    return [
        {
            "id": legacy_window_id,
            "focused": True,
            "tabs": tabs,
        }
    ]


def _normalize_tabs(raw_tabs: Any, window_id: int) -> list[dict[str, Any]]:
    tabs: list[dict[str, Any]] = []
    if not isinstance(raw_tabs, list):
        return tabs
    for raw_tab in raw_tabs:
        if not isinstance(raw_tab, dict):
            continue
        tab_id = _as_int(raw_tab.get("id"))
        if tab_id is None:
            continue
        tabs.append(
            {
                "id": tab_id,
                "window_id": _as_int(raw_tab.get("window_id")) or window_id,
                "title": _clean_text(raw_tab.get("title"))[:500],
                "url": _clean_text(raw_tab.get("url"))[:4096],
                "active": bool(raw_tab.get("active")),
                "favIconUrl": _clean_text(raw_tab.get("favIconUrl"))[:4096],
                "debuggable": raw_tab.get("debuggable") is not False,
            }
        )
    return tabs


def _sanitize_browser_metadata(raw: dict[str, Any]) -> dict[str, Any]:
    return {
        "name": (_clean_text(raw.get("name")) or "Chromium")[:80],
        "version": _clean_text(raw.get("version"))[:80],
        "platform": _clean_text(raw.get("platform"))[:80] or None,
    }


def _normalize_element(index: int, raw: dict[str, Any]) -> dict[str, Any]:
    element = dict(raw)
    element["element_ref"] = _clean_text(
        raw.get("element_ref") or raw.get("ref")
    ) or f"e{index + 1}"
    attributes: dict[str, str] = {}
    raw_attributes = raw.get("attributes") or raw.get("allAttributes")
    if isinstance(raw_attributes, dict):
        for name, value in raw_attributes.items():
            if isinstance(name, str) and isinstance(value, (str, int, float, bool)):
                attributes[name] = str(value)[:500]
    aliases = {
        "id": "id",
        "name": "name",
        "testId": "data-testid",
        "testid": "data-testid",
        "ariaLabel": "aria-label",
        "title": "title",
        "alt": "alt",
    }
    for source, destination in aliases.items():
        value = _clean_text(raw.get(source))
        if value and destination not in attributes:
            attributes[destination] = value[:500]
    element["attributes"] = attributes
    element["accessible_name"] = _clean_text(
        raw.get("accessible_name") or raw.get("computedAccessibleName")
    ) or None
    for field in (
        "tag",
        "role",
        "ariaLabel",
        "label",
        "placeholder",
        "text",
        "shortSelector",
    ):
        if field in element:
            element[field] = _clean_text(element[field]) or None
    return element


def _implicit_role(element: dict[str, Any]) -> str:
    tag = _clean_text(element.get("tag")).lower()
    attributes = element.get("attributes", {})
    input_type = _clean_text(attributes.get("type")).lower()
    if tag == "button" or (tag == "input" and input_type in {"button", "submit"}):
        return "button"
    if tag == "a":
        return "link"
    if tag == "textarea":
        return "textbox"
    if tag == "input":
        if input_type == "checkbox":
            return "checkbox"
        if input_type == "radio":
            return "radio"
        return "textbox"
    if tag == "select":
        return "combobox"
    return ""


def _candidate(
    kind: str,
    arguments: dict[str, Any],
    expression: str,
    score: int,
    rationale: str,
    context: dict[str, Any],
    warnings: list[str] | None = None,
) -> dict[str, Any]:
    return {
        "kind": kind,
        "arguments": arguments,
        "expression": expression,
        "context": context,
        "match_count": 0,
        "visible": False,
        "enabled": False,
        "verification": "unverified",
        "score": score,
        "stability_rationale": rationale,
        "warnings": warnings or [],
    }


def _selector_warnings(selector: str) -> list[str]:
    warnings: list[str] = []
    lowered = selector.lower()
    if ":nth-" in lowered or ":first" in lowered or ":last" in lowered:
        warnings.append("positional_selector")
    if selector.count(">") >= 3 or len(selector) > 160:
        warnings.append("long_dom_path")
    if any(
        marker in selector
        for marker in ("sc-", "__", "css-", "emotion-", "_ngcontent")
    ):
        warnings.append("generated_class")
    if any(marker in lowered for marker in ("x=", "y=", "coordinate")):
        warnings.append("coordinate_selector")
    return warnings


def _targets_equal(left: Any, right: Any) -> bool:
    if not isinstance(left, dict) or not isinstance(right, dict):
        return False
    return (
        _clean_text(left.get("extension_instance_id"))
        == _clean_text(right.get("extension_instance_id"))
        and _as_int(left.get("window_id")) == _as_int(right.get("window_id"))
        and _as_int(left.get("tab_id")) == _as_int(right.get("tab_id"))
    )


def _bounded_ttl(value: Any) -> int:
    parsed = _as_int(value) or DEFAULT_LEASE_TTL_SEC
    return max(MIN_LEASE_TTL_SEC, min(MAX_LEASE_TTL_SEC, parsed))


def _as_int(value: Any) -> int | None:
    try:
        if value is None or isinstance(value, bool):
            return None
        return int(value)
    except (TypeError, ValueError):
        return None


def _clean_text(value: Any) -> str:
    return str(value).strip() if value is not None else ""


def _js_string(value: str) -> str:
    encoded = json.dumps(value, ensure_ascii=False)
    return encoded.replace("</", "<\\/")


def _intent_words(value: str) -> set[str]:
    normalized = "".join(
        char.lower() if char.isalnum() else " " for char in value
    )
    return {word for word in normalized.split() if len(word) >= 2}
