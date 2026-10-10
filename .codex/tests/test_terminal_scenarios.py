import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("scenarios", Path(__file__).parents[1] / "tools/check_scenarios.py")
scenarios = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scenarios)


class TerminalScenarios(unittest.TestCase):
    def test_sent_keys_are_not_proof_of_visible_input(self):
        case = {"expected_text": "uv init"}
        check = scenarios.observation_ok
        self.assertFalse(check({"kind": "terminal_input", "sent": True}, case, {}))
        self.assertFalse(check({"kind": "terminal_input", "text": "PS> uv init", "changed": False}, case, {}))
        self.assertFalse(check({"kind": "terminal_input", "text": "PS> uv init\nPS>", "changed": True}, case, {}))
        self.assertTrue(check({"kind": "terminal_input", "text": "PS> uv init", "changed": True}, case, {}))

    def test_terminal_text_and_cancelled_synthesis_need_observations(self):
        check = scenarios.observation_ok
        case = {"expected_text": "JARVIS_TEST"}
        self.assertFalse(check({"kind": "terminal_output", "text": "JARVIS_TEST"}, case, {}))
        self.assertTrue(check({"kind": "terminal_output", "text": "PS> JARVIS_TEST", "source": "uia"}, case, {}))
        self.assertFalse(check({"kind": "speech_stopped", "stopped": True, "pending_audio": True}, {}, {}))
        self.assertTrue(check({"kind": "speech_stopped", "stopped": True, "pending_audio": False}, {}, {}))
