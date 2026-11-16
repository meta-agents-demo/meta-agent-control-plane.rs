import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "observe_host_agents.py"
SPEC = importlib.util.spec_from_file_location("observe_host_agents", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class HostObserverTests(unittest.TestCase):
    def test_parser_reports_only_agent_process_columns(self):
        rows = MODULE.parse_ps_lines(
            [
                "  101 1 101 2.5 4096 0.1 S /Applications/ChatGPT.app/Contents/MacOS/ChatGPT",
                "  202 101 101 0.0 2048 0.0 S /opt/bin/codex",
                "  303 1 303 1.0 1024 0.0 S /usr/bin/unrelated",
                "  404 1 404 0.5 512 0.0 S /Users/test/bin/ai-agent-bridge",
            ]
        )

        self.assertEqual([row["pid"] for row in rows], [101, 202, 404])
        self.assertEqual(rows[0]["rss_bytes"], 4096 * 1024)
        self.assertEqual(rows[0]["process_role"], "app")
        self.assertEqual(rows[1]["provider"], "openai")
        self.assertEqual(rows[2]["process_role"], "bridge")
        self.assertNotIn("arguments", rows[0])

    def test_claude_is_classified_as_anthropic_cli(self):
        self.assertEqual(
            MODULE.classify_process("/opt/homebrew/bin/claude"),
            ("anthropic", "agent_cli"),
        )

    def test_child_executable_is_not_mislabeled_as_codex_cli(self):
        self.assertEqual(
            MODULE.classify_process(
                "/Applications/ChatGPT.app/Contents/Frameworks/Codex/browser_crashpad_handler"
            ),
            ("openai", "agent_service"),
        )


if __name__ == "__main__":
    unittest.main()
