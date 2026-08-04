#!/usr/bin/env python3
from __future__ import annotations

from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (ROOT / "scripts/coordination-dashboard.js").read_text(encoding="utf-8")
UI = (ROOT / "src/coordination_ui.rs").read_text(encoding="utf-8")


class CoordinationDashboardContractTests(unittest.TestCase):
    def test_websocket_is_same_origin_and_token_is_a_first_message_frame(self) -> None:
        self.assertIn("const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';", SCRIPT)
        self.assertIn("new WebSocket(`${scheme}//${location.host}/ws/ui`)", SCRIPT)
        self.assertIn("socket.send(JSON.stringify({ token: token() }))", SCRIPT)
        self.assertNotIn("?token=", SCRIPT)
        self.assertNotIn("encodeURIComponent(token", SCRIPT)

    def test_revision_updates_and_lag_notices_coalesce_refreshes(self) -> None:
        for snippet in (
            "message.kind === 'authenticated'",
            "message.kind === 'resync_required'",
            "Number.isInteger(message.revision)",
            "message.revision > state.plan.revision",
            "if (state.refreshing)",
            "state.dirty = true",
            "if (state.refreshTimer) return",
            "setTimeout(() =>",
            "}, 100)",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, SCRIPT)

    def test_reconnect_backoff_is_bounded_and_polling_is_only_a_safety_net(self) -> None:
        self.assertIn("Math.min(15000, 600 * (2 ** state.retry++))", SCRIPT)
        self.assertIn("setTimeout(connect, delay)", SCRIPT)
        self.assertIn("setInterval(refresh, 30000)", SCRIPT)
        self.assertNotIn("setInterval(refresh, 5000)", SCRIPT)

    def test_token_changes_restart_both_stream_and_snapshot_read(self) -> None:
        save_handler = SCRIPT[SCRIPT.index("$('save-token').addEventListener") :]
        for snippet in (
            "sessionStorage.setItem('meta-agent-read-token', value)",
            "sessionStorage.removeItem('meta-agent-read-token')",
            "state.retry = 0",
            "connect()",
            "refresh()",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, save_handler)

    def test_dynamic_plan_fields_remain_html_escaped(self) -> None:
        for snippet in (
            "${esc(item.agent_id)}",
            "${esc(item.task_id)}",
            "${esc(item.rationale)}",
            "${esc(item.recommended_action)}",
            "${esc(item.explanation)}",
            "item.unresolved_dependencies.map(esc)",
        ):
            with self.subTest(snippet=snippet):
                self.assertIn(snippet, SCRIPT)

    def test_static_shell_exposes_stream_state_without_plan_payload(self) -> None:
        self.assertIn('id="stream-indicator"', UI)
        self.assertIn('id="stream-label"', UI)
        self.assertIn("advisory and read-only", UI)
        self.assertNotIn("CoordinationPlan {", UI)


if __name__ == "__main__":
    unittest.main()
