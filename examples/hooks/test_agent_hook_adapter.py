from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import agent_hook_adapter as adapter  # noqa: E402


class AdapterMappingTests(unittest.TestCase):
    def assert_sensitive_values_absent(self, envelope: dict[str, object], *values: str) -> None:
        encoded = json.dumps(envelope, sort_keys=True)
        for value in values:
            self.assertNotIn(value, encoded)

    def test_claude_hook_drops_prompt_tool_arguments_results_and_reasoning(self) -> None:
        envelope = adapter.map_claude(
            {
                "hook_event_name": "PreToolUse",
                "session_id": "claude-session",
                "tool_name": "Bash",
                "prompt": "PRIVATE USER PROMPT",
                "tool_input": {"command": "cat /private/secret"},
                "tool_response": "PRIVATE TOOL RESULT",
                "reasoning": "PRIVATE REASONING",
            },
            model_override="test-claude-model",
            pid=123,
            resources={
                "cpu_percent": 12.5,
                "rss_bytes": 64 * 1024 * 1024,
                "memory_percent": 1.5,
            },
        )
        self.assertIsNotNone(envelope)
        assert envelope is not None
        self.assertEqual(envelope["kind"], "tool_started")
        self.assertEqual(envelope["tool_name"], "Bash")
        self.assertEqual(envelope["cpu_percent"], 12.5)
        self.assertFalse(envelope["control_capable"])
        self.assert_sensitive_values_absent(
            envelope,
            "PRIVATE USER PROMPT",
            "cat /private/secret",
            "PRIVATE TOOL RESULT",
            "PRIVATE REASONING",
        )

    def test_gemini_hook_drops_model_request_and_response_content(self) -> None:
        envelope = adapter.map_gemini(
            {
                "hookEventName": "AfterModel",
                "sessionId": "gemini-session",
                "llm_request": {"prompt": "PRIVATE GEMINI PROMPT"},
                "llm_response": {"text": "PRIVATE GEMINI RESPONSE"},
            },
            model_override="test-gemini-model",
        )
        self.assertIsNotNone(envelope)
        assert envelope is not None
        self.assertEqual(envelope["kind"], "model_response")
        self.assert_sensitive_values_absent(
            envelope,
            "PRIVATE GEMINI PROMPT",
            "PRIVATE GEMINI RESPONSE",
        )

    def test_codex_skips_agent_messages_and_reasoning(self) -> None:
        token_state: dict[str, tuple[int, int]] = {}
        for item_type in ("agentMessage", "reasoning", "userMessage"):
            envelope = adapter.map_codex_notification(
                {
                    "method": "item/completed",
                    "params": {
                        "threadId": "thr-1",
                        "turnId": "turn-1",
                        "item": {
                            "id": f"item-{item_type}",
                            "type": item_type,
                            "text": "PRIVATE CODEX CONTENT",
                        },
                    },
                },
                token_state=token_state,
            )
            self.assertIsNone(envelope)

    def test_codex_tool_event_exposes_type_not_command_or_arguments(self) -> None:
        envelope = adapter.map_codex_notification(
            {
                "method": "item/started",
                "params": {
                    "threadId": "thr-1",
                    "turnId": "turn-1",
                    "item": {
                        "id": "item-1",
                        "type": "commandExecution",
                        "command": "cat /private/secret",
                        "arguments": {"danger": "PRIVATE ARGUMENT"},
                    },
                },
            },
            token_state={},
            model_override="test-codex-model",
        )
        self.assertIsNotNone(envelope)
        assert envelope is not None
        self.assertEqual(envelope["kind"], "tool_started")
        self.assertEqual(envelope["tool_name"], "commandExecution")
        self.assert_sensitive_values_absent(
            envelope,
            "cat /private/secret",
            "PRIVATE ARGUMENT",
        )

    def test_codex_cumulative_usage_becomes_deltas_without_double_counting(self) -> None:
        state: dict[str, tuple[int, int]] = {}
        first = adapter.map_codex_notification(
            {
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": "thr-usage",
                    "tokenUsage": {
                        "total": {"inputTokens": 100, "outputTokens": 25}
                    },
                },
            },
            token_state=state,
        )
        second = adapter.map_codex_notification(
            {
                "method": "thread/tokenUsage/updated",
                "params": {
                    "threadId": "thr-usage",
                    "tokenUsage": {
                        "total": {"inputTokens": 130, "outputTokens": 40}
                    },
                },
            },
            token_state=state,
        )
        self.assertIsNotNone(first)
        self.assertIsNotNone(second)
        assert first is not None and second is not None
        self.assertEqual(first["input_tokens_delta"], 100)
        self.assertEqual(first["output_tokens_delta"], 25)
        self.assertEqual(second["input_tokens_delta"], 30)
        self.assertEqual(second["output_tokens_delta"], 15)
        self.assertNotEqual(first["event_id"], second["event_id"])


if __name__ == "__main__":
    unittest.main()
