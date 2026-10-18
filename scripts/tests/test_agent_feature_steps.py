"""History bindings must wait for restoration, not previously visible text."""

import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch


spec = importlib.util.spec_from_file_location(
    "agent_feature_steps",
    Path(__file__).parents[2]
    / "crates/teshi-tui-runner/tests/fixtures/agent_feature_steps.py",
)
bindings = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bindings)


class RestoringWorld(bindings.World):
    def __init__(self, completion):
        self.completion = completion
        self.ready = False
        self.polls = 0

    def _session_records(self):
        return [{"updated_at": "100", "messages": [
            {"content": "saved history"}, {"content": "saved answer"}
        ]}]

    def command(self, text):
        if text != "/resume":
            raise AssertionError("unexpected command")

    def screen(self):
        self.polls += 1
        if self.polls < 4:
            # The old transcript is visible while input is still modal-owned.
            return "▌ saved history\nProject sessions\nWaiting for the previous Agent to exit"
        self.ready = True
        return "▌ saved history\n" + self.completion


class HistoryRestorationTests(unittest.TestCase):
    def test_visible_history_does_not_mean_restoration_has_finished(self):
        for completion in ("", "Loaded session with 2 messages",
                           "Resumed session from 2 messages"):
            with self.subTest(completion=completion):
                world = RestoringWorld(completion)
                with patch.object(bindings.time, "sleep"):
                    world.resume()
                self.assertTrue(world.ready,
                                "returned while the shutdown-wait modal still owns input")

    def test_command_waits_for_input_acceptance_and_consumption(self):
        class QueuedInputWorld(bindings.World):
            def __init__(self):
                self.phase = "idle"
                self.polls = 0
                self.accepted = False
                self.consumed = False

            def focus_input(self):
                pass

            def op(self, operation, text=""):
                if operation == "write":
                    self.typed = text
                    self.phase = "queued"
                elif operation == "key":
                    if not self.accepted:
                        raise AssertionError("submitted before the application accepted input")
                    self.phase = "submitted"
                    self.polls = 0

            def screen(self):
                self.polls += 1
                if self.phase == "queued":
                    if self.polls < 3:
                        # A completion row can match the command before input does.
                        return "/agent Select session Agent\n│Type your message..."
                    self.accepted = True
                    return "│" + self.typed
                if self.polls < 2:
                    return "│" + self.typed
                self.consumed = True
                return "Select Agent for this session\n│Type your message..."

        world = QueuedInputWorld()
        with patch.object(bindings.time, "sleep"):
            world.command("/agent")
        self.assertTrue(world.consumed, "returned before the command was processed")


if __name__ == "__main__":
    unittest.main()
