"""Teshi HTTP API BDD helper: Jinja2 envelopes, httpx, extract/assert, NDJSON.

Step definitions import this module and call ``call(template_id)``. Interactive
TUI/GPUI talks to ``api_service.py``, which wraps the same ``Session``. Pure
``@api`` CI can run behave against the same helpers.

Templates MUST NOT execute Python or touch the host OS. ``{% include %}`` and
``{% import %}`` are restricted to configured template roots.
"""

from __future__ import annotations

import json
import os
import re
import sys
import time
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, TextIO

try:
    from jinja2 import BaseLoader, Environment, StrictUndefined, TemplateNotFound
    from jinja2.sandbox import SandboxedEnvironment
except ImportError as exc:  # pragma: no cover - import preflight catches this
    raise ImportError(
        "jinja2 is required for Teshi API BDD. Install it with: pip install jinja2"
    ) from exc

HttpSend = Callable[[str, str, dict[str, str], Any, int | None], "HttpResult"]

REDACTED = "***"
_SENSITIVE_HEADERS = {
    "authorization",
    "cookie",
    "set-cookie",
    "proxy-authorization",
    "x-api-key",
}
_SENSITIVE_SUBSTRINGS = ("token", "password", "secret", "api-key", "apikey", "authorization")
_API_MARKER = re.compile(r"^\[API\]\s*", re.IGNORECASE)


class ApiError(Exception):
    """Raised when an envelope render, HTTP call, extract, or assert fails."""


@dataclass
class HttpResult:
    """Minimal HTTP round-trip used by the two-pass renderer."""

    status: int
    headers: dict[str, str]
    body: str
    json_value: Any = None


class PendingResponse:
    """Stand-in for ``response`` during the request render pass.

    Attribute and item access yield another pending node so extract expressions
    do not fail the first pass. Stringifying yields an empty value so the
    rendered JSON stays valid.
    """

    def __getattr__(self, name: str) -> PendingResponse:
        return PendingResponse()

    def __getitem__(self, key: Any) -> PendingResponse:
        return PendingResponse()

    def __str__(self) -> str:
        return ""

    def __iter__(self):
        return iter(())

    def __bool__(self) -> bool:
        return False

    def __html__(self) -> str:
        return ""


class ResponseView:
    """Attribute-friendly view of an HTTP response for Jinja2 extract/assert."""

    def __init__(self, result: HttpResult) -> None:
        self.status = result.status
        self.headers = result.headers
        self.body = result.body
        self.json = _wrap_json(result.json_value)


class JsonView:
    """Dict/list wrapper so ``response.json.id`` and ``response.json['id']`` both work."""

    def __init__(self, data: Any) -> None:
        self._data = data

    def __getattr__(self, name: str) -> Any:
        if name.startswith("_"):
            raise AttributeError(name)
        if isinstance(self._data, dict) and name in self._data:
            return _wrap_json(self._data[name])
        return None

    def __getitem__(self, key: Any) -> Any:
        if isinstance(self._data, dict):
            return _wrap_json(self._data[key])
        if isinstance(self._data, list):
            return _wrap_json(self._data[key])
        raise KeyError(key)

    def __iter__(self):
        if isinstance(self._data, dict):
            return iter(self._data)
        if isinstance(self._data, list):
            return iter(self._data)
        return iter(())

    def __bool__(self) -> bool:
        return bool(self._data)

    def __str__(self) -> str:
        if isinstance(self._data, (dict, list)):
            return json.dumps(self._data)
        if self._data is None:
            return ""
        return str(self._data)


def _wrap_json(value: Any) -> Any:
    if isinstance(value, (dict, list)):
        return JsonView(value)
    return value


