import importlib.util
import pathlib
import unittest


SCRIPT = pathlib.Path(__file__).parents[1] / "scripts" / "bridge_peer.py"
SPEC = importlib.util.spec_from_file_location("bridge_peer", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class BridgePeerTests(unittest.TestCase):
    def test_prompt_labels_transcript_untrusted_and_forbids_tools(self):
        prompt = MODULE.build_prompt(
            {
                "room": {"objective": "Cross-check the design"},
                "messages": [
                    {
                        "author": {
                            "display_name": "Human",
                            "participant_id": "human",
                        },
                        "summary": "Inspect the evidence",
                    }
                ],
            },
            "bridge-codex",
        )
        self.assertIn("untrusted visible summaries", prompt)
        self.assertIn("Do not use any tools", prompt)
        self.assertIn("Cross-check the design", prompt)
        self.assertIn("Human (human): Inspect the evidence", prompt)

    def test_failure_summary_distinguishes_credits_from_authentication(self):
        self.assertIn("credit", MODULE.failure_summary("Credit balance exhausted"))
        self.assertIn("authentication", MODULE.failure_summary("Authentication required"))
        self.assertIn(
            "disabled", MODULE.failure_summary("Organization has disabled subscription access")
        )

    def test_provider_environment_removes_only_relevant_api_key(self):
        original = MODULE.os.environ.copy()
        try:
            MODULE.os.environ["OPENAI_API_KEY"] = "not-for-test-use"
            MODULE.os.environ["ANTHROPIC_API_KEY"] = "not-for-test-use"
            openai = MODULE.sanitized_environment("openai")
            anthropic = MODULE.sanitized_environment("anthropic")
            self.assertNotIn("OPENAI_API_KEY", openai)
            self.assertNotIn("ANTHROPIC_API_KEY", anthropic)
        finally:
            MODULE.os.environ.clear()
            MODULE.os.environ.update(original)

    def test_poll_responds_only_to_newest_foreign_message_and_marks_backlog_seen(self):
        seen = set()
        snapshot = {
            "messages": [
                {"message_id": "older", "author": {"participant_id": "human"}},
                {"message_id": "own", "author": {"participant_id": "bridge-codex"}},
                {"message_id": "newest", "author": {"participant_id": "bridge-claude"}},
            ]
        }
        selected = MODULE.newest_unseen_foreign_message(
            snapshot, "bridge-codex", seen
        )
        self.assertEqual(selected["message_id"], "newest")
        self.assertEqual(seen, {"older", "newest"})
        self.assertIsNone(
            MODULE.newest_unseen_foreign_message(snapshot, "bridge-codex", seen)
        )


if __name__ == "__main__":
    unittest.main()
