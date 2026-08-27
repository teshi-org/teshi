"""Versioned multi-session broker contracts for Teshi browser agents."""

from __future__ import annotations

import asyncio
import base64
import getpass
import hashlib
import json
import os
import re
import secrets
import time
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import parse_qsl, urlencode, urlsplit, urlunsplit

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
MAX_ELEMENT_REFERENCES_PER_SESSION = 512
ELEMENT_REFERENCE_TTL_SEC = 120.0
DEFAULT_CONSOLE_MAX_AGE_MS = 300_000
DEFAULT_CONSOLE_MAX_ENTRIES = 500
DEFAULT_CONSOLE_MAX_BYTES = 1_048_576
MAX_CONSOLE_MAX_AGE_MS = 3_600_000
MAX_CONSOLE_MAX_ENTRIES = 5_000
MAX_CONSOLE_MAX_BYTES = 8_388_608
MAX_CONSOLE_EVENT_TEXT_BYTES = 16_384
KNOWN_CONSOLE_LEVELS = {"debug", "log", "info", "warn", "error"}
DEFAULT_NETWORK_MAX_AGE_MS = 300_000
DEFAULT_NETWORK_MAX_ENTRIES = 1_000
DEFAULT_NETWORK_MAX_BYTES = 2_097_152
DEFAULT_NETWORK_MAX_BODY_BYTES = 262_144
MAX_NETWORK_MAX_AGE_MS = 3_600_000
MAX_NETWORK_MAX_ENTRIES = 10_000
MAX_NETWORK_MAX_BYTES = 16_777_216
MAX_NETWORK_MAX_BODY_BYTES = 2_097_152
MAX_NETWORK_BATCH_EVENTS = 100
MAX_NETWORK_BATCH_BYTES = 4_194_304
MAX_NETWORK_PENDING_EVENTS = 2_000
DEFAULT_SENSITIVE_DIAGNOSTIC_FIELDS = {
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "token",
    "access-token",
    "refresh-token",
    "password",
    "passwd",
    "secret",
}
REDACTION_MARKER = "[REDACTED]"
DEFAULT_CAPABILITY_GRANT_TTL_SEC = 300
MIN_CAPABILITY_GRANT_TTL_SEC = 30
MAX_CAPABILITY_GRANT_TTL_SEC = 3600
MAX_PRIVILEGED_AUDIT_RECORDS = 1000
PRIVILEGED_CAPABILITIES = {
    "javascript",
    "raw-cdp",
    "cookies",
    "cookie-values",
    "content-settings",
    "extension-management",
}

KNOWN_FEATURE_IDS = {
    "p0.control",
    "p1.observability_artifacts",
    "p1.filtered_network_capture",
    "p1.network_batch_transport",
    "p2.javascript",
    "p2.raw_cdp",
    "p2.cookies",
    "p2.content_settings",
    "p2.extension_management",
}
FILTERED_NETWORK_CAPTURE_FEATURES = (
    "p1.filtered_network_capture",
    "p1.network_batch_transport",
)
KNOWN_ACTIONS = {
    "click",
    "pointer_click",
    "fill",
    "type",
    "select",
    "press_key",
    "assert_visible",
    "assert_text",
    "navigate",
    "go_back",
    "upload",
}
KNOWN_BROWSER_OPERATIONS = {
    "capture_browser_screenshot",
    "generate_browser_pdf",
    "start_console_capture",
    "list_console_events",
    "clear_console_capture",
    "stop_console_capture",
    "start_network_capture",
    "list_network_requests",
    "get_network_request_detail",
    "clear_network_capture",
    "stop_network_capture",
    "execute_privileged_javascript",
    "execute_privileged_cdp",
    "list_browser_cookies",
    "access_browser_content_setting",
    "list_browser_extensions",
}
P1_BROWSER_OPERATIONS = {
    "capture_browser_screenshot",
    "generate_browser_pdf",
    "start_console_capture",
    "list_console_events",
    "clear_console_capture",
    "stop_console_capture",
    "start_network_capture",
    "list_network_requests",
    "get_network_request_detail",
    "clear_network_capture",
    "stop_network_capture",
}
P2_BROWSER_OPERATION_FEATURES = {
    "execute_privileged_javascript": "p2.javascript",
    "execute_privileged_cdp": "p2.raw_cdp",
    "list_browser_cookies": "p2.cookies",
    "access_browser_content_setting": "p2.content_settings",
    "list_browser_extensions": "p2.extension_management",
}

MUTATING_COMMANDS = {
    "get_page_snapshot",
    "resolve_playwright_locator",
    "verify_playwright_locator",
    "verify_playwright_locators",
    "capture_browser_evidence",
    "highlight_selector",
    "clear_highlight",
    "execute_locator",
    "execute_browser_action",
    "heal_execute_locator",
    "enhance_locator",
    "navigate",
    "activate_tab",
    "open_tab",
    "close_tab",
    "create_window",
    "group_tabs",
    "open_project",
    "start_console_capture",
    "clear_console_capture",
    "stop_console_capture",
    "start_network_capture",
    "clear_network_capture",
    "stop_network_capture",
}