class RootBoundLoader(BaseLoader):
    """Loads templates only from resolved roots; rejects path escape."""

    def __init__(self, roots: list[Path]) -> None:
        self.roots = [root.resolve() for root in roots]

    def get_source(self, environment: Environment, template: str) -> tuple[str, str, Callable[[], bool]]:
        del environment
        last_error: Exception | None = None
        for root in self.roots:
            candidate = (root / template).resolve()
            try:
                candidate.relative_to(root)
            except ValueError:
                last_error = ApiError(
                    f"template include {template!r} resolves outside template roots"
                )
                continue
            if not candidate.is_file():
                continue
            source = candidate.read_text(encoding="utf-8")
            mtime = candidate.stat().st_mtime

            def uptodate(path: Path = candidate, expected: float = mtime) -> bool:
                try:
                    return path.stat().st_mtime == expected
                except OSError:
                    return False

            return source, str(candidate), uptodate
        if last_error is not None:
            raise last_error
        raise TemplateNotFound(template)


def strip_api_marker(text: str) -> tuple[bool, str]:
    """Strip a leading ``[API]`` token from step body text.

    Returns ``(was_api, remaining_text)``. Matching uses the remaining text.
    """
    trimmed = text.lstrip()
    match = _API_MARKER.match(trimmed)
    if match is None:
        return False, text
    return True, trimmed[match.end() :]


def env_var_to_vars_key(name: str) -> str | None:
    """Map ``TESHI_API_TOKEN`` to ``token``; other names return ``None``."""
    prefix = "TESHI_API_"
    if not name.startswith(prefix) or name == prefix:
        return None
    rest = name[len(prefix) :]
    if not rest:
        return None
    return rest.lower()


def seed_vars_from_env(environ: dict[str, str] | None = None) -> dict[str, Any]:
    """Build the env-derived slice of scenario vars from ``TESHI_API_*``."""
    source = os.environ if environ is None else environ
    seeded: dict[str, Any] = {}
    for key, value in source.items():
        mapped = env_var_to_vars_key(key)
        if mapped is not None:
            seeded[mapped] = value
    return seeded


def load_api_config(project_root: Path) -> dict[str, Any]:
    """Read ``teshi.toml`` ``[api]``. Missing file yields defaults."""
    config_path = project_root / "teshi.toml"
    templates = "api"
    vars_seed: dict[str, Any] = {}
    redact_keys: list[str] = []
    if config_path.is_file():
        try:
            import tomllib
        except ImportError:  # pragma: no cover - py<3.11
            tomllib = None  # type: ignore[assignment]
        if tomllib is not None:
            with config_path.open("rb") as handle:
                parsed = tomllib.load(handle)
            api = parsed.get("api") or {}
            if isinstance(api, dict):
                templates = str(api.get("templates") or templates)
                extra_vars = api.get("vars") or {}
                if isinstance(extra_vars, dict):
                    vars_seed = {str(k): v for k, v in extra_vars.items()}
                extra_redact = api.get("redact_keys") or []
                if isinstance(extra_redact, list):
                    redact_keys = [str(item) for item in extra_redact]
                # Convenience: scalar keys besides templates/vars/redact_keys seed vars.
                for key, value in api.items():
                    if key in {"templates", "vars", "redact_keys"}:
                        continue
                    if isinstance(value, (str, int, float, bool)):
                        vars_seed.setdefault(str(key), value)
    return {
        "templates": templates,
        "vars": vars_seed,
        "redact_keys": redact_keys,
    }


def is_sensitive_name(name: str, extra_keys: list[str] | None = None) -> bool:
    """Return whether a header or JSON field name should be redacted by default."""
    lowered = name.lower()
    if lowered in _SENSITIVE_HEADERS:
        return True
    if any(token in lowered for token in _SENSITIVE_SUBSTRINGS):
        return True
    extras = extra_keys or []
    return any(lowered == extra.lower() or extra.lower() in lowered for extra in extras)


