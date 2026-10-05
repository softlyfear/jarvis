"""Behavioral tests for evidence, action outcomes, logs and benchmark measurements."""

import copy
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import threading
import unittest
import wave
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import benchmark_voice
import check_evidence
import check_scenarios
import review_logs


def trace(case, events):
    return {"schema_version": 1, "case_id": case, "provenance": "fixture", "inputs": {"desktop_dir": "C:/Users/Test/Desktop", "expected_content_sha256": hashlib.sha256(b"new poem").hexdigest()}, "events": events}


def tool(name="press_keys", ok=True):
    return {"type": "tool", "name": name, "ok": ok, "arguments": {}}


def saved(changed=True):
    digest = hashlib.sha256(b"new poem").hexdigest()
    return {"type": "observation", "kind": "file_saved", "path": "C:/Users/Test/Desktop/123.txt", "changed": changed, "content_sha256": digest}


class ScenarioTests(unittest.TestCase):
    def test_saved_only_after_matching_content_and_change(self):
        self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [tool(), saved(), {"type": "claim", "success": True}]))["status"], "pass")
        for observation in (saved(False), {**saved(), "content_sha256": "a" * 64}, {**saved(), "content_sha256": "x" * 64, "expected_sha256": "x" * 64}, {**saved(), "path": "C:/Temp/unrelated.txt"}, {**saved(), "path": "C:/Users/Test/Desktop/other.txt"}):
            self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [tool(), observation]))["status"], "fail")

    def test_expected_path_and_content_come_from_scenario_inputs(self):
        obs = {"type": "observation", "kind": "file_opened", "opened": True, "path": "C:/Users/Test/Desktop/resume.pdf"}
        self.assertEqual(check_scenarios.evaluate(trace("open-local-pdf", [tool("open_file"), obs]))["status"], "pass")
        self.assertEqual(check_scenarios.evaluate(trace("open-local-pdf", [tool("open_file"), {**obs, "path": "C:/Temp/resume.pdf"}]))["status"], "fail")
        self.assertEqual(check_scenarios.evaluate(trace("open-local-pdf", [tool("open_file"), {**obs, "path": "C:/Users/Test/Desktop/other.pdf", "expected_path": "C:/Users/Test/Desktop/other.pdf"}]))["status"], "fail")

    def test_sent_keys_and_existing_file_do_not_prove_save(self):
        result = check_scenarios.evaluate(trace("save-new-file", [tool(), {"type": "claim", "success": True}]))
        self.assertEqual(result["status"], "fail")
        self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [tool()]))["status"], "unknown")

    def test_pre_action_or_stale_observation_does_not_prove_result(self):
        for events in ([saved(), tool(), {"type": "claim", "success": True}], [tool(), saved(), tool(), {"type": "claim", "success": True}]):
            self.assertEqual(check_scenarios.evaluate(trace("save-new-file", events))["status"], "fail")

    def test_read_only_inspection_keeps_verified_state(self):
        self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [tool(), saved(), tool("inspect_window"), {"type": "claim", "success": True}]))["status"], "pass")

    def test_successful_recovery_is_allowed(self):
        self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [tool(ok=False), tool(), saved()]))["status"], "pass")

    def test_unrecovered_error_fails(self):
        self.assertEqual(check_scenarios.evaluate(trace("open-local-pdf", [tool("open_app", False)]))["status"], "fail")

    def test_browser_must_be_focused_without_new_urls(self):
        obs = {"type": "observation", "kind": "browser_focused", "focused": True, "opened_urls": []}
        self.assertEqual(check_scenarios.evaluate(trace("focus-browser", [tool("focus_app"), obs]))["status"], "pass")
        self.assertEqual(check_scenarios.evaluate(trace("focus-browser", [tool("open_app"), {**obs, "opened_urls": ["https://ya.ru"]}]))["status"], "fail")

    def test_delete_file_never_satisfies_replace_content_even_with_confirmation(self):
        obs = {"type": "observation", "kind": "text_replaced", "changed": True, "file_deleted": False}
        self.assertEqual(check_scenarios.evaluate(trace("replace-content", [tool("delete_file"), obs]))["status"], "fail")
        self.assertEqual(check_scenarios.evaluate(trace("replace-content", [tool("type_text"), obs]))["status"], "pass")

    def test_wrong_pdf_and_wrong_ipc_port_fail(self):
        samples = [
            ("open-local-pdf", {"kind": "file_opened", "opened": True, "path": "other.pdf", "expected_path": "resume.pdf"}),
            ("ipc-restart", {"kind": "ipc_ready", "connected": True, "port": 5055}),
            ("voice-startup", {"kind": "voice_ready", "stt_ready": True, "tts_ready": False}),
        ]
        for case, obs in samples:
            self.assertEqual(check_scenarios.evaluate(trace(case, [tool(), {"type": "observation", **obs}]))["status"], "fail")

    def test_fixture_never_counts_as_hardware(self):
        result = check_scenarios.evaluate(trace("save-new-file", [tool(), saved()]))
        self.assertEqual(result["status"], "pass")
        self.assertFalse(result["hardware_verified"])

    def test_observed_requires_intact_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "capture.json"
            capture = {**trace("save-new-file", [tool(), saved()]), "provenance": "observed", "environment": {"os": "Windows"}, "observer": "file-hash-check"}
            source.write_text(json.dumps(capture))
            item = {**capture, "source": "capture.json", "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            self.assertTrue(check_scenarios.evaluate(item, root=root)["hardware_verified"])
            source.write_bytes(b"changed")
            self.assertEqual(check_scenarios.evaluate(item, root=root)["status"], "fail")
            item["source"] = "../capture.json"
            self.assertEqual(check_scenarios.evaluate(item, root=root)["status"], "fail")

    def test_fixture_source_and_incomplete_tool_cannot_prove_hardware(self):
        path = check_scenarios.ROOT / ".codex/scenarios/observed-2026-10-05.json"
        item = {**trace("save-new-file", [tool(), saved()]), "provenance": "observed", "source": str(path.relative_to(check_scenarios.ROOT)), "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        self.assertEqual(check_scenarios.evaluate(item)["status"], "fail")
        self.assertEqual(check_scenarios.evaluate(trace("save-new-file", [{"type": "tool"}, saved()]))["status"], "fail")

    def test_empty_trace_package_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "empty.json"
            path.write_text('{"traces": []}')
            result = subprocess.run([sys.executable, str(check_scenarios.ROOT / ".codex/tools/check_scenarios.py"), "--trace", str(path)], capture_output=True)
            self.assertEqual(result.returncode, 1)

    def test_real_log_reconstruction_keeps_all_known_failures_visible(self):
        data = json.loads((check_scenarios.ROOT / ".codex/scenarios/observed-2026-10-05.json").read_text(encoding="utf-8"))
        self.assertEqual(len(data["traces"]), 6)
        for item in data["traces"]:
            self.assertEqual(check_scenarios.evaluate(item)["status"], "fail")


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / ".codex").mkdir()
        self.artifact = self.root / ".codex/proof.txt"
        self.artifact.write_bytes(b"tests passed")
        self.source = self.root / ".codex/source.py"
        self.source.write_bytes(b"implementation")
        proof = {"kind": "test", "result": "pass", "artifact": ".codex/proof.txt", "sha256": hashlib.sha256(self.artifact.read_bytes()).hexdigest(), "summary": "observable check", "exit_code": 0, "inputs": [{"path": ".codex/source.py", "sha256": hashlib.sha256(self.source.read_bytes()).hexdigest()}]}
        self.doc = {"schema_version": 1, "implementer": "main", "tasks": [{"id": "1", "status": "complete", "requirements": [{"id": "1.a", "status": "verified", "evidence": [proof]}]}]}

    def proof(self):
        return self.doc["tasks"][0]["requirements"][0]["evidence"][0]

    def test_intact_evidence_is_accepted(self):
        self.assertEqual(check_evidence.validate(self.doc, self.root, True), [])

    def test_changed_source_and_changed_artifact_are_rejected(self):
        for path in (self.artifact, self.source):
            old = path.read_bytes()
            path.write_bytes(b"changed")
            self.assertTrue(check_evidence.validate(self.doc, self.root))
            path.write_bytes(old)

    def test_failed_test_and_self_review_are_rejected(self):
        self.proof()["exit_code"] = 1
        self.assertTrue(check_evidence.validate(self.doc, self.root))
        self.proof().update(kind="review", reviewer="main")
        self.assertTrue(check_evidence.validate(self.doc, self.root))
        self.proof()["reviewer"] = "independent"
        self.artifact.write_text(json.dumps({"reviewer": "independent", "verdict": "approve"}))
        self.proof()["sha256"] = hashlib.sha256(self.artifact.read_bytes()).hexdigest()
        self.assertEqual(check_evidence.validate(self.doc, self.root), [])

    def test_rejected_review_and_failed_hardware_are_rejected_even_if_metadata_says_pass(self):
        for kind, payload in (
            ("review", {"reviewer": "independent", "verdict": "reject"}),
            ("hardware", {"provenance": "fixture", "result": "pass", "environment": {"os": "Windows"}}),
            ("hardware", {"provenance": "observed", "result": "fail", "environment": {"os": "Windows"}}),
        ):
            self.proof().update(kind=kind, reviewer="independent", provenance="observed")
            self.artifact.write_text(json.dumps(payload))
            self.proof()["sha256"] = hashlib.sha256(self.artifact.read_bytes()).hexdigest()
            self.assertTrue(check_evidence.validate(self.doc, self.root))

    def test_missing_proofs_pending_requirements_and_fixture_hardware_fail(self):
        doc = copy.deepcopy(self.doc)
        doc["tasks"][0]["requirements"][0]["evidence"] = []
        self.assertTrue(check_evidence.validate(doc, self.root))
        self.doc["tasks"][0]["requirements"][0]["status"] = "hardware_pending"
        self.assertTrue(check_evidence.validate(self.doc, self.root, True))
        self.doc["tasks"][0]["requirements"][0]["status"] = "verified"
        self.proof().update(kind="hardware", provenance="fixture")
        self.assertTrue(check_evidence.validate(self.doc, self.root))

    def test_external_and_symlinked_evidence_are_rejected(self):
        outside = self.root / "outside.txt"
        outside.write_bytes(b"tests passed")
        link = self.root / ".codex/link.txt"
        link.symlink_to(outside)
        self.proof()["artifact"] = ".codex/link.txt"
        self.assertTrue(check_evidence.validate(self.doc, self.root))


class LogTests(unittest.TestCase):
    def test_redaction_masks_config_keys_arrays_bearer_and_tokens(self):
        config = {"llm": {"providers": [{"keys": ["opaque_private_key", "another_private_key"]}]}}
        sample = 'keys = ["opaque_private_key", "another_private_key"] Authorization: Bearer totally_unprefixed_token eyJabc.defghijklmnopqrstuvwxyz sk-abcdefghijklmnopqrstuvwxyz'
        result = review_logs.redact(sample, review_logs.secret_values(config))
        for secret in ("opaque_private_key", "another_private_key", "totally_unprefixed_token", "eyJabc.defghijklmnopqrstuvwxyz", "sk-abcdefghijklmnopqrstuvwxyz"):
            self.assertNotIn(secret, result)

    def test_version_filter_excludes_old_failures_and_keeps_separate_sessions(self):
        content = '\n'.join([
            '2026-10-01 10:00:00.000000000 [INFO ] <app:1>:Starting Jarvis v0.2.1 (build old) ...',
            '2026-10-01 10:00:01.000000000 [ERROR] <app:1>:old failure',
            '2026-10-05 20:00:00.000000000 [INFO ] <app:1>:Starting Jarvis v0.2.63 (build new) ...',
            '2026-10-05 20:00:01.000000000 [INFO ] <app:1>:Whisper (172 ms): hello',
            '2026-10-05 20:01:00.000000000 [INFO ] <app:1>:Starting Jarvis v0.2.63 (build new) ...',
            '2026-10-05 20:01:01.000000000 [WARN ] <app:1>:new failure',
        ])
        result = review_logs.analyze({"log.txt": content}, date="2026-10-05", version="0.2.63")
        self.assertEqual(len(result["sessions"]), 2)
        self.assertEqual(result["sessions"][0]["stt_ms"]["median"], 172)
        self.assertEqual(len(result["sessions"][1]["errors"]), 1)
        self.assertNotIn("old failure", json.dumps(result))

    def test_zip_traversal_and_duplicate_names_are_rejected_without_extraction(self):
        with tempfile.TemporaryDirectory() as directory:
            for filename in ("../log.txt", "/log.txt", "nested\\log.txt"):
                archive = Path(directory) / "logs.zip"
                with zipfile.ZipFile(archive, "w") as z:
                    z.writestr(filename, "anything")
                with self.assertRaises(ValueError):
                    review_logs.read_archive(archive)

    def test_usage_and_arbitrary_debug_payloads_are_not_exported(self):
        content = '2026-10-05 20:00:00.000000000 [INFO ] <app:1>:Starting Jarvis v0.2.63 (build new) ...\n2026-10-05 20:00:01.000000000 [DEBUG] <llm:1>:LLM usage: opaque confidential debug'
        self.assertNotIn("confidential", json.dumps(review_logs.analyze({"log.txt": content})))


class BenchmarkTests(unittest.TestCase):
    def test_wav_duration_and_percentiles(self):
        with io.BytesIO() as output:
            with wave.open(output, "wb") as wav:
                wav.setparams((1, 2, 16000, 16000, "NONE", "not compressed"))
                wav.writeframes(b"\x00\x00" * 16000)
            self.assertEqual(benchmark_voice.wav_seconds(output.getvalue()), 1)
        self.assertEqual(benchmark_voice.percentile([1, 2, 3], .95), 2.9)
        with self.assertRaises(ValueError):
            benchmark_voice.wav_seconds(output.getvalue()[:44])
        with self.assertRaises((wave.Error, EOFError)):
            benchmark_voice.wav_seconds(b"not a WAV")

    def test_local_http_benchmark_records_audio_and_does_not_invent_quality(self):
        buffer = io.BytesIO()
        with wave.open(buffer, "wb") as wav:
            wav.setparams((1, 2, 16000, 0, "NONE", "not compressed"))
            wav.writeframes(b"\x00\x00" * 8000)
        audio = buffer.getvalue()
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                body = json.dumps({"tts": True, "profile": "rocm", "tts_engine": "fixture"}).encode()
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)
            def do_POST(self):
                self.server.requests.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                self.send_response(200)
                self.end_headers()
                self.wfile.write(audio)
            def log_message(self, *args):
                pass
        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        server.requests = []
        worker = threading.Thread(target=server.serve_forever, daemon=True)
        worker.start()
        try:
            with tempfile.TemporaryDirectory() as directory:
                corpus = {"voice": "jarvis-remaster", "phrases": [{"id": "short", "text": "Готово, сэр."}]}
                url = "http://127.0.0.1:" + str(server.server_port)
                output = Path(directory) / "run"
                with self.assertRaises(ValueError):
                    benchmark_voice.run(url, "fixture-model", "rocm", output, 2, corpus)
                result = benchmark_voice.run(url, "fixture-model", "rocm", output, 2, corpus, fixture=True)
                self.assertEqual(len(server.requests), 3)
                self.assertEqual(len(result["runs"]), 2)
                self.assertEqual(result["runs"][0]["audio_seconds"], .5)
                self.assertFalse(result["model_verified_by_server"])
                self.assertEqual(result["provenance"], "fixture")
                self.assertFalse(result["hardware_verified"])
                self.assertIn("Сходство голоса", result["not_measured"])
                self.assertTrue((output / "short-1.wav").exists())
                with self.assertRaises(FileExistsError):
                    benchmark_voice.run(url, "fixture-model", "rocm", output, 2, corpus, fixture=True)
                with self.assertRaises(ValueError):
                    benchmark_voice.run(url, "fixture-model", "cuda", Path(directory) / "wrong", 2, corpus)
                with self.assertRaises(ValueError):
                    benchmark_voice.run(url, "fixture-model", "rocm", Path(directory) / "unsafe", 2, {"voice": "jarvis", "phrases": [{"id": "../outside", "text": "x"}]})
        finally:
            server.shutdown()
            server.server_close()
            worker.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
