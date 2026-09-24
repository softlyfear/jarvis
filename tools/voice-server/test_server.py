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
        health = json.loads(r.read())
    assert health["ok"] and health["stt"] and not health["tts"]


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
    assert rec.description == "faster-whisper small/cpu"
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
    monkeypatch.setattr(server.gpu, "load_profile", lambda **kw: server.gpu.describe({"profile": "cuda", "gpu": "RTX 3060"}))
    monkeypatch.setattr(sys, "argv", ["server.py", "--download-only"])
    server.main()
    assert calls == [("stt", "large-v3-turbo"), ("tts", server.XTTS_MODEL)]


# ---------------------------------------------------------------- GPU profiles


import gpu  # noqa: E402

RTX = {"name": "NVIDIA GeForce RTX 3060", "vendor": "nvidia"}
RX6700 = {"name": "AMD Radeon RX 6700 XT", "vendor": "amd"}
RYZEN_IGPU = {"name": "AMD Radeon(TM) Graphics", "vendor": "amd"}
RX580 = {"name": "Radeon RX 580 Series", "vendor": "amd"}
ARC = {"name": "Intel(R) Arc(TM) A750 Graphics", "vendor": "intel"}


@pytest.mark.parametrize(
    "adapters, opencl, profile, gfx",
    [
        ([RTX], [], "cuda", None),
        ([RYZEN_IGPU, RTX], ["gfx1036"], "cuda", None),  # laptop: NVIDIA wins over the Radeon iGPU
        ([RYZEN_IGPU, RX6700], ["gfx1036", "gfx1031"], "rocm", "gfx1031"),
        ([{"name": "AMD Radeon RX 7900 XTX", "vendor": "amd"}], [], "rocm", "gfx1100"),
        ([RYZEN_IGPU], ["gfx1036"], "rocm", "gfx1036"),
        ([RX580], ["gfx803"], "vulkan", "gfx803"),  # Polaris: no ROCm PyTorch
        ([ARC], [], "vulkan", None),
        ([], [], "cpu", None),
    ],
)
def test_profile_choice(adapters, opencl, profile, gfx):
    info = gpu.choose(adapters, opencl)
    assert (info["profile"], info["gfx"]) == (profile, gfx)
    assert info["stt"] == ("faster-whisper" if profile == "cuda" else "whispercpp")
    assert info["tts_device"] == ("gpu" if profile in ("cuda", "rocm") else "cpu")


def test_profile_overrides():
    assert gpu.choose([RX6700], [], forced_profile="cpu")["profile"] == "cpu"
    assert gpu.choose([RX580], [], forced_gfx="gfx1030")["profile"] == "rocm"


def test_gfx_from_product_names():
    names = {
        "AMD Radeon RX 9070 XT": "gfx1201",
        "AMD Radeon RX 7600": "gfx1102",
        "AMD Radeon RX 7700S": "gfx1102",
        "AMD Radeon RX 7800 XT": "gfx1101",
        "AMD Radeon RX 6800M": "gfx1031",
        "AMD Radeon RX 6800 XT": "gfx1030",
        "AMD Radeon RX 6600": "gfx1032",
        "AMD Radeon RX 5700 XT": "gfx1010",
        "AMD Radeon 780M Graphics": "gfx1103",
        "AMD Radeon 8060S Graphics": "gfx1151",
        "AMD Radeon(TM) Graphics": None,
    }
    for name, gfx in names.items():
        assert gpu.gfx_from_name(name) == gfx, name


def test_wmi_adapters_are_parsed():
    data = [
        {"Name": "AMD Radeon RX 6700 XT", "PNPDeviceID": "PCI\\VEN_1002&DEV_73DF&SUBSYS_1"},
        {"Name": "Microsoft Basic Display Adapter", "PNPDeviceID": "ROOT\\BasicDisplay"},
        {"Name": "Parsec Virtual Display Adapter", "PNPDeviceID": "ROOT\\DISPLAY\\0000"},
    ]
    assert gpu.parse_adapters(data) == [{"name": "AMD Radeon RX 6700 XT", "vendor": "amd"}]
    single = {"Name": "NVIDIA GeForce RTX 3060", "PNPDeviceID": "PCI\\VEN_10DE&DEV_2504"}
    assert gpu.parse_adapters(single) == [{"name": "NVIDIA GeForce RTX 3060", "vendor": "nvidia"}]