def redact_value(value: Any, extra_keys: list[str] | None = None) -> Any:
    """Redact sensitive keys in nested JSON-like structures."""
    if isinstance(value, dict):
        return {
            key: (REDACTED if is_sensitive_name(str(key), extra_keys) else redact_value(inner, extra_keys))
            for key, inner in value.items()
        }
    if isinstance(value, list):
        return [redact_value(item, extra_keys) for item in value]
    return value


def redact_headers(headers: dict[str, str], extra_keys: list[str] | None = None) -> dict[str, str]:
    """Redact sensitive HTTP header values."""
    return {
        key: (REDACTED if is_sensitive_name(key, extra_keys) else value)
        for key, value in headers.items()
    }


def redact_exchange(exchange: dict[str, Any], extra_keys: list[str] | None = None) -> dict[str, Any]:
    """Return a copy of an ``http_exchange`` payload with secrets masked."""
    redacted = dict(exchange)
    if isinstance(redacted.get("request_headers"), dict):
        redacted["request_headers"] = redact_headers(redacted["request_headers"], extra_keys)
    if isinstance(redacted.get("response_headers"), dict):
        redacted["response_headers"] = redact_headers(redacted["response_headers"], extra_keys)
    if "request_body" in redacted:
        redacted["request_body"] = redact_value(redacted["request_body"], extra_keys)
    if "response_body" in redacted:
        redacted["response_body"] = redact_value(redacted["response_body"], extra_keys)
    if isinstance(redacted.get("extract"), dict):
        redacted["extract"] = redact_value(redacted["extract"], extra_keys)
    redacted["redacted"] = True
    return redacted


def _default_http_send(
    method: str,
    url: str,
    headers: dict[str, str],
    body: Any,
    timeout_ms: int | None,
) -> HttpResult:
    try:
        import httpx
    except ImportError as exc:  # pragma: no cover
        raise ApiError("httpx is required for Teshi API BDD. Install it with: pip install httpx") from exc

    timeout = (timeout_ms if timeout_ms is not None else 30_000) / 1000.0
    kwargs: dict[str, Any] = {"headers": headers or None, "timeout": timeout}
    if body is None or body == "":
        pass
    elif isinstance(body, (dict, list)):
        kwargs["json"] = body
    else:
        kwargs["content"] = body if isinstance(body, (bytes, bytearray)) else str(body)

    with httpx.Client(follow_redirects=True) as client:
        response = client.request(method.upper(), url, **kwargs)
    text = response.text
    json_value: Any = None
    try:
        json_value = response.json()
    except Exception:
        json_value = None
    return HttpResult(
        status=response.status_code,
        headers={k: v for k, v in response.headers.items()},
        body=text,
        json_value=json_value,
    )


def _compile_step_pattern(pattern: str) -> re.Pattern[str]:
    parts: list[str] = []
    cursor = 0
    for match in re.finditer(r"\{(\w+)\}", pattern):
        parts.append(re.escape(pattern[cursor : match.start()]))
        parts.append(rf"(?P<{match.group(1)}>.+?)")
        cursor = match.end()
    parts.append(re.escape(pattern[cursor:]))
    return re.compile("^" + "".join(parts) + "$")


def _unquote(value: str) -> str:
    if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
        return value[1:-1]
    return value


def _assert_passed(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, (int, float)):
        return value != 0
    text = str(value).strip().lower()
    if text in {"false", "0", "", "none", "null", "undefined"}:
        return False
    return True


@dataclass
class StepDef:
    """One registered Gherkin step pattern and callable."""

    pattern: str
    regex: re.Pattern[str]
    func: Callable[..., Any]