LEASE_REQUIRED_COMMANDS = MUTATING_COMMANDS | {
    "list_console_events",
    "list_network_requests",
    "get_network_request_detail",
    "execute_privileged_javascript",
    "execute_privileged_cdp",
    "list_browser_cookies",
    "access_browser_content_setting",
    "list_browser_extensions",
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
            response["recovery"] = _sanitize_public_value(self.recovery)
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
class ElementReferenceRecord:
    """Revision-bound presentation alias isolated to one complete target."""

    alias: str
    target: dict[str, Any]
    snapshot_id: str
    page_context_revision: str
    context: dict[str, Any]
    element: dict[str, Any]
    created_at: float


@dataclass
class ConsoleEventRecord:
    """One bounded console event retained with a monotonic eviction clock."""

    value: dict[str, Any]
    created_at: float
    byte_size: int


@dataclass
class ConsoleCaptureState:
    """Target-scoped console capture state owned by one extension session."""

    target: dict[str, Any]
    levels: set[str]
    max_age_ms: int
    max_entries: int
    max_bytes: int
    sensitive_fields: set[str]
    events: list[ConsoleEventRecord] = field(default_factory=list)
    byte_size: int = 0


@dataclass
class NetworkRequestRecord:
    """One request/response metadata record retained without a response body."""

    value: dict[str, Any]
    created_at: float
    byte_size: int


@dataclass
class NetworkCaptureState:
    """Target-scoped bounded network capture configuration and metadata."""

    target: dict[str, Any]
    capture_id: str
    allowed_hostnames: tuple[str, ...]
    capture_request_bodies: bool
    max_request_body_bytes: int
    max_age_ms: int
    max_entries: int
    max_bytes: int
    max_body_bytes: int
    sensitive_fields: set[str]
    requests: dict[str, NetworkRequestRecord] = field(default_factory=dict)
    byte_size: int = 0
    active: bool = True
    acknowledged_sequence: int = 0
    highest_seen_sequence: int = 0
    pending_events: dict[int, dict[str, Any]] = field(default_factory=dict)
    clear_sequence: int = 0
    dropped_events: int = 0
    dropped_batches: int = 0
    dropped_bytes: int = 0
    rejected_events: int = 0
    duplicate_events: int = 0
    termination_reason: str | None = None
    terminated_at_ms: int | None = None


@dataclass
class CapabilityGrant:
    """Short-lived privileged authorization bound to one broker and Profile."""

    grant_id: str
    token_hash: str
    capability: str
    extension_instance_id: str
    project_root: str
    caller_label: str
    local_user: str
    broker_instance_id: str
    issued_wall_time: float
    expires_monotonic: float
    expires_wall_time: float
    revoked: bool = False

    def public_summary(self) -> dict[str, Any]:
        """Return non-reusable grant metadata."""
        return {
            "grant_id": self.grant_id,
            "capability": self.capability,
            "extension_instance_id": self.extension_instance_id,
            "project_root": self.project_root,
            "caller_label": self.caller_label,
            "issued_at_ms": int(self.issued_wall_time * 1000),
            "expires_at_ms": int(self.expires_wall_time * 1000),
            "revoked": self.revoked,
        }


@dataclass
class SessionRecord:
    """Mutable broker state for one extension installation."""

    extension_instance_id: str
    profile_label: str = ""
    profile_label_managed: bool = False
    extension_version: str = "legacy"
    protocol_version: int = LEGACY_PROTOCOL_VERSION
    browser: dict[str, Any] = field(default_factory=dict)
    features: list[dict[str, Any]] = field(default_factory=list)
    supported_actions: list[str] = field(default_factory=list)
    supported_operations: list[str] = field(default_factory=list)
    optional_permissions: dict[str, bool] = field(default_factory=dict)
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
    element_references: dict[str, ElementReferenceRecord] = field(
        default_factory=dict
    )
    console_captures: dict[str, ConsoleCaptureState] = field(default_factory=dict)
    network_captures: dict[str, NetworkCaptureState] = field(default_factory=dict)
    network_capture_barriers: dict[str, int] = field(default_factory=dict)

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
            "capabilities": {
                "features": self.features,
                "supported_actions": self.supported_actions,
                "supported_operations": self.supported_operations,
                "optional_permissions": self.optional_permissions,
            },
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

    def __init__(
        self,
        heartbeat_ttl: float = DEFAULT_HEARTBEAT_TTL_SEC,
        *,
        broker_instance_id: str | None = None,
        local_user: str | None = None,
    ) -> None:
        self.heartbeat_ttl = heartbeat_ttl
        self.broker_instance_id = broker_instance_id or secrets.token_hex(16)
        self.local_user = (local_user or getpass.getuser() or "local-user")[:120]
        self.sessions: dict[str, SessionRecord] = {}
        self.pending: dict[str, PendingRequest] = {}
        self.quarantined_responses: list[dict[str, Any]] = []
        self.capability_grants: dict[str, CapabilityGrant] = {}
        self.privileged_audit: list[dict[str, Any]] = []

    def create_capability_grant(
        self,
        *,
        extension_instance_id: str,
        lease_token: str,
        capability: Any,
        project_root: Any,
        caller_label: Any,
        ttl_secs: Any = DEFAULT_CAPABILITY_GRANT_TTL_SEC,
        interactive_confirmed: bool = False,
        non_interactive: bool = False,
        acknowledged_capability: Any = "",
        policy_capabilities: set[str] | None = None,
    ) -> dict[str, Any]:
        """Issue one explicit grant after lease, confirmation, and policy gates."""
        record = self.require_session(extension_instance_id)
        self.validate_lease(record, lease_token)
        name = _clean_text(capability).lower()
        if name not in PRIVILEGED_CAPABILITIES:
            raise BrokerError(
                "invalid_browser_operation",
                "unknown privileged browser capability",
                {"supported_capabilities": sorted(PRIVILEGED_CAPABILITIES)},
            )
        project = _canonical_project_root(project_root)
        caller = _clean_text(caller_label)[:120]
        if not project or not caller:
            raise BrokerError(
                "invalid_browser_operation",
                "project_root and caller_label are required for a capability grant",
            )
        if non_interactive:
            if _clean_text(acknowledged_capability).lower() != name:
                raise BrokerError(
                    "browser_capability_denied",
                    "non-interactive grant requires an exact capability acknowledgement",
                )
            if policy_capabilities is None or name not in policy_capabilities:
                raise BrokerError(
                    "browser_capability_denied",
                    "effective browser policy denies this non-interactive capability",
                    {"capability": name, "policy": "default-deny"},
                )
        elif not interactive_confirmed:
            raise BrokerError(
                "browser_capability_denied",
                "interactive capability grant requires explicit confirmation",
                {"capability": name},
            )
        ttl = _bounded_capability_ttl(ttl_secs)
        now = time.monotonic()
        wall_now = time.time()
        token = f"grant_{secrets.token_urlsafe(32)}"
        grant_id = f"cap_{secrets.token_hex(12)}"
        grant = CapabilityGrant(
            grant_id=grant_id,
            token_hash=_token_hash(token),
            capability=name,
            extension_instance_id=record.extension_instance_id,
            project_root=project,
            caller_label=caller,
            local_user=self.local_user,
            broker_instance_id=self.broker_instance_id,
            issued_wall_time=wall_now,
            expires_monotonic=now + ttl,
            expires_wall_time=wall_now + ttl,
        )
        self.capability_grants[grant_id] = grant
        return {**grant.public_summary(), "grant_token": token}

    def list_capability_grants(
        self, *, project_root: Any, extension_instance_id: Any = ""
    ) -> list[dict[str, Any]]:
        """List current grant metadata without reusable tokens."""
        self.expire_capability_grants()
        project = _canonical_project_root(project_root)
        instance = _clean_text(extension_instance_id)
        return [
            grant.public_summary()
            for grant in self.capability_grants.values()
            if grant.project_root == project
            and (not instance or grant.extension_instance_id == instance)
        ]

    def revoke_capability_grant(self, grant_id: Any, *, project_root: Any) -> dict[str, Any]:
        """Revoke one project-bound grant by its non-secret identifier."""
        grant = self.capability_grants.get(_clean_text(grant_id))
        if grant is None or grant.project_root != _canonical_project_root(project_root):
            raise BrokerError("browser_capability_denied", "capability grant is unavailable")
        grant.revoked = True
        return {"grant_id": grant.grant_id, "revoked": True}

    def validate_capability_grant(
        self,
        *,
        token: Any,
        capability: Any,
        extension_instance_id: Any,
        project_root: Any,
        caller_label: Any,
    ) -> CapabilityGrant:
        """Validate every binding of one privileged grant."""
        name = _clean_text(capability).lower()
        token_hash = _token_hash(_clean_text(token))
        now = time.monotonic()
        candidates = [
            grant
            for grant in self.capability_grants.values()
            if secrets.compare_digest(grant.token_hash, token_hash)
        ]
        if not candidates:
            raise BrokerError("browser_capability_denied", "a valid capability grant is required")
        grant = candidates[0]
        if grant.revoked:
            raise BrokerError("browser_capability_denied", "capability grant was revoked")
        if now >= grant.expires_monotonic:
            self.capability_grants.pop(grant.grant_id, None)
            raise BrokerError("browser_capability_denied", "capability grant expired")
        bindings_match = (
            grant.capability == name
            and grant.extension_instance_id == _clean_text(extension_instance_id)
            and grant.project_root == _canonical_project_root(project_root)
            and grant.caller_label == _clean_text(caller_label)[:120]
            and grant.local_user == self.local_user
            and grant.broker_instance_id == self.broker_instance_id
        )
        if not bindings_match:
            raise BrokerError("browser_capability_denied", "capability grant scope does not match this request")
        return grant

    def expire_capability_grants(self, now: float | None = None) -> None:
        """Remove expired grants without retaining their secret hashes."""
        current = time.monotonic() if now is None else now
        for grant_id, grant in list(self.capability_grants.items()):
            if current >= grant.expires_monotonic:
                self.capability_grants.pop(grant_id, None)

    def append_privileged_audit(
        self,
        *,
        capability: Any,
        caller_label: Any,
        target: dict[str, Any] | None,
        request_id: Any,
        outcome: Any,
        arguments: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Retain one bounded metadata-only privileged audit record."""
        record = {
            "timestamp_ms": int(time.time() * 1000),
            "capability": _clean_text(capability)[:80],
            "caller_label": _clean_text(caller_label)[:120],
            "target": _sanitize_public_value(target or {}),
            "request_id": _clean_text(request_id)[:160],
            "outcome": _clean_text(outcome)[:80],
            "arguments": _sanitize_public_value(
                _redact_mapping(
                    arguments or {}, set(DEFAULT_SENSITIVE_DIAGNOSTIC_FIELDS)
                )
            ),
        }
        self.privileged_audit.append(record)
        del self.privileged_audit[:-MAX_PRIVILEGED_AUDIT_RECORDS]
        return record

    def list_privileged_audit(self, limit: Any = 100) -> list[dict[str, Any]]:
        """Return newest bounded audit metadata."""
        bounded = max(1, min(int(limit or 100), MAX_PRIVILEGED_AUDIT_RECORDS))
        return list(self.privileged_audit[-bounded:])

    @staticmethod
    def require_optional_permission(record: SessionRecord, permission: Any) -> None:
        """Fail closed until the extension advertises the exact optional permission."""
        name = _clean_text(permission)
        if not record.optional_permissions.get(name, False):
            raise BrokerError(
                "browser_capability_unavailable",
                "required Chromium optional permission is not approved",
                {"permission": name, "approval": "extension-popup"},
            )

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
        if not record.profile_label_managed:
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
        record.features = _sanitize_feature_availability(payload.get("features"))
        record.supported_actions = _sanitize_supported_actions(
            payload.get("supported_actions")
        )
        record.supported_operations = _sanitize_supported_operations(
            payload.get("supported_operations")
        )
        record.optional_permissions = _sanitize_optional_permissions(
            payload.get("optional_permissions")
        )
        previous_urls = {
            (_as_int(tab.get("window_id")), _as_int(tab.get("id"))): _clean_text(
                tab.get("url")
            )
            for tab in record.iter_tabs()
        }
        record.windows = _normalize_windows(payload)
        current_tabs = record.iter_tabs()
        current_keys = {
            (_as_int(tab.get("window_id")), _as_int(tab.get("id")))
            for tab in current_tabs
        }
        for (window_id, tab_id), previous_url in previous_urls.items():
            current_url = next(
                (
                    _clean_text(tab.get("url"))
                    for tab in current_tabs
                    if _as_int(tab.get("window_id")) == window_id
                    and _as_int(tab.get("id")) == tab_id
                ),
                "",
            )
            if (window_id, tab_id) not in current_keys or (
                previous_url and current_url and previous_url != current_url
            ):
                self.clear_element_references(
                    record,
                    target={
                        "extension_instance_id": record.extension_instance_id,
                        "window_id": window_id,
                        "tab_id": tab_id,
                    },
                )
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
            "command_transports": ["direct-ws", "heartbeat"],
            "accepted_features": record.features,
            "cmd": command,
        }

    def lookup_sessions(
        self,
        *,
        extension_instance_id: str = "",
        profile_label: str = "",
        browser_name: str = "",
        tab_id: int | None = None,
    ) -> list[dict[str, Any]]:
        """Find sessions by typed public identity without assuming tab IDs are global."""
        matches: list[dict[str, Any]] = []
        for record in self.sessions.values():
            if extension_instance_id and record.extension_instance_id != extension_instance_id:
                continue
            if profile_label and record.profile_label != profile_label:
                continue
            if browser_name and _clean_text(record.browser.get("name")).casefold() != browser_name.casefold():
                continue
            tabs = record.iter_tabs()
            if tab_id is not None and not any(_as_int(tab.get("id")) == tab_id for tab in tabs):
                continue
            matches.append(record.public_contract(time.monotonic(), self.heartbeat_ttl))
        return matches

    def set_profile_label(self, extension_instance_id: str, label: str) -> str:
        """Set a unique live display alias while opaque identity remains authoritative."""
        record = self.require_session(extension_instance_id)
        normalized = _clean_text(label)[:120]
        if not normalized:
            raise BrokerError("invalid_browser_operation", "profile label is required")
        now = time.monotonic()
        duplicate = next(
            (
                item
                for item in self.sessions.values()
                if item.extension_instance_id != extension_instance_id
                and item.alive(now, self.heartbeat_ttl)
                and item.profile_label.casefold() == normalized.casefold()
            ),
            None,
        )
        if duplicate is not None:
            raise BrokerError(
                "ambiguous_browser_target",
                f"profile label is already used by another live session: {normalized}",
                {"profile_label": normalized},
            )
        record.profile_label = normalized
        record.profile_label_managed = True
        return normalized

    def clear_profile_label(self, extension_instance_id: str) -> None:
        record = self.require_session(extension_instance_id)
        record.profile_label = ""
        record.profile_label_managed = True

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
        required_features: list[str] = []
        required_feature = _clean_text(data.get("required_feature"))
        if operation in P1_BROWSER_OPERATIONS:
            required_features.append("p1.observability_artifacts")
            if operation == "start_network_capture":
                required_features.extend(FILTERED_NETWORK_CAPTURE_FEATURES)
        elif operation in P2_BROWSER_OPERATION_FEATURES:
            required_features.append(P2_BROWSER_OPERATION_FEATURES[operation])
        elif required_feature:
            required_features.append(required_feature)
        for required_feature in required_features:
            availability = next(
                (
                    item
                    for item in record.features
                    if item.get("feature") == required_feature
                ),
                None,
            )
            if availability is None or not availability.get("available"):
                recovery: dict[str, Any] = {"required_feature": required_feature}
                if availability and availability.get("reason"):
                    recovery["reason"] = availability["reason"]
                raise BrokerError(
                    "browser_capability_unavailable",
                    f"selected browser session does not provide {required_feature}",
                    recovery,
                )
        if operation in (P1_BROWSER_OPERATIONS | set(P2_BROWSER_OPERATION_FEATURES)) and operation not in record.supported_operations:
            raise BrokerError(
                "browser_capability_unavailable",
                f"selected browser session does not advertise operation {operation}",
                {
                    "operation": operation,
                    "supported_operations": record.supported_operations,
                },
            )
        ephemeral_token: str | None = None
        if operation in LEASE_REQUIRED_COMMANDS:
            supplied = _clean_text(data.get("lease_token"))
            if supplied:
                self.validate_lease(record, supplied)
            elif operation in MUTATING_COMMANDS and not explicit and legacy_compatibility:
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
                    "browser operation requires a valid lease_token",
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
                "duplicate_browser_mutation"
                if operation in MUTATING_COMMANDS
                else "invalid_browser_operation",
                f"duplicate request_id: {request_id}",
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

    def cache_snapshot_references(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        response: dict[str, Any],
    ) -> None:
        """Publish deterministic aliases while retaining opaque locator recipes."""
        raw_elements = response.get("interactive_elements")
        if not isinstance(raw_elements, list):
            return
        now = time.monotonic()
        snapshot_id = _clean_text(response.get("snapshot_id")) or _clean_text(
            response.get("request_id")
        )
        revision = _clean_text(response.get("page_context_revision"))
        response["snapshot_id"] = snapshot_id
        target_prefix = _reference_target_prefix(target)
        self.clear_element_references(record, target=target)
        published: list[dict[str, Any]] = []
        for index, raw in enumerate(raw_elements[:MAX_ELEMENT_REFERENCES_PER_SESSION]):
            if not isinstance(raw, dict):
                continue
            alias = f"@e{len(published) + 1}"
            element = dict(raw)
            context = element.get("context")
            normalized_context = dict(context) if isinstance(context, dict) else {}
            record.element_references[f"{target_prefix}:{alias}"] = (
                ElementReferenceRecord(
                    alias=alias,
                    target=dict(target),
                    snapshot_id=snapshot_id,
                    page_context_revision=revision,
                    context=normalized_context,
                    element=element,
                    created_at=now,
                )
            )
            element["ref"] = alias
            element["snapshot_id"] = snapshot_id
            element["page_context_revision"] = revision
            published.append(element)
        response["interactive_elements"] = published
        self._evict_element_references(record, now)

    def resolve_element_reference(
        self,
        extension_instance_id: str,
        target: dict[str, Any],
        alias: str,
        *,
        page_context_revision: str = "",
        snapshot_id: str = "",
    ) -> ElementReferenceRecord:
        """Resolve one alias only within its target, revision, snapshot, age, and cache."""
        record = self.require_session(extension_instance_id)
        now = time.monotonic()
        self._evict_element_references(record, now)
        key = f"{_reference_target_prefix(target)}:{_clean_text(alias)}"
        reference = record.element_references.get(key)
        stale = (
            reference is None
            or not _targets_equal(reference.target, target)
            or now - reference.created_at > ELEMENT_REFERENCE_TTL_SEC
            or (
                bool(page_context_revision)
                and reference.page_context_revision != page_context_revision
            )
            or (bool(snapshot_id) and reference.snapshot_id != snapshot_id)
        )
        if stale:
            raise BrokerError(
                "stale_element_reference",
                f"element reference {_clean_text(alias) or '<empty>'} is stale or belongs to another target",
                {
                    "extension_instance_id": extension_instance_id,
                    "target": _sanitize_public_value(target),
                    "retry": "request a new snapshot and use its revision-bound reference",
                },
            )
        return reference

    def clear_element_references(
        self,
        record: SessionRecord,
        *,
        target: dict[str, Any] | None = None,
    ) -> None:
        """Clear all aliases or only aliases owned by one complete target."""
        if target is None:
            record.element_references.clear()
            return
        prefix = f"{_reference_target_prefix(target)}:"
        record.element_references = {
            key: value
            for key, value in record.element_references.items()
            if not key.startswith(prefix)
        }

    def start_console_capture(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        *,
        levels: Any = None,
        max_age_ms: Any = None,
        max_entries: Any = None,
        max_bytes: Any = None,
        sensitive_fields: Any = None,
    ) -> dict[str, Any]:
        """Start or replace one bounded target-scoped console capture."""
        normalized_levels = _normalize_console_levels(levels)
        state = ConsoleCaptureState(
            target=dict(target),
            levels=normalized_levels,
            max_age_ms=_bounded_int(
                max_age_ms,
                DEFAULT_CONSOLE_MAX_AGE_MS,
                1_000,
                MAX_CONSOLE_MAX_AGE_MS,
                "max_age_ms",
            ),
            max_entries=_bounded_int(
                max_entries,
                DEFAULT_CONSOLE_MAX_ENTRIES,
                1,
                MAX_CONSOLE_MAX_ENTRIES,
                "max_entries",
            ),
            max_bytes=_bounded_int(
                max_bytes,
                DEFAULT_CONSOLE_MAX_BYTES,
                1_024,
                MAX_CONSOLE_MAX_BYTES,
                "max_bytes",
            ),
            sensitive_fields=_normalize_sensitive_fields(sensitive_fields),
        )
        record.console_captures[_reference_target_prefix(target)] = state
        return _console_capture_summary(state)

    def record_console_event(
        self,
        extension_instance_id: str,
        target: dict[str, Any],
        raw_event: Any,
    ) -> bool:
        """Retain a console event only for its active session and target."""
        record = self.sessions.get(extension_instance_id)
        if record is None or not isinstance(target, dict):
            return False
        if target.get("extension_instance_id") != extension_instance_id:
            return False
        state = record.console_captures.get(_reference_target_prefix(target))
        if state is None or not _targets_equal(state.target, target):
            return False
        event = _sanitize_console_event(raw_event, state.sensitive_fields)
        if event is None or event["level"] not in state.levels:
            return False
        now = time.monotonic()
        self._evict_console_events(state, now)
        encoded = json.dumps(event, ensure_ascii=False, separators=(",", ":")).encode(
            "utf-8"
        )
        if len(encoded) > state.max_bytes:
            event["text"] = _truncate_utf8(
                str(event.get("text", "")),
                max(0, state.max_bytes - 512),
            )
            event["truncated"] = True
            encoded = json.dumps(
                event, ensure_ascii=False, separators=(",", ":")
            ).encode("utf-8")
        if len(encoded) > state.max_bytes:
            return False
        item = ConsoleEventRecord(event, now, len(encoded))
        state.events.append(item)
        state.byte_size += item.byte_size
        self._evict_console_events(state, now)
        return True

    def list_console_events(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        *,
        levels: Any = None,
        max_age_ms: Any = None,
        max_entries: Any = None,
        max_bytes: Any = None,
    ) -> dict[str, Any]:
        """List a bounded view without weakening the capture's retention limits."""
        state = self._require_console_capture(record, target)
        now = time.monotonic()
        self._evict_console_events(state, now)
        selected_levels = (
            _normalize_console_levels(levels) if levels is not None else state.levels
        )
        age_limit = min(
            state.max_age_ms,
            _bounded_int(
                max_age_ms,
                state.max_age_ms,
                0,
                state.max_age_ms,
                "max_age_ms",
            ),
        )
        entry_limit = min(
            state.max_entries,
            _bounded_int(
                max_entries,
                state.max_entries,
                1,
                state.max_entries,
                "max_entries",
            ),
        )
        byte_limit = min(
            state.max_bytes,
            _bounded_int(
                max_bytes,
                state.max_bytes,
                1,
                state.max_bytes,
                "max_bytes",
            ),
        )
        selected: list[dict[str, Any]] = []
        selected_bytes = 0
        for item in reversed(state.events):
            if (now - item.created_at) * 1000 > age_limit:
                continue
            if item.value.get("level") not in selected_levels:
                continue
            if len(selected) >= entry_limit or selected_bytes + item.byte_size > byte_limit:
                break
            selected.append(dict(item.value))
            selected_bytes += item.byte_size
        selected.reverse()
        return {
            **_console_capture_summary(state),
            "events": selected,
            "returned_entries": len(selected),
            "returned_bytes": selected_bytes,
        }

    def clear_console_capture(
        self, record: SessionRecord, target: dict[str, Any]
    ) -> dict[str, Any]:
        """Clear retained events while leaving capture active."""
        state = self._require_console_capture(record, target)
        removed_entries = len(state.events)
        removed_bytes = state.byte_size
        state.events.clear()
        state.byte_size = 0
        return {
            **_console_capture_summary(state),
            "removed_entries": removed_entries,
            "removed_bytes": removed_bytes,
        }

    def stop_console_capture(
        self, record: SessionRecord, target: dict[str, Any]
    ) -> dict[str, Any]:
        """Stop capture and discard its in-memory diagnostic data."""
        key = _reference_target_prefix(target)
        state = record.console_captures.pop(key, None)
        if state is None:
            return {"target": dict(target), "active": False, "removed_entries": 0}
        return {
            "target": dict(target),
            "active": False,
            "removed_entries": len(state.events),
            "removed_bytes": state.byte_size,
        }

    def _require_console_capture(
        self, record: SessionRecord, target: dict[str, Any]
    ) -> ConsoleCaptureState:
        state = record.console_captures.get(_reference_target_prefix(target))
        if state is None or not _targets_equal(state.target, target):
            raise BrokerError(
                "invalid_browser_operation",
                "console capture is not active for the selected target",
            )
        return state

    @staticmethod
    def _evict_console_events(state: ConsoleCaptureState, now: float) -> None:
        while state.events and (
            (now - state.events[0].created_at) * 1000 > state.max_age_ms
            or len(state.events) > state.max_entries
            or state.byte_size > state.max_bytes
        ):
            removed = state.events.pop(0)
            state.byte_size -= removed.byte_size

    def start_network_capture(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        *,
        allowed_hostnames: Any = None,
        capture_request_bodies: Any = False,
        max_request_body_bytes: Any = None,
        max_age_ms: Any = None,
        max_entries: Any = None,
        max_bytes: Any = None,
        max_body_bytes: Any = None,
        sensitive_fields: Any = None,
    ) -> dict[str, Any]:
        """Start or replace a hostname-filtered bounded network capture."""
        hostnames = _normalize_allowed_hostnames(allowed_hostnames)
        state = NetworkCaptureState(
            target=dict(target),
            capture_id=secrets.token_urlsafe(24),
            allowed_hostnames=hostnames,
            capture_request_bodies=bool(capture_request_bodies),
            max_request_body_bytes=_bounded_int(
                max_request_body_bytes,
                DEFAULT_NETWORK_MAX_BODY_BYTES,
                1,
                MAX_NETWORK_MAX_BODY_BYTES,
                "max_request_body_bytes",
            ),
            max_age_ms=_bounded_int(
                max_age_ms,
                DEFAULT_NETWORK_MAX_AGE_MS,
                1_000,
                MAX_NETWORK_MAX_AGE_MS,
                "max_age_ms",
            ),
            max_entries=_bounded_int(
                max_entries,
                DEFAULT_NETWORK_MAX_ENTRIES,
                1,
                MAX_NETWORK_MAX_ENTRIES,
                "max_entries",
            ),
            max_bytes=_bounded_int(
                max_bytes,
                DEFAULT_NETWORK_MAX_BYTES,
                2_048,
                MAX_NETWORK_MAX_BYTES,
                "max_bytes",
            ),
            max_body_bytes=_bounded_int(
                max_body_bytes,
                DEFAULT_NETWORK_MAX_BODY_BYTES,
                1_024,
                MAX_NETWORK_MAX_BODY_BYTES,
                "max_body_bytes",
            ),
            sensitive_fields=_normalize_sensitive_fields(sensitive_fields),
        )
        target_key = _reference_target_prefix(target)
        previous = record.network_captures.get(target_key)
        if previous is not None:
            record.network_capture_barriers[
                _network_capture_identity(previous.target, previous.capture_id)
            ] = max(
                previous.acknowledged_sequence,
                previous.highest_seen_sequence,
            )
            while len(record.network_capture_barriers) > 64:
                record.network_capture_barriers.pop(
                    next(iter(record.network_capture_barriers))
                )
        record.network_captures[target_key] = state
        return _network_capture_summary(state)

    def record_network_event(
        self,
        extension_instance_id: str,
        target: dict[str, Any],
        raw_event: Any,
    ) -> bool:
        """Reject legacy HTTP events for enhanced filtered captures."""
        record = self.sessions.get(extension_instance_id)
        if record is None or not isinstance(target, dict) or not isinstance(raw_event, dict):
            return False
        if target.get("extension_instance_id") != extension_instance_id:
            return False
        state = record.network_captures.get(_reference_target_prefix(target))
        if state is None or not _targets_equal(state.target, target):
            return False
        # Enhanced captures must only be populated by authenticated, sequenced batches.
        if state.capture_id:
            return False
        return self._merge_network_event(state, raw_event)

    def accept_network_batch(
        self,
        extension_instance_id: str,
        payload: Any,
    ) -> dict[str, Any]:
        """Merge one authenticated batch and return its contiguous acknowledgement."""
        if not isinstance(payload, dict):
            return _network_ack("", {}, 0, False, "invalid_batch")
        target = payload.get("target")
        capture_id = _clean_text(payload.get("capture_id"))[:256]
        if not isinstance(target, dict):
            return _network_ack(capture_id, {}, 0, False, "invalid_target")
        record = self.sessions.get(extension_instance_id)
        if (
            record is None
            or target.get("extension_instance_id") != extension_instance_id
        ):
            return _network_ack(capture_id, target, 0, False, "target_mismatch")
        state = record.network_captures.get(_reference_target_prefix(target))
        if (
            state is None
            or not _targets_equal(state.target, target)
            or state.capture_id != capture_id
        ):
            barrier = record.network_capture_barriers.get(
                _network_capture_identity(target, capture_id)
            )
            if barrier is not None:
                return _network_ack(capture_id, target, barrier, True)
            return _network_ack(capture_id, target, 0, False, "capture_mismatch")

        raw_events = payload.get("events")
        if not isinstance(raw_events, list):
            return _network_ack(
                capture_id,
                target,
                state.acknowledged_sequence,
                False,
                "invalid_events",
            )
        batch_bytes = len(
            json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode(
                "utf-8"
            )
        )
        if (
            len(raw_events) > MAX_NETWORK_BATCH_EVENTS
            or batch_bytes > MAX_NETWORK_BATCH_BYTES
        ):
            state.dropped_batches += 1
            return _network_ack(
                capture_id,
                target,
                state.acknowledged_sequence,
                False,
                "batch_too_large",
            )
        for item in raw_events:
            if not isinstance(item, dict):
                state.rejected_events += 1
                continue
            sequence = _as_int(item.get("seq"))
            if sequence is None:
                sequence = _as_int(item.get("sequence"))
            if sequence is None or sequence <= 0:
                state.rejected_events += 1
                continue
            state.highest_seen_sequence = max(state.highest_seen_sequence, sequence)
            if (
                sequence <= state.acknowledged_sequence
                or sequence <= state.clear_sequence
                or sequence in state.pending_events
            ):
                state.duplicate_events += 1
                continue
            if sequence > state.acknowledged_sequence + MAX_NETWORK_PENDING_EVENTS:
                state.rejected_events += 1
                continue
            event = item.get("event")
            if not isinstance(event, dict):
                event = {
                    key: value
                    for key, value in item.items()
                    if key not in {"seq", "sequence"}
                }
            state.pending_events[sequence] = event

        while state.acknowledged_sequence + 1 in state.pending_events:
            state.acknowledged_sequence += 1
            event = state.pending_events.pop(state.acknowledged_sequence)
            if state.active:
                if not self._merge_network_event(state, event):
                    state.rejected_events += 1
            else:
                state.rejected_events += 1

        state.dropped_events = max(
            state.dropped_events,
            _nonnegative_diagnostic(payload, "dropped_events", "dropped_event_count"),
        )
        state.dropped_batches = max(
            state.dropped_batches,
            _nonnegative_diagnostic(payload, "dropped_batches", "dropped_batch_count"),
        )
        state.dropped_bytes = max(
            state.dropped_bytes,
            _nonnegative_diagnostic(payload, "dropped_bytes"),
        )
        termination = payload.get("termination")
        termination_reason = _clean_text(payload.get("termination_reason"))[:256]
        if not termination_reason and isinstance(termination, dict):
            termination_reason = _clean_text(termination.get("reason"))[:256]
        if termination_reason:
            barrier_payload = payload
            if isinstance(termination, dict):
                barrier_payload = {**payload, **termination}
            barrier = _sequence_barrier(
                barrier_payload,
                max(state.acknowledged_sequence, state.highest_seen_sequence),
            )
            self._terminate_network_capture(state, termination_reason, barrier)
        return _network_ack(
            capture_id,
            target,
            state.acknowledged_sequence,
            True,
        )

    def _merge_network_event(
        self,
        state: NetworkCaptureState,
        raw_event: dict[str, Any],
    ) -> bool:
        """Merge a policy-validated CDP event into one retained request."""
        request_id = _clean_text(raw_event.get("request_id"))[:256]
        event_type = _clean_text(raw_event.get("event_type"))
        if not request_id or event_type not in {
            "request",
            "response",
            "finished",
            "failed",
        }:
            return False
        now = time.monotonic()
        self._evict_network_requests(state, now)
        previous = state.requests.get(request_id)
        if event_type == "request":
            raw_url = _clean_text(raw_event.get("url"))[:8192]
            if not _url_matches_allowed_hostname(raw_url, state.allowed_hostnames):
                return False
        elif previous is None:
            # Never create an orphan response that bypassed request hostname filtering.
            return False
        value = dict(previous.value) if previous is not None else {
            "request_id": request_id,
            "timestamp_ms": _as_int(raw_event.get("timestamp_ms"))
            or int(time.time() * 1000),
        }
        if event_type == "request":
            value.update(
                {
                    "url": _redact_url(
                        _clean_text(raw_event.get("url"))[:8192],
                        state.sensitive_fields,
                    ),
                    "method": _clean_text(raw_event.get("method"))[:32],
                    "resource_type": _clean_text(raw_event.get("resource_type"))[:64],
                    "request_headers": _redact_mapping(
                        raw_event.get("headers"), state.sensitive_fields
                    ),
                }
            )
            if state.capture_request_bodies and "request_body" in raw_event:
                request_body = raw_event.get("request_body")
                if not isinstance(request_body, dict):
                    request_body = {
                        "body": request_body,
                        "encoding": raw_event.get("request_body_encoding"),
                        "base64_encoded": raw_event.get(
                            "request_body_base64_encoded"
                        ),
                        "original_size": raw_event.get(
                            "request_body_original_size"
                        ),
                        "truncated": raw_event.get("request_body_truncated"),
                        "unavailable_reason": raw_event.get(
                            "request_body_unavailable_reason"
                        ),
                    }
                value["request_body"] = _bounded_request_body(
                    request_body,
                    state.max_request_body_bytes,
                )
        elif event_type == "response":
            value.update(
                {
                    "status": _as_int(raw_event.get("status")) or 0,
                    "status_text": _clean_text(raw_event.get("status_text"))[:256],
                    "mime_type": _clean_text(raw_event.get("mime_type"))[:256],
                    "protocol": _clean_text(raw_event.get("protocol"))[:64],
                    "from_cache": bool(raw_event.get("from_cache")),
                    "response_headers": _redact_mapping(
                        raw_event.get("headers"), state.sensitive_fields
                    ),
                }
            )
        elif event_type == "finished":
            value.update(
                {
                    "finished": True,
                    "encoded_data_length": max(
                        0, _as_int(raw_event.get("encoded_data_length")) or 0
                    ),
                }
            )
        else:
            value.update(
                {
                    "finished": True,
                    "failed": True,
                    "error_text": _clean_text(raw_event.get("error_text"))[:1024],
                    "canceled": bool(raw_event.get("canceled")),
                }
            )
        encoded = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode(
            "utf-8"
        )
        if len(encoded) > state.max_bytes:
            return False
        if previous is not None:
            state.byte_size -= previous.byte_size
        state.requests.pop(request_id, None)
        state.requests[request_id] = NetworkRequestRecord(value, now, len(encoded))
        state.byte_size += len(encoded)
        self._evict_network_requests(state, now)
        return request_id in state.requests

    def list_network_requests(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        *,
        max_age_ms: Any = None,
        max_entries: Any = None,
        max_bytes: Any = None,
    ) -> dict[str, Any]:
        state = self._require_network_capture(record, target)
        now = time.monotonic()
        self._evict_network_requests(state, now)
        age_limit = min(
            state.max_age_ms,
            _bounded_int(max_age_ms, state.max_age_ms, 0, state.max_age_ms, "max_age_ms"),
        )
        entry_limit = min(
            state.max_entries,
            _bounded_int(max_entries, state.max_entries, 1, state.max_entries, "max_entries"),
        )
        byte_limit = min(
            state.max_bytes,
            _bounded_int(max_bytes, state.max_bytes, 1, state.max_bytes, "max_bytes"),
        )
        selected: list[dict[str, Any]] = []
        selected_bytes = 0
        for item in reversed(list(state.requests.values())):
            if (now - item.created_at) * 1000 > age_limit:
                continue
            if len(selected) >= entry_limit or selected_bytes + item.byte_size > byte_limit:
                break
            selected.append(_network_summary(item.value))
            selected_bytes += item.byte_size
        selected.reverse()
        return {
            **_network_capture_summary(state),
            "requests": selected,
            "returned_entries": len(selected),
            "returned_bytes": selected_bytes,
        }

    def get_network_request_detail(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        request_id: Any,
    ) -> dict[str, Any]:
        state = self._require_network_capture(record, target)
        self._evict_network_requests(state, time.monotonic())
        normalized = _clean_text(request_id)
        item = state.requests.get(normalized)
        if item is None:
            raise BrokerError(
                "browser_target_not_found",
                "captured network request was not found or has expired",
                {"request_id": normalized[:256]},
            )
        return {
            **_network_capture_summary(state),
            "request": dict(item.value),
        }

    def bound_network_body(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        request_id: Any,
        body: Any,
        base64_encoded: Any,
        requested_max_bytes: Any = None,
    ) -> dict[str, Any]:
        """Return only an explicitly requested, size-bounded response body."""
        state = self._require_network_capture(record, target)
        detail = self.get_network_request_detail(record, target, request_id)
        limit = min(
            state.max_body_bytes,
            _bounded_int(
                requested_max_bytes,
                state.max_body_bytes,
                1,
                state.max_body_bytes,
                "max_body_bytes",
            ),
        )
        encoded_body = _clean_text(body)
        is_base64 = bool(base64_encoded)
        if is_base64:
            try:
                raw = base64.b64decode(encoded_body, validate=True)
            except (ValueError, TypeError) as exc:
                raise BrokerError(
                    "browser_operation_failed", "browser returned an invalid base64 body"
                ) from exc
            original_size = len(raw)
            truncated = original_size > limit
            output = base64.b64encode(raw[:limit]).decode("ascii")
        else:
            raw = encoded_body.encode("utf-8")
            original_size = len(raw)
            truncated = original_size > limit
            output = raw[:limit].decode("utf-8", errors="ignore")
        return {
            **detail,
            "body": output,
            "base64_encoded": is_base64,
            "truncated": truncated,
            "original_size": original_size,
            "returned_size": min(original_size, limit),
        }

    def clear_network_capture(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        sequence_barrier: Any = None,
    ) -> dict[str, Any]:
        state = self._require_network_capture(record, target)
        removed_entries = len(state.requests)
        removed_bytes = state.byte_size
        state.requests.clear()
        state.byte_size = 0
        barrier = _bounded_sequence(
            sequence_barrier,
            max(state.acknowledged_sequence, state.highest_seen_sequence),
        )
        state.clear_sequence = max(state.clear_sequence, barrier)
        state.acknowledged_sequence = max(state.acknowledged_sequence, barrier)
        state.pending_events = {
            sequence: event
            for sequence, event in state.pending_events.items()
            if sequence > state.acknowledged_sequence
        }
        return {
            **_network_capture_summary(state),
            "removed_entries": removed_entries,
            "removed_bytes": removed_bytes,
        }

    def stop_network_capture(
        self,
        record: SessionRecord,
        target: dict[str, Any],
        *,
        sequence_barrier: Any = None,
        termination_reason: str = "explicit_stop",
    ) -> dict[str, Any]:
        state = record.network_captures.pop(_reference_target_prefix(target), None)
        if state is None:
            return {"target": dict(target), "active": False, "removed_entries": 0}
        barrier = _bounded_sequence(
            sequence_barrier,
            max(state.acknowledged_sequence, state.highest_seen_sequence),
        )
        self._terminate_network_capture(state, termination_reason, barrier)
        record.network_capture_barriers[
            _network_capture_identity(target, state.capture_id)
        ] = state.acknowledged_sequence
        while len(record.network_capture_barriers) > 64:
            record.network_capture_barriers.pop(
                next(iter(record.network_capture_barriers))
            )
        return {
            **_network_capture_summary(state),
            "removed_entries": len(state.requests),
            "removed_bytes": state.byte_size,
        }

    @staticmethod
    def _terminate_network_capture(
        state: NetworkCaptureState,
        reason: str,
        sequence_barrier: int,
    ) -> None:
        state.active = False
        state.termination_reason = _clean_text(reason)[:256] or "unknown"
        state.terminated_at_ms = int(time.time() * 1000)
        state.clear_sequence = max(state.clear_sequence, sequence_barrier)
        state.acknowledged_sequence = max(
            state.acknowledged_sequence, sequence_barrier
        )
        state.pending_events = {
            sequence: event
            for sequence, event in state.pending_events.items()
            if sequence > state.acknowledged_sequence
        }

    def _require_network_capture(
        self, record: SessionRecord, target: dict[str, Any]
    ) -> NetworkCaptureState:
        state = record.network_captures.get(_reference_target_prefix(target))
        if state is None or not _targets_equal(state.target, target):
            raise BrokerError(
                "invalid_browser_operation",
                "network capture is not active for the selected target",
            )
        return state

    @staticmethod
    def _evict_network_requests(state: NetworkCaptureState, now: float) -> None:
        while state.requests:
            first_key = next(iter(state.requests))
            first = state.requests[first_key]
            if not (
                (now - first.created_at) * 1000 > state.max_age_ms
                or len(state.requests) > state.max_entries
                or state.byte_size > state.max_bytes
            ):
                break
            state.requests.pop(first_key)
            state.byte_size -= first.byte_size

    def _evict_element_references(
        self, record: SessionRecord, now: float
    ) -> None:
        live = [
            (key, reference)
            for key, reference in record.element_references.items()
            if now - reference.created_at <= ELEMENT_REFERENCE_TTL_SEC
        ]
        live.sort(key=lambda item: item[1].created_at, reverse=True)
        record.element_references = dict(
            live[:MAX_ELEMENT_REFERENCES_PER_SESSION]
        )

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
        record = self.sessions.get(pending.extension_instance_id)
        if record is not None and response.get("ok"):
            if pending.operation == "get_page_snapshot":
                self.cache_snapshot_references(record, pending.target, response)
            elif pending.operation in {"navigate", "close_tab"}:
                self.clear_element_references(record, target=pending.target)
        if not pending.future.done():
            pending.future.set_result(response)
        self._release_ephemeral_lease(pending)
        return pending

    def take_queued_command(
        self, extension_instance_id: str, request_id: str
    ) -> dict[str, Any] | None:
        """Atomically claim one queued command for a negotiated direct transport."""
        record = self.sessions.get(extension_instance_id)
        if record is None:
            return None
        for index, command in enumerate(record.command_queue):
            if str(command.get("request_id") or "") == request_id:
                return record.command_queue.pop(index)
        return None

    def restore_queued_command(
        self, extension_instance_id: str, command: dict[str, Any]
    ) -> None:
        """Restore a direct-send failure to bounded heartbeat fallback once."""
        request_id = _clean_text(command.get("request_id"))
        pending = self.pending.get(request_id)
        record = self.sessions.get(extension_instance_id)
        if pending is None or record is None:
            return
        if any(
            queued.get("request_id") == request_id
            for queued in record.command_queue
        ):
            return
        record.command_queue.insert(0, command)

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
        self.expire_capability_grants(current)
        for record in list(self.sessions.values()):
            self._release_expired_lease(record, current)
            if record.alive(current, self.heartbeat_ttl):
                continue
            record.command_queue.clear()
            record.lease = None
            record.element_references.clear()
            record.console_captures.clear()
            record.network_captures.clear()
            record.network_capture_barriers.clear()
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


def _sanitize_feature_availability(raw: Any) -> list[dict[str, Any]]:
    if not isinstance(raw, list):
        return []
    features: dict[str, dict[str, Any]] = {}
    for item in raw[:64]:
        if not isinstance(item, dict):
            continue
        feature = _clean_text(item.get("feature"))
        if feature not in KNOWN_FEATURE_IDS:
            continue
        entry: dict[str, Any] = {
            "feature": feature,
            "available": bool(item.get("available")),
        }
        reason = _clean_text(item.get("reason"))[:200]
        if reason:
            entry["reason"] = reason
        features[feature] = entry
    return [features[name] for name in sorted(features)]


def _sanitize_supported_actions(raw: Any) -> list[str]:
    if not isinstance(raw, list):
        return []
    return sorted(
        {
            action
            for value in raw[:64]
            if (action := _clean_text(value)) in KNOWN_ACTIONS
        }
    )


def _sanitize_supported_operations(raw: Any) -> list[str]:
    if not isinstance(raw, list):
        return []
    return sorted(
        {
            operation
            for value in raw[:128]
            if (operation := _clean_text(value)) in KNOWN_BROWSER_OPERATIONS
        }
    )


def _sanitize_optional_permissions(raw: Any) -> dict[str, bool]:
    if not isinstance(raw, dict):
        return {}
    allowed = {"cookies", "content_settings", "extension_management"}
    return {
        key: bool(value)
        for key, value in sorted(raw.items())
        if key in allowed
    }


def _sanitize_public_value(value: Any) -> Any:
    if isinstance(value, dict):
        result: dict[str, Any] = {}
        for key, nested in value.items():
            normalized = str(key).lower().replace("-", "_")
            if normalized in {
                "lease_token",
                "capability_grant",
                "capability_grant_token",
            } or normalized.endswith("_secret"):
                continue
            result[str(key)] = _sanitize_public_value(nested)
        return result
    if isinstance(value, list):
        return [_sanitize_public_value(item) for item in value]
    return value


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


def _reference_target_prefix(target: dict[str, Any]) -> str:
    """Build a stable non-secret cache namespace for a complete browser target."""
    return ":".join(
        [
            _clean_text(target.get("extension_instance_id")),
            str(_as_int(target.get("window_id")) or 0),
            str(_as_int(target.get("tab_id")) or 0),
        ]
    )


def _normalize_console_levels(value: Any) -> set[str]:
    if value is None:
        return set(KNOWN_CONSOLE_LEVELS)
    if not isinstance(value, list):
        raise BrokerError("invalid_browser_operation", "levels must be an array")
    levels = {_clean_text(item).lower() for item in value}
    unknown = sorted(levels - KNOWN_CONSOLE_LEVELS)
    if unknown:
        raise BrokerError(
            "invalid_browser_operation",
            "unsupported console level filter",
            {"unsupported_levels": unknown, "supported_levels": sorted(KNOWN_CONSOLE_LEVELS)},
        )
    if not levels:
        raise BrokerError("invalid_browser_operation", "levels must not be empty")
    return levels


def _bounded_int(
    value: Any,
    default: int,
    minimum: int,
    maximum: int,
    field_name: str,
) -> int:
    if value is None:
        return default
    parsed = _as_int(value)
    if parsed is None or parsed < minimum or parsed > maximum:
        raise BrokerError(
            "invalid_browser_operation",
            f"{field_name} must be between {minimum} and {maximum}",
        )
    return parsed


def _truncate_utf8(value: str, max_bytes: int) -> str:
    encoded = value.encode("utf-8")
    if len(encoded) <= max_bytes:
        return value
    return encoded[:max_bytes].decode("utf-8", errors="ignore")


def _sanitize_console_event(
    raw: Any, sensitive_fields: set[str]
) -> dict[str, Any] | None:
    if not isinstance(raw, dict):
        return None
    level = _clean_text(raw.get("level")).lower()
    if level == "warning":
        level = "warn"
    if level not in KNOWN_CONSOLE_LEVELS:
        level = "log"
    timestamp_ms = _as_int(raw.get("timestamp_ms")) or int(time.time() * 1000)
    event: dict[str, Any] = {
        "timestamp_ms": timestamp_ms,
        "level": level,
        "text": _truncate_utf8(
            _redact_console_text(_clean_text(raw.get("text")), sensitive_fields),
            MAX_CONSOLE_EVENT_TEXT_BYTES,
        ),
    }
    source = _clean_text(raw.get("source"))[:120]
    url = _clean_text(raw.get("url"))[:4096]
    line_number = _as_int(raw.get("line_number"))
    if source:
        event["source"] = source
    if url:
        event["url"] = url
    if line_number is not None and line_number >= 0:
        event["line_number"] = line_number
    if len(_clean_text(raw.get("text")).encode("utf-8")) > MAX_CONSOLE_EVENT_TEXT_BYTES:
        event["truncated"] = True
    return event


def _console_capture_summary(state: ConsoleCaptureState) -> dict[str, Any]:
    return {
        "target": dict(state.target),
        "active": True,
        "levels": sorted(state.levels),
        "retention": {
            "max_age_ms": state.max_age_ms,
            "max_entries": state.max_entries,
            "max_bytes": state.max_bytes,
        },
        "retained_entries": len(state.events),
        "retained_bytes": state.byte_size,
    }


def _normalize_sensitive_fields(value: Any) -> set[str]:
    fields = set(DEFAULT_SENSITIVE_DIAGNOSTIC_FIELDS)
    if value is None:
        return fields
    if not isinstance(value, list):
        raise BrokerError(
            "invalid_browser_operation", "sensitive_fields must be an array"
        )
    for item in value[:128]:
        normalized = _clean_text(item).lower()[:128]
        if normalized:
            fields.add(normalized)
    return fields


def _is_sensitive_field(name: str, sensitive_fields: set[str]) -> bool:
    normalized = name.lower().replace("_", "-")
    return normalized in sensitive_fields or any(
        marker in normalized for marker in ("token", "password", "passwd", "secret")
    )


def _redact_mapping(value: Any, sensitive_fields: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        return {}
    result: dict[str, Any] = {}
    for raw_name, raw_value in list(value.items())[:256]:
        name = str(raw_name)[:256]
        if _is_sensitive_field(name, sensitive_fields):
            result[name] = REDACTION_MARKER
        elif isinstance(raw_value, dict):
            result[name] = _redact_mapping(raw_value, sensitive_fields)
        elif isinstance(raw_value, list):
            result[name] = [
                REDACTION_MARKER
                if isinstance(item, dict)
                else _clean_text(item)[:4096]
                for item in raw_value[:128]
            ]
        else:
            result[name] = _clean_text(raw_value)[:4096]
    return result


def _redact_console_text(value: str, sensitive_fields: set[str]) -> str:
    redacted = value
    for field_name in sorted(sensitive_fields, key=len, reverse=True):
        field = re.escape(field_name)
        pattern = re.compile(
            rf"(?i)([\"']?{field}[\"']?\s*[:=]\s*)([^,;\n}}]+)"
        )
        redacted = pattern.sub(rf"\1{REDACTION_MARKER}", redacted)
    return redacted


def _redact_url(value: str, sensitive_fields: set[str]) -> str:
    try:
        parts = urlsplit(value)
        if not parts.query:
            return value
        query = urlencode(
            [
                (name, REDACTION_MARKER if _is_sensitive_field(name, sensitive_fields) else item)
                for name, item in parse_qsl(parts.query, keep_blank_values=True)
            ],
            doseq=True,
        )
        return urlunsplit((parts.scheme, parts.netloc, parts.path, query, parts.fragment))
    except ValueError:
        return value


def _normalize_allowed_hostnames(value: Any) -> tuple[str, ...]:
    if isinstance(value, str):
        raw_values = [value]
    elif isinstance(value, (list, tuple, set)):
        raw_values = list(value)
    else:
        raw_values = []
    normalized: set[str] = set()
    for raw in raw_values:
        hostname = _clean_text(raw).rstrip(".").lower()
        if not hostname:
            continue
        if any(marker in hostname for marker in ("://", "/", "\\", "@", "*", "?", "#")):
            raise BrokerError(
                "invalid_browser_operation",
                "allowed_hostnames must contain exact hostnames without URL components or wildcards",
            )
        try:
            hostname = hostname.encode("idna").decode("ascii")
        except UnicodeError as exc:
            raise BrokerError(
                "invalid_browser_operation",
                "allowed_hostnames contains an invalid hostname",
            ) from exc
        if (
            len(hostname) > 253
            or ":" in hostname
            or any(
                not label
                or len(label) > 63
                or label.startswith("-")
                or label.endswith("-")
                or re.fullmatch(r"[a-z0-9-]+", label) is None
                for label in hostname.split(".")
            )
        ):
            raise BrokerError(
                "invalid_browser_operation",
                "allowed_hostnames contains an invalid exact hostname",
            )
        normalized.add(hostname)
    if not normalized:
        raise BrokerError(
            "invalid_browser_operation",
            "start_network_capture requires at least one exact hostname",
        )
    return tuple(sorted(normalized))


def _url_matches_allowed_hostname(
    value: str,
    allowed_hostnames: tuple[str, ...],
) -> bool:
    try:
        parsed = urlsplit(value)
        hostname = (parsed.hostname or "").rstrip(".").lower()
        if parsed.scheme.lower() not in {"http", "https"} or not hostname:
            return False
        hostname = hostname.encode("idna").decode("ascii")
    except (UnicodeError, ValueError):
        return False
    return hostname in allowed_hostnames


def _bounded_request_body(value: Any, limit: int) -> dict[str, Any]:
    if not isinstance(value, dict):
        return {
            "encoding": "utf8",
            "body": "",
            "captured_size": 0,
            "original_size": None,
            "truncated": False,
            "unavailable_reason": "invalid_request_body",
        }
    encoding = _clean_text(value.get("encoding")).lower()
    if not encoding:
        encoding = "base64" if value.get("base64_encoded") else "utf8"
    body = value.get("body")
    if body is None:
        body = value.get("data")
    body_text = str(body) if body is not None else ""
    unavailable_reason = _clean_text(
        value.get("unavailable_reason") or value.get("unavailable")
    )[:256]
    if encoding == "base64":
        try:
            raw = base64.b64decode(body_text, validate=True)
        except (TypeError, ValueError):
            raw = b""
            unavailable_reason = unavailable_reason or "invalid_base64"
        bounded = raw[:limit]
        output = base64.b64encode(bounded).decode("ascii")
    else:
        encoding = "utf8"
        raw = body_text.encode("utf-8")
        bounded = raw[:limit]
        output = bounded.decode("utf-8", errors="ignore")
        bounded = output.encode("utf-8")
    reported_original = _as_int(value.get("original_size"))
    original_size = (
        max(0, reported_original) if reported_original is not None else None
    )
    truncated = bool(value.get("truncated")) or len(raw) > limit
    if original_size is not None:
        truncated = truncated or original_size > len(bounded)
    return {
        "encoding": encoding,
        "body": output,
        "captured_size": len(bounded),
        "original_size": original_size,
        "truncated": truncated,
        "unavailable_reason": unavailable_reason or None,
    }


def _nonnegative_diagnostic(payload: dict[str, Any], *names: str) -> int:
    for name in names:
        parsed = _as_int(payload.get(name))
        if parsed is not None:
            return max(0, parsed)
    diagnostics = payload.get("diagnostics")
    if isinstance(diagnostics, dict):
        for name in names:
            parsed = _as_int(diagnostics.get(name))
            if parsed is not None:
                return max(0, parsed)
    return 0


def _bounded_sequence(value: Any, default: int) -> int:
    parsed = _as_int(value)
    return max(0, default if parsed is None else parsed)


def _sequence_barrier(payload: dict[str, Any], default: int) -> int:
    for name in ("sequence_barrier", "final_sequence", "last_sequence"):
        if name in payload:
            return _bounded_sequence(payload.get(name), default)
    return max(0, default)


def _network_ack(
    capture_id: str,
    target: dict[str, Any],
    acknowledged_sequence: int,
    accepted: bool,
    reason: str | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "type": "network_ack",
        "capture_id": capture_id,
        "target": dict(target),
        "ack_seq": max(0, acknowledged_sequence),
        "acknowledged_sequence": max(0, acknowledged_sequence),
        "accepted": accepted,
    }
    if reason:
        result["reason"] = reason
    return result


def _network_capture_identity(target: dict[str, Any], capture_id: str) -> str:
    return f"{_reference_target_prefix(target)}:{capture_id}"


def _network_capture_summary(state: NetworkCaptureState) -> dict[str, Any]:
    return {
        "target": dict(state.target),
        "capture_id": state.capture_id,
        "active": state.active,
        "allowed_hostnames": list(state.allowed_hostnames),
        "capture_request_bodies": state.capture_request_bodies,
        "retention": {
            "max_age_ms": state.max_age_ms,
            "max_entries": state.max_entries,
            "max_bytes": state.max_bytes,
            "max_body_bytes": state.max_body_bytes,
            "max_request_body_bytes": state.max_request_body_bytes,
        },
        "retained_entries": len(state.requests),
        "retained_bytes": state.byte_size,
        "delivery": {
            "acknowledged_sequence": state.acknowledged_sequence,
            "highest_seen_sequence": state.highest_seen_sequence,
            "clear_sequence": state.clear_sequence,
            "pending_sequences": len(state.pending_events),
            "dropped_events": state.dropped_events,
            "dropped_batches": state.dropped_batches,
            "dropped_bytes": state.dropped_bytes,
            "rejected_events": state.rejected_events,
            "duplicate_events": state.duplicate_events,
        },
        "termination": {
            "reason": state.termination_reason,
            "terminated_at_ms": state.terminated_at_ms,
        },
    }


def _network_summary(value: dict[str, Any]) -> dict[str, Any]:
    return {
        key: value[key]
        for key in (
            "request_id",
            "timestamp_ms",
            "url",
            "method",
            "resource_type",
            "status",
            "status_text",
            "mime_type",
            "protocol",
            "from_cache",
            "finished",
            "failed",
            "error_text",
            "canceled",
            "encoded_data_length",
        )
        if key in value
    }


def _bounded_ttl(value: Any) -> int:
    parsed = _as_int(value) or DEFAULT_LEASE_TTL_SEC
    return max(MIN_LEASE_TTL_SEC, min(MAX_LEASE_TTL_SEC, parsed))


def _bounded_capability_ttl(value: Any) -> int:
    parsed = _as_int(value) or DEFAULT_CAPABILITY_GRANT_TTL_SEC
    return max(
        MIN_CAPABILITY_GRANT_TTL_SEC,
        min(MAX_CAPABILITY_GRANT_TTL_SEC, parsed),
    )


def _canonical_project_root(value: Any) -> str:
    raw = _clean_text(value)
    if not raw:
        return ""
    return os.path.normcase(os.path.realpath(os.path.abspath(raw)))


def _token_hash(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


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