def test_vulkan_device_prefers_discrete():
    assert gpu.pick_vulkan_device([(0, "AMD Radeon(TM) Graphics", 1), (1, "AMD Radeon RX 6700 XT", 2)]) == 1
    assert gpu.pick_vulkan_device([(0, "llvmpipe", 4), (1, "Intel UHD", 1)]) == 1
    assert gpu.pick_vulkan_device([(0, "llvmpipe", 4)]) is None
    assert isinstance(gpu.vulkan_devices(), list)  # no loader or no GPU: empty, never an exception


def test_saved_profile(tmp_path):
    assert gpu.load_profile(tmp_path / "missing.json")["profile"] == "cuda"
    # the server detects the card when the installer did not save a profile
    detected = gpu.load_profile(tmp_path / "missing.json", fallback=lambda: gpu.choose([RX6700]))
    assert (detected["profile"], detected["gfx"]) == ("rocm", "gfx1031")
    path = tmp_path / "gpu-profile.json"
    path.write_text(json.dumps({"profile": "rocm", "gpu": "AMD Radeon RX 6700 XT", "gfx": "gfx1031"}), encoding="utf-8")
    saved = gpu.load_profile(path)
    assert (saved["profile"], saved["stt"], saved["tts_device"]) == ("rocm", "whispercpp", "gpu")


def test_engine_follows_profile(monkeypatch, tmp_path):
    import types

    args = types.SimpleNamespace(stt_engine="auto", whisper_model="auto")
    exe = tmp_path / "whisper-server"
    monkeypatch.setattr(server, "whispercpp_exe", lambda: exe)
    cpu = gpu.describe({"profile": "cpu"})
    rocm = gpu.describe({"profile": "rocm"})
    # no whisper.cpp build next to the server: faster-whisper instead
    assert server.resolve_engine(args, rocm) == ("faster-whisper", "large-v3-turbo")
    exe.write_bytes(b"")
    assert server.resolve_engine(args, rocm) == ("whispercpp", "large-v3-turbo")
    assert server.resolve_engine(args, cpu) == ("whispercpp", "small")
    assert server.resolve_engine(args, gpu.describe({"profile": "cuda"})) == ("faster-whisper", "large-v3-turbo")


# ---------------------------------------------------------------- whisper.cpp

FAKE_WHISPER_SERVER = r"""#!/usr/bin/env python3
# stands in for whisper-server: records how it was started, answers /inference
import json, os, sys
from http.server import BaseHTTPRequestHandler, HTTPServer

args = sys.argv[1:]
log = os.environ["FAKE_LOG"]
with open(log, "a", encoding="utf-8") as f:
    f.write(json.dumps({"args": args, "vk": os.environ.get("GGML_VK_VISIBLE_DEVICES")}) + "\n")
if "-ng" not in args and os.environ.get("FAKE_GPU_CRASH"):
    sys.exit(3)  # like a broken Vulkan driver
port = int(args[args.index("--port") + 1])

class H(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b"<html></html>")
    def do_POST(self):
        length = int(self.headers["Content-Length"])
        body = self.rfile.read(length)
        ok = self.path == "/inference" and b'name="file"; filename="audio.wav"' in body and b"RIFF" in body
        ok = ok and 'name="prompt"'.encode() in body and "Джарвис".encode() in body
        text = "Джарвис, открой  телеграм " if ok else "BAD REQUEST"
        data = json.dumps({"text": text}).encode()
        self.send_response(200); self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
    def log_message(self, *a): pass

HTTPServer(("127.0.0.1", port), H).serve_forever()
"""


@pytest.fixture
def fake_whisper(tmp_path, monkeypatch):
    import os
    import stat
    import sys

    exe = tmp_path / "whisper-server"
    exe.write_text(FAKE_WHISPER_SERVER.replace("/usr/bin/env python3", sys.executable), encoding="utf-8")
    exe.chmod(exe.stat().st_mode | stat.S_IEXEC)
    model = tmp_path / "ggml-test.bin"
    model.write_bytes(b"model")
    log = tmp_path / "calls.jsonl"
    monkeypatch.setenv("FAKE_LOG", str(log))
    if os.name == "nt":
        pytest.skip("the fake server is a script with a shebang")

    def calls():
        return [json.loads(line) for line in log.read_text(encoding="utf-8").splitlines()]

    return exe, model, calls