@dataclass
class Session:
    """Scenario-scoped API runner: vars, template roots, and HTTP transport."""

    project_root: Path
    http_send: HttpSend | None = None
    ndjson_out: TextIO | None = None
    _vars: dict[str, Any] = field(default_factory=dict)
    _seed: dict[str, Any] = field(default_factory=dict)
    _redact_keys: list[str] = field(default_factory=list)
    _template_roots: list[Path] = field(default_factory=list)
    _exchanges: dict[str, dict[str, Any]] = field(default_factory=dict)
    _current_step_id: str | None = None
    _current_case_id: str | None = None
    _step_defs: list[StepDef] = field(default_factory=list)

    def __post_init__(self) -> None:
        global _ACTIVE
        self.project_root = Path(self.project_root).resolve()
        config = load_api_config(self.project_root)
        templates_rel = Path(str(config["templates"]))
        template_root = templates_rel if templates_rel.is_absolute() else self.project_root / templates_rel
        self._template_roots = [template_root.resolve()]
        self._redact_keys = list(config["redact_keys"])
        self._seed = {}
        self._seed.update(config["vars"])
        self._seed.update(seed_vars_from_env())
        if self.http_send is None:
            self.http_send = _default_http_send
        if self.ndjson_out is None and os.environ.get("TESHI_API_NDJSON", "").strip() not in {"", "0"}:
            self.ndjson_out = sys.stdout
        self.begin_scenario()
        _ACTIVE = self

    @property
    def vars(self) -> dict[str, Any]:
        """Current scenario variables (captures, extract, env, teshi.toml)."""
        return self._vars

    def begin_scenario(self) -> None:
        """Clear scenario vars and re-seed from teshi.toml plus ``TESHI_API_*``."""
        self._vars = dict(self._seed)
        self._exchanges.clear()
        self._current_step_id = None
        self._current_case_id = None

    def set_step_context(self, case_id: str | None, step_id: str | None) -> None:
        """Bind subsequent ``http_exchange`` events to a Gherkin step."""
        self._current_case_id = case_id
        self._current_step_id = step_id

    def register_when(self, pattern: str, func: Callable[..., Any]) -> Callable[..., Any]:
        """Register a step definition used by the sidecar dispatcher and behave."""
        self._step_defs.append(StepDef(pattern=pattern, regex=_compile_step_pattern(pattern), func=func))
        try:
            from behave import when as behave_when

            return behave_when(pattern)(func)
        except ImportError:
            return func

    def load_step_modules(self) -> list[Path]:
        """Import ``features/steps/*.py`` so ``@when`` handlers register."""
        global _ACTIVE
        _ACTIVE = self
        steps_dir = self.project_root / "features" / "steps"
        loaded: list[Path] = []
        if not steps_dir.is_dir():
            return loaded
        sys.path.insert(0, str(self.project_root))
        sys.path.insert(0, str(steps_dir))
        # Ensure ``import teshi_api`` from step files resolves to this module.
        resources_dir = Path(__file__).resolve().parent
        if str(resources_dir) not in sys.path:
            sys.path.insert(0, str(resources_dir))
        for path in sorted(steps_dir.glob("*.py")):
            if path.name.startswith("_"):
                continue
            spec_name = f"teshi_api_steps_{path.stem}"
            import importlib.util

            spec = importlib.util.spec_from_file_location(spec_name, path)
            if spec is None or spec.loader is None:
                continue
            module = importlib.util.module_from_spec(spec)
            sys.modules[spec_name] = module
            spec.loader.exec_module(module)
            loaded.append(path)
        return loaded

    def execute_step(self, text: str, case_id: str | None = None, step_id: str | None = None) -> dict[str, Any]:
        """Match stripped step text against registered defs and run the handler."""
        _was_api, remaining = strip_api_marker(text)
        del _was_api
        step_id = step_id or str(uuid.uuid4())
        self.set_step_context(case_id, step_id)
        events: list[dict[str, Any]] = []
        start_event = {
            "type": "start_step",
            "case_id": case_id,
            "step_id": step_id,
            "text": remaining,
        }
        events.append(start_event)
        self._emit(start_event)
        before_ids = set(self._exchanges)
        try:
            matched = False
            for definition in self._step_defs:
                match = definition.regex.match(remaining)
                if match is None:
                    continue
                matched = True
                captures = {key: _unquote(value) for key, value in match.groupdict().items()}
                self._vars.update(captures)
                context = StepContext(self)
                definition.func(context, **captures)
                break
            if not matched:
                raise ApiError(f"undefined API step: {remaining}")
            status = "passed"
            message = None
            ok = True
        except Exception as exc:
            status = "failed"
            message = str(exc)
            ok = False

        new_ids = [eid for eid in self._exchanges if eid not in before_ids]
        public_exchanges = [self._public_exchange(eid) for eid in new_ids]
        events.extend(public_exchanges)
        end_event = {
            "type": "end_step",
            "case_id": case_id,
            "step_id": step_id,
            "status": status,
        }
        if message:
            end_event["message"] = message
        events.append(end_event)
        self._emit(end_event)
        result: dict[str, Any] = {
            "ok": ok,
            "step_id": step_id,
            "events": events,
            "exchanges": public_exchanges,
            "vars": dict(self._vars),
        }
        if message:
            result["error"] = message
        return result

    def call(self, template_id: str) -> dict[str, Any]:
        """Render one ``.json.j2`` envelope, send HTTP, extract, and assert.

        ``call`` takes only a template identifier. Captures and prior extract
        values must already be in :attr:`vars`.
        """
        source_path = self._resolve_template(template_id)
        source = source_path.read_text(encoding="utf-8")
        env = self._jinja_env()
        request_envelope = self._render_json(env, source, response=PendingResponse())
        method = str(request_envelope.get("method") or "").strip()
        url = str(request_envelope.get("url") or "").strip()
        if not method or not url:
            raise ApiError(f"{template_id}: envelope must include method and url")
        headers = request_envelope.get("headers") or {}
        if not isinstance(headers, dict):
            raise ApiError(f"{template_id}: headers must be an object")
        headers = {str(k): str(v) for k, v in headers.items()}
        body = request_envelope.get("body")
        timeout_ms = request_envelope.get("timeout_ms")
        if timeout_ms is not None:
            timeout_ms = int(timeout_ms)

        started = time.perf_counter()
        assert self.http_send is not None
        result = self.http_send(method, url, headers, body, timeout_ms)
        duration_ms = int((time.perf_counter() - started) * 1000)

        response_view = ResponseView(result)
        full_envelope = self._render_json(env, source, response=response_view)
        extract = full_envelope.get("extract") or {}
        if extract and not isinstance(extract, dict):
            raise ApiError(f"{template_id}: extract must be an object")
        extracted: dict[str, Any] = {}
        if isinstance(extract, dict):
            for key, value in extract.items():
                extracted[str(key)] = value
                self._vars[str(key)] = value

        asserts = full_envelope.get("assert") or {}
        assert_results: list[dict[str, Any]] = []
        failed = False
        if asserts:
            if not isinstance(asserts, dict):
                raise ApiError(f"{template_id}: assert must be an object")
            for name, value in asserts.items():
                passed = _assert_passed(value)
                assert_results.append({"name": str(name), "passed": passed, "value": value})
                if not passed:
                    failed = True

        exchange_id = str(uuid.uuid4())
        raw = {
            "type": "http_exchange",
            "exchange_id": exchange_id,
            "case_id": self._current_case_id,
            "step_id": self._current_step_id,
            "template": self._rel_template(source_path),
            "method": method.upper(),
            "url": url,
            "request_headers": headers,
            "request_body": body,
            "status": result.status,
            "response_headers": result.headers,
            "response_body": result.json_value if result.json_value is not None else result.body,
            "duration_ms": duration_ms,
            "extract": extracted,
            "asserts": assert_results,
            "redacted": False,
        }
        self._exchanges[exchange_id] = raw
        self._emit(self._public_exchange(exchange_id))
        if failed:
            failed_names = [item["name"] for item in assert_results if not item["passed"]]
            raise ApiError(f"{template_id}: assertion failed: {', '.join(failed_names)}")
        return raw

    def get_exchange(self, exchange_id: str, *, redact: bool = True) -> dict[str, Any]:
        """Return a stored exchange; default path is redacted."""
        raw = self._exchanges.get(exchange_id)
        if raw is None:
            raise ApiError(f"unknown exchange {exchange_id}")
        if redact:
            return redact_exchange(raw, self._redact_keys)
        copy = dict(raw)
        copy["redacted"] = False
        return copy

    def _public_exchange(self, exchange_id: str) -> dict[str, Any]:
        return redact_exchange(self._exchanges[exchange_id], self._redact_keys)

    def _emit(self, event: dict[str, Any]) -> None:
        if self.ndjson_out is None:
            return
        self.ndjson_out.write(json.dumps(event, ensure_ascii=False) + "\n")
        self.ndjson_out.flush()

    def _jinja_env(self) -> Environment:
        env = SandboxedEnvironment(
            loader=RootBoundLoader(self._template_roots),
            undefined=StrictUndefined,
            autoescape=False,
        )
        env.globals.clear()
        return env

    def _render_json(self, env: Environment, source: str, response: Any) -> dict[str, Any]:
        try:
            rendered = env.from_string(source).render(response=response, **self._vars)
        except TemplateNotFound as exc:
            raise ApiError(f"template include {exc} is not under the API template roots") from exc
        except Exception as exc:
            raise ApiError(f"Jinja2 render failed: {exc}") from exc
        try:
            parsed = json.loads(rendered)
        except json.JSONDecodeError as exc:
            raise ApiError(f"rendered envelope is not JSON: {exc}") from exc
        if not isinstance(parsed, dict):
            raise ApiError("rendered envelope must be a JSON object")
        return parsed

    def _resolve_template(self, template_id: str) -> Path:
        rel = template_id.replace("\\", "/")
        for root in self._template_roots:
            candidate = (root / rel).resolve()
            try:
                candidate.relative_to(root)
            except ValueError as exc:
                raise ApiError(f"template {template_id!r} resolves outside template roots") from exc
            if candidate.is_file():
                return candidate
        raise ApiError(f"template not found: {template_id}")

    def _rel_template(self, path: Path) -> str:
        for root in self._template_roots:
            try:
                return path.relative_to(root).as_posix()
            except ValueError:
                continue
        return path.name


