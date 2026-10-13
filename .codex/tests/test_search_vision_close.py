import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("scenarios_new", Path(__file__).parents[1] / "tools/check_scenarios.py")
scenarios = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scenarios)


class SearchVisionClose(unittest.TestCase):
    def test_search_requires_the_observed_google_query(self):
        case = {"expected_query": "a b&c"}
        check = lambda url: scenarios.observation_ok({"kind": "search_opened", "url": url}, case, {})
        self.assertTrue(check("https://www.google.com/search?q=a%20b%26c"))
        self.assertFalse(check("https://yandex.ru/search/?text=a%20b%26c"))
        self.assertFalse(check("https://www.google.com.evil/search?q=a%20b%26c"))
        self.assertFalse(check("https://www.google.com/search?q=wrong"))
        self.assertFalse(check("https://www.google.com/search?q=a%20b%26c&q=wrong"))

    def test_counted_request_and_headers_are_not_a_screen_description(self):
        case = {"expected_model": "gemini-flash-lite-latest"}
        event = {"kind": "screen_description", "requested_model": case["expected_model"], "http_status": 200, "timed_out": False}
        self.assertFalse(scenarios.observation_ok(event, case, {}))
        event["description"] = "Открыт PDF."
        self.assertTrue(scenarios.observation_ok(event, case, {}))
        event["timed_out"] = True
        self.assertFalse(scenarios.observation_ok(event, case, {}))

    def test_short_reply_needs_the_corresponding_window_state(self):
        check = scenarios.observation_ok
        case = {"expected_speech": "Окно закрыто."}
        event = {"kind": "window_closed_reply", "speech": case["expected_speech"], "closed": False}
        self.assertFalse(check(event, case, {}))
        event["closed"] = True
        self.assertTrue(check(event, case, {}))
        event["speech"] += " Программа могла остаться в фоне."
        self.assertFalse(check(event, case, {}))
        case = {"expected_speech": "Открытого окна нет."}
        event = {"kind": "background_close_reply", "speech": case["expected_speech"], "window_found": False, "process_found": True}
        self.assertTrue(check(event, case, {}))
        event["window_found"] = True
        self.assertFalse(check(event, case, {}))
