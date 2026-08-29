"""Unit tests for Teshi API BDD helper: two-pass Jinja2, extract, assert, sandbox."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path
from typing import Any

RESOURCES = Path(__file__).resolve().parents[1]
FIXTURE = Path(__file__).resolve().parent / "fixtures" / "api-bdd"
sys.path.insert(0, str(RESOURCES))

from teshi_api import (  # noqa: E402
    ApiError,
    HttpResult,
    Session,
    env_var_to_vars_key,
    redact_exchange,
    seed_vars_from_env,
    strip_api_marker,
)


class FakeHttp:
    def __init__(self) -> None:
        self.calls: list[dict[str, Any]] = []
        self._queue: list[HttpResult] = []

    def enqueue(self, result: HttpResult) -> None:
        self._queue.append(result)

    def __call__(
        self,
        method: str,
        url: str,
        headers: dict[str, str],
        body: Any,
        timeout_ms: int | None,
    ) -> HttpResult:
        self.calls.append(
            {
                "method": method,
                "url": url,
                "headers": headers,
                "body": body,
                "timeout_ms": timeout_ms,
            }
        )
        if not self._queue:
            raise AssertionError("unexpected HTTP call")
        return self._queue.pop(0)


def created_user(user_id: str = "42", name: str = "Ada") -> HttpResult:
    payload = {"id": user_id, "name": name}
    return HttpResult(
        status=201,
        headers={"content-type": "application/json"},
        body=json.dumps(payload),
        json_value=payload,
    )


def fetched_user(user_id: str = "42", name: str = "Ada") -> HttpResult:
    payload = {"id": user_id, "name": name}
    return HttpResult(
        status=200,
        headers={"content-type": "application/json"},
        body=json.dumps(payload),
        json_value=payload,
    )


class TeshiApiHelperTests(unittest.TestCase):
    def test_strip_api_marker(self) -> None:
        was_api, rest = strip_api_marker('[API] I create a user named "Ada"')
        self.assertTrue(was_api)
        self.assertEqual(rest, 'I create a user named "Ada"')
        was_api, rest = strip_api_marker("I click save")
        self.assertFalse(was_api)
        self.assertEqual(rest, "I click save")

    def test_env_mapping(self) -> None:
        self.assertEqual(env_var_to_vars_key("TESHI_API_TOKEN"), "token")
        self.assertEqual(env_var_to_vars_key("TESHI_API_BASE_URL"), "base_url")
        self.assertIsNone(env_var_to_vars_key("TESHI_RUNNER_CMD"))
        seeded = seed_vars_from_env({"TESHI_API_TOKEN": "secret", "PATH": "/"})
        self.assertEqual(seeded["token"], "secret")
        self.assertNotIn("PATH", seeded)

    def test_request_render_does_not_require_response(self) -> None:
        http = FakeHttp()
        http.enqueue(created_user())
        session = Session(FIXTURE, http_send=http)
        session.vars["name"] = "Ada"
        session.call("create_user.json.j2")
        self.assertEqual(http.calls[0]["method"], "POST")
        self.assertEqual(http.calls[0]["url"], "https://api.example.test/users")
        self.assertEqual(http.calls[0]["body"]["name"], "Ada")
        self.assertEqual(http.calls[0]["body"]["role"], "member")

    def test_extract_sees_http_response(self) -> None:
        http = FakeHttp()
        http.enqueue(created_user("42"))
        session = Session(FIXTURE, http_send=http)
        session.vars["name"] = "Ada"
        session.call("create_user.json.j2")
        self.assertEqual(session.vars["user_id"], "42")

    def test_extract_chains_across_two_calls(self) -> None:
        http = FakeHttp()
        http.enqueue(created_user("99", "Ada"))
        http.enqueue(fetched_user("99", "Ada"))
        session = Session(FIXTURE, http_send=http)
        session.vars["name"] = "Ada"
        session.call("create_user.json.j2")
        session.call("get_user.json.j2")
        self.assertEqual(http.calls[1]["url"], "https://api.example.test/users/99")
        self.assertEqual(session.vars["fetched_name"], "Ada")

    def test_assert_failure_raises(self) -> None:
        http = FakeHttp()
        http.enqueue(
            HttpResult(status=500, headers={}, body="nope", json_value={"error": "nope"})
        )
        session = Session(FIXTURE, http_send=http)
        session.vars["name"] = "Ada"
        with self.assertRaises(ApiError) as ctx:
            session.call("create_user.json.j2")
        self.assertIn("assertion failed", str(ctx.exception))
        exchange = next(iter(session._exchanges.values()))
        self.assertFalse(exchange["asserts"][0]["passed"])

    def test_include_outside_template_root_is_rejected(self) -> None:
        http = FakeHttp()
        session = Session(FIXTURE, http_send=http)
        with self.assertRaises(ApiError) as ctx:
            session.call("escape_include.json.j2")
        self.assertIn("outside", str(ctx.exception).lower())
        self.assertEqual(http.calls, [])

    def test_redact_authorization_header(self) -> None:
        exchange = {
            "request_headers": {"Authorization": "Bearer super-secret", "Accept": "json"},
            "request_body": {"password": "hidden", "name": "Ada"},
        }
        redacted = redact_exchange(exchange)
        self.assertEqual(redacted["request_headers"]["Authorization"], "***")
        self.assertEqual(redacted["request_headers"]["Accept"], "json")
        self.assertEqual(redacted["request_body"]["password"], "***")
        self.assertEqual(redacted["request_body"]["name"], "Ada")
        self.assertTrue(redacted["redacted"])

    def test_scenario_vars_cleared_between_scenarios(self) -> None:
        http = FakeHttp()
        http.enqueue(created_user("1"))
        session = Session(FIXTURE, http_send=http)
        session.vars["name"] = "Ada"
        session.call("create_user.json.j2")
        self.assertEqual(session.vars["user_id"], "1")
        session.begin_scenario()
        self.assertNotIn("user_id", session.vars)
        self.assertEqual(session.vars["base_url"], "https://api.example.test")

    def test_execute_step_matches_and_correlates_exchanges(self) -> None:
        http = FakeHttp()
        http.enqueue(created_user("7"))
        http.enqueue(fetched_user("7"))
        session = Session(FIXTURE, http_send=http)
        session.load_step_modules()
        session.begin_scenario()
        first = session.execute_step('[API] I create a user named "Ada"', case_id="c1")
        self.assertTrue(first["ok"])
        self.assertEqual(len(first["exchanges"]), 1)
        self.assertEqual(first["exchanges"][0]["step_id"], first["step_id"])
        second = session.execute_step("[API] I fetch that user", case_id="c1")
        self.assertTrue(second["ok"])
        self.assertEqual(session.vars["user_id"], "7")
        self.assertEqual(http.calls[1]["url"], "https://api.example.test/users/7")


if __name__ == "__main__":
    unittest.main()