class StepContext:
    """Behave-like context passed to step defs; ``context.api`` is the session."""

    def __init__(self, session: Session) -> None:
        self.api = session
        self.vars = session.vars


_ACTIVE: Session | None = None
_PENDING_WHEN: list[tuple[str, Callable[..., Any]]] = []


def configure(project_root: str | Path, http_send: HttpSend | None = None, ndjson_out: TextIO | None = None) -> Session:
    """Create (or replace) the process-wide session used by ``call`` / ``when``."""
    global _ACTIVE
    session = Session(Path(project_root), http_send=http_send, ndjson_out=ndjson_out)
    for pattern, func in _PENDING_WHEN:
        session.register_when(pattern, func)
    _ACTIVE = session
    return session


def get_session() -> Session:
    """Return the process-wide session, requiring :func:`configure` first."""
    if _ACTIVE is None:
        raise ApiError("teshi_api.configure(project_root) must be called before call()")
    return _ACTIVE


def call(template_id: str) -> dict[str, Any]:
    """Invoke :meth:`Session.call` on the process-wide session."""
    return get_session().call(template_id)


def when(pattern: str) -> Callable[[Callable[..., Any]], Callable[..., Any]]:
    """Register a step definition on the active session, or queue until configure."""

    def decorator(func: Callable[..., Any]) -> Callable[..., Any]:
        if _ACTIVE is not None:
            return _ACTIVE.register_when(pattern, func)
        _PENDING_WHEN.append((pattern, func))
        try:
            from behave import when as behave_when

            return behave_when(pattern)(func)
        except ImportError:
            return func

    return decorator


given = when
then = when
