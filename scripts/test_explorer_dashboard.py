#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (ROOT / "scripts/explorer-dashboard.js").read_text(encoding="utf-8")
UI = (ROOT / "src/explorer_ui.rs").read_text(encoding="utf-8")
API = (ROOT / "src/explorer_api.rs").read_text(encoding="utf-8")


class ExplorerDashboardContractTests(unittest.TestCase):
    def test_websocket_is_same_origin_and_token_is_first_message_only(self) -> None:
        self.assertIn("const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';", SCRIPT)
        self.assertIn("new WebSocket(`${scheme}//${location.host}/ws/ui`)", SCRIPT)
        self.assertIn("socket.send(JSON.stringify({ token: token() }))", SCRIPT)
        self.assertNotIn("?token=", SCRIPT)
        self.assertNotIn("encodeURIComponent(token", SCRIPT)

    def test_fetch_is_read_only_authenticated_and_server_bounded(self) -> None:
        for snippet in (
            "fetch(explorerUrl()",
            "headers: headers()",
            "cache: 'no-store'",
            "new URLSearchParams",
            "timeline_limit",
            "session_limit",
            "lesson_limit",
            "boundedInput('timeline-limit', 1, 250, 100)",
            "boundedInput('session-limit', 1, 250, 100)",
            "boundedInput('lesson-limit', 1, 1000, 250)",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, SCRIPT)
        for method in ("POST", "PUT", "PATCH", "DELETE"):
            with self.subTest(method=method):
                self.assertNotIn(f"method: '{method}'", SCRIPT)
                self.assertNotIn(f'method: "{method}"', SCRIPT)

    def test_revision_and_lag_updates_are_coalesced_with_bounded_reconnect(self) -> None:
        for snippet in (
            "message.kind === 'authenticated'",
            "message.kind === 'resync_required'",
            "message.revision > state.snapshot.revision",
            "if (state.refreshing)",
            "state.dirty = true",
            "if (state.refreshTimer) return",
            "}, 100)",
            "Math.min(15000, 600 * (2 ** state.retry++))",
            "setInterval(refresh, 30000)",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, SCRIPT)

    def test_retention_limits_and_omissions_are_visible(self) -> None:
        for identifier in (
            'id="timeline-limit"',
            'id="session-limit"',
            'id="lesson-limit"',
            'id="retention"',
            'id="stat-sessions"',
            'id="stat-events"',
            'id="stat-lessons"',
        ):
            with self.subTest(identifier=identifier):
                self.assertIn(identifier, UI)
        self.assertIn("snapshot.retention.returned_timeline_events", SCRIPT)
        self.assertIn("snapshot.retention.total_timeline_events", SCRIPT)
        self.assertIn("snapshot.retention.returned_sessions", SCRIPT)
        self.assertIn("snapshot.retention.returned_lessons", SCRIPT)

    def test_all_visible_dynamic_fields_are_escaped(self) -> None:
        for snippet in (
            "${esc(agent.display_name || agent.agent.agent_id)}",
            "${esc(agent.agent.agent_id)}",
            "${esc(session.session_id)}",
            "${esc(record.event.event_id)}",
            "${esc(record.event.agent.agent_id)}",
            "${esc(JSON.stringify(record.event.data))}",
            "${esc(item.lesson.statement)}",
            "${esc(item.lesson.lesson_id)}",
            "Object.entries(system.caches).map",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, SCRIPT)

    def test_filter_operates_on_retained_client_state_only(self) -> None:
        self.assertIn("state.query = event.target.value.trim().toLowerCase()", SCRIPT)
        self.assertIn("render()", SCRIPT)
        self.assertNotIn("search=", SCRIPT)
        self.assertNotIn("filter=", SCRIPT)

    def test_static_shell_and_api_route_preserve_privacy_boundary(self) -> None:
        self.assertIn("retention-aware", UI)
        self.assertIn("does not dispatch agents", UI)
        self.assertIn('route("/explorer", get(page))', API)
        self.assertIn('route("/api/v1/explorer", get(explorer))', API)
        self.assertIn("authorize_read", API)
        self.assertNotIn("ExplorerSnapshot {", UI)


if __name__ == "__main__":
    unittest.main()