def test_whispercpp_uses_the_discrete_gpu(fake_whisper, monkeypatch):
    exe, model, calls = fake_whisper
    monkeypatch.setattr(server.gpu, "vulkan_devices", lambda: [(0, "AMD Radeon(TM) Graphics", 1), (1, "AMD Radeon RX 6700 XT", 2)])
    rec = server.WhisperCppRecognizer(str(model), exe=exe, ready_timeout=20)
    try:
        assert rec.transcribe_wav(make_wav([0] * 16000)) == "Джарвис, открой телеграм"
        assert rec.description.endswith("/vulkan")
        assert "RX 6700 XT" in rec.device
        started = calls()
        assert len(started) == 1 and started[0]["vk"] == "1"
        assert "-ng" not in started[0]["args"] and started[0]["args"][started[0]["args"].index("-l") + 1] == "ru"
    finally:
        rec.stop()


def test_whispercpp_falls_back_to_cpu(fake_whisper, monkeypatch):
    exe, model, calls = fake_whisper
    monkeypatch.setenv("FAKE_GPU_CRASH", "1")
    monkeypatch.setattr(server.gpu, "vulkan_devices", lambda: [(0, "AMD Radeon RX 580", 2)])
    rec = server.WhisperCppRecognizer(str(model), exe=exe, ready_timeout=20)
    try:
        assert rec.description.endswith("/cpu")
        assert [("-ng" in c["args"]) for c in calls()] == [False, True]
        assert rec.transcribe_wav(make_wav([0] * 1600)) == "Джарвис, открой телеграм"
    finally:
        rec.stop()


def test_whispercpp_restarts_after_the_process_dies(fake_whisper, monkeypatch):
    exe, model, calls = fake_whisper
    monkeypatch.setattr(server.gpu, "vulkan_devices", lambda: [])
    rec = server.WhisperCppRecognizer(str(model), use_gpu=False, exe=exe, ready_timeout=20)
    try:
        rec.stop()  # e.g. killed by an update
        assert rec.transcribe_wav(make_wav([0] * 1600)) == "Джарвис, открой телеграм"
        assert len(calls()) == 2
    finally:
        rec.stop()


def test_download_file_resumes(tmp_path):
    from http.server import BaseHTTPRequestHandler

    payload = bytes(range(256)) * 100

    class H(BaseHTTPRequestHandler):
        def do_GET(self):
            start = int(self.headers.get("Range", "bytes=0-")[6:].rstrip("-") or 0)
            self.send_response(206 if start else 200)
            self.send_header("Content-Length", str(len(payload) - start))
            self.end_headers()
            self.wfile.write(payload[start:])

        def log_message(self, *a):
            pass

    srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    try:
        dest = tmp_path / "m.bin"
        (tmp_path / "m.bin.part").write_bytes(payload[:1000])  # an interrupted download
        server.download_file(f"http://127.0.0.1:{srv.server_address[1]}/m.bin", dest, len(payload))
        assert dest.read_bytes() == payload
        assert not (tmp_path / "m.bin.part").exists()
    finally:
        srv.shutdown()


def test_reference_wav_is_read_without_torchaudio(tmp_path):
    import numpy as np

    path = tmp_path / "ref.wav"
    path.write_bytes(make_wav([1000, -1000] * 4800, rate=48000, channels=2))  # like the Jarvis samples
    samples, rate = server.read_wav(path)
    assert rate == 48000 and len(samples) == 4800
    assert np.allclose(samples, 0.0)  # left and right cancel out when mixed to mono
    assert len(server.resample(np.ones(4800, dtype=np.float32), 48000, 22050)) == 2205


def test_tts_import_does_not_require_torchcodec(monkeypatch):
    import sys
    import types

    import_utils = types.SimpleNamespace(is_torchcodec_available=lambda: False)
    utils = types.SimpleNamespace(import_utils=import_utils)
    monkeypatch.setitem(sys.modules, "transformers", types.SimpleNamespace(utils=utils))
    monkeypatch.setitem(sys.modules, "transformers.utils", utils)
    monkeypatch.setitem(sys.modules, "transformers.utils.import_utils", import_utils)
    server.allow_tts_without_torchcodec()
    assert import_utils.is_torchcodec_available() is True
