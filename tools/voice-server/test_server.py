"""Tests for server.py that do not need the models (run: python -m pytest)."""

import io
import json
import threading
import urllib.error
import urllib.request
import wave
from http.server import ThreadingHTTPServer

import pytest

import server


def make_wav(samples, rate=16000, channels=1):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(channels)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"".join(int(s).to_bytes(2, "little", signed=True) for s in samples))
    return buf.getvalue()


def test_split_text_respects_limit_and_keeps_words():
    text = "Первое предложение. " + "слово " * 80 + "Конец!"
    chunks = server.split_text(text, limit=50)
    assert all(len(c) <= 50 for c in chunks)
    assert " ".join(chunks).split() == text.split()


def test_split_text_merges_short_sentences():
    assert server.split_text("Да. Нет. Может быть.", limit=100) == ["Да. Нет. Может быть."]


def test_to_wav_bytes_is_valid_wav():
    data = server.to_wav_bytes([0.0, 0.5, -0.5, 2.0] * 100, sample_rate=24000)
    with wave.open(io.BytesIO(data)) as w:
        assert w.getframerate() == 24000
        assert w.getnchannels() == 1
        assert w.getnframes() == 400


def test_reference_wavs_found(tmp_path):
    (tmp_path / "a.wav").write_bytes(b"")
    (tmp_path / "b.mp3").write_bytes(b"")
    assert server.find_reference_wavs([tmp_path]) == [str(tmp_path / "a.wav")]


def test_wav_to_float32_decodes_and_validates():
    samples = server.wav_to_float32(make_wav([0, 16384, -32768]))
    assert list(samples) == [0.0, 0.5, -1.0]
    stereo = server.wav_to_float32(make_wav([100, 300], channels=2))
    assert len(stereo) == 1
    with pytest.raises(ValueError):
        server.wav_to_float32(make_wav([0], rate=44100))


def test_hallucinations_are_dropped():
    assert server.clean_transcript("  Открой   телеграм ") == "Открой телеграм"
    assert server.clean_transcript("Субтитры сделал DimaTorzok") == ""
    assert server.clean_transcript("Продолжение следует...") == ""


class FakeRecognizer:
    def __init__(self):
        self.calls = []

    def transcribe_wav(self, data, language="ru"):
        self.calls.append((len(server.wav_to_float32(data)), language))
        return "открой телеграм"


@pytest.fixture
def http_server():
    started = []

    def start(recognizer=None, voice=None):
        srv = ThreadingHTTPServer(("127.0.0.1", 0), server.make_handler(recognizer, voice))
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        started.append(srv)
        return f"http://127.0.0.1:{srv.server_address[1]}"

    yield start
    for srv in started:
        srv.shutdown()


def post(url, body, content_type):
    req = urllib.request.Request(url, data=body, headers={"Content-Type": content_type}, method="POST")
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


def test_stt_endpoint(http_server):
    fake = FakeRecognizer()
    base = http_server(recognizer=fake)
    status, body = post(base + "/stt?language=ru", make_wav([0] * 1600), "audio/wav")
    assert status == 200
    assert json.loads(body) == {"text": "открой телеграм"}
    assert fake.calls == [(1600, "ru")]

    assert post(base + "/stt", make_wav([0], rate=8000), "audio/wav")[0] == 400
    assert post(base + "/stt", b"not a wav", "audio/wav")[0] == 400

    with urllib.request.urlopen(base + "/health") as r:
        assert json.loads(r.read()) == {"ok": True, "stt": True, "tts": False}


def test_disabled_parts_return_503(http_server):
    base = http_server()
    assert post(base + "/stt", make_wav([0]), "audio/wav")[0] == 503
    assert post(base + "/tts", b'{"text": "x"}', "application/json")[0] == 503


class FakeWhisperModel:
    """Stands in for faster_whisper.WhisperModel; CUDA fails like a missing cuBLAS DLL."""

    created = []

    def __init__(self, name, device, compute_type):
        FakeWhisperModel.created.append((name, device, compute_type))
        self.device = device

    def transcribe(self, samples, **kwargs):
        if self.device == "cuda":
            raise RuntimeError("Library cublas64_12.dll is not found")

        class Seg:
            text = " Субтитры сделал DimaTorzok" if len(samples) < 16000 else " Открой  телеграм "

        return iter([Seg()]), None


def test_recognizer_falls_back_to_cpu_small(monkeypatch):
    import sys
    import types

    fake_module = types.SimpleNamespace(WhisperModel=FakeWhisperModel)
    monkeypatch.setitem(sys.modules, "faster_whisper", fake_module)
    FakeWhisperModel.created.clear()

    rec = server.Recognizer("large-v3-turbo", "auto", "int8_float16")
    assert FakeWhisperModel.created == [
        ("large-v3-turbo", "cuda", "int8_float16"),
        ("small", "cpu", "int8"),
    ]
    assert rec.description == "small/cpu"
    assert rec.transcribe_wav(make_wav([0] * 16000)) == "Открой телеграм"
    # a hallucination on a short noise burst is dropped
    assert rec.transcribe_wav(make_wav([0] * 800)) == ""


def test_recognizer_gives_up_when_every_device_fails(monkeypatch):
    import sys
    import types

    monkeypatch.setitem(sys.modules, "faster_whisper", types.SimpleNamespace(WhisperModel=FakeWhisperModel))
    with pytest.raises(RuntimeError):
        server.Recognizer("large-v3-turbo", "cuda", "int8_float16")


def test_model_caches_live_next_to_the_server():
    import os

    assert os.environ["HF_HOME"].endswith(os.path.join("models", "hf"))
    assert os.environ["TTS_HOME"].endswith(os.path.join("models", "tts"))


def test_download_only_uses_both_downloaders(monkeypatch):
    import sys
    import types

    calls = []
    monkeypatch.setitem(sys.modules, "faster_whisper", types.SimpleNamespace(download_model=lambda name: calls.append(("stt", name))))

    class FakeManager:
        def download_model(self, name):
            calls.append(("tts", name))

    monkeypatch.setitem(sys.modules, "TTS.utils.manage", types.SimpleNamespace(ModelManager=FakeManager))
    monkeypatch.setattr(sys, "argv", ["server.py", "--download-only"])
    server.main()
    assert calls == [("stt", "large-v3-turbo"), ("tts", server.XTTS_MODEL)]
