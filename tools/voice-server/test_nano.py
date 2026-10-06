import io
import json
import sys
from types import SimpleNamespace
import wave

import numpy as np
import pytest

import nano
import server


def test_download_repairs_partial_and_corrupt_files(tmp_path, monkeypatch):
    payload = b"model"
    spec = {"name": "weights.data", "size": len(payload), "sha256": nano.hashlib.sha256(payload).hexdigest()}
    monkeypatch.setattr(nano, "MODELS", [{"repo": "owner/model", "revision": "pinned", "directory": "tts", "files": [spec]}])
    calls = []

    def download(repo, name, **kwargs):
        calls.append((repo, name, kwargs))
        path = tmp_path / "tts" / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(payload)

    monkeypatch.setitem(sys.modules, "huggingface_hub", SimpleNamespace(hf_hub_download=download))
    nano.download_models(tmp_path)
    nano.download_models(tmp_path)
    assert len(calls) == 1 and calls[0][2]["revision"] == "pinned"
    (tmp_path / "tts" / spec["name"]).write_bytes(b"wrong")
    nano.download_models(tmp_path)
    assert len(calls) == 2
    monkeypatch.setitem(sys.modules, "huggingface_hub", SimpleNamespace(hf_hub_download=lambda *a, **k: None))
    (tmp_path / "tts" / spec["name"]).write_bytes(b"x")
    with pytest.raises(RuntimeError, match="incomplete or corrupt"):
        nano.download_models(tmp_path)


class Tokenizer:
    def encode(self, text, out_type=int):
        return list(range(len(text)))


@pytest.mark.parametrize("text", ["Не сохранять файл. Открой папку.", "а" * 200, "Диск C:\\Games\\Steam и 12345."])
def test_chunking_keeps_all_words_and_negations(text):
    chunks = nano.split_text(text, Tokenizer(), budget=15)
    assert "".join(chunks).replace(" ", "") == text.replace(" ", "")
    assert all(len(chunk) <= 15 for chunk in chunks)


def fake_voice(monkeypatch, limit=False, fail=False):
    voice = nano.NanoVoice.__new__(nano.NanoVoice)
    voice.rate, voice.prompt, voice.tokenizer = 48000, [[1] * 16], Tokenizer()
    voice.lock = nano.threading.Lock()
    resets = []

    def generate(rows, on_frame):
        on_frame([], 0, [1] * 16)
        if fail:
            raise RuntimeError("inference error")
        return [0] * (nano.MAX_FRAMES if limit else 1)

    voice.runtime = SimpleNamespace(
        build_voice_clone_request_rows=lambda prompt, text: {"text": text, "prompt": prompt},
        generate_audio_frames=generate,
        codec_streaming_session=SimpleNamespace(
            reset=lambda: resets.append(True),
            run_frames=lambda rows: (np.ones((1, 2, 4800), dtype=np.float32) * 0.1, 4800),
        ),
    )
    monkeypatch.setattr(nano, "resample_audio", lambda wav, rate, target: wav[::2])
    return voice, resets


def test_codec_writes_complete_wav_and_resets_between_requests(monkeypatch):
    voice, resets = fake_voice(monkeypatch)
    for _ in range(2):
        with wave.open(io.BytesIO(voice.synthesize("Готов."))) as wav:
            assert (wav.getframerate(), wav.getnchannels(), wav.getsampwidth(), wav.getnframes()) == (24000, 1, 2, 2400)
    assert len(resets) == 4


@pytest.mark.parametrize("limit,fail,match", [(True, False, "audio limit"), (False, True, "inference error")])
def test_failed_generation_is_not_reported_as_complete(monkeypatch, limit, fail, match):
    voice, resets = fake_voice(monkeypatch, limit, fail)
    with pytest.raises(RuntimeError, match=match):
        voice.synthesize("Не сохранять.")
    assert len(resets) == 2


@pytest.mark.parametrize("profile", ["cpu", "vulkan", "rocm", "cuda"])
def test_nano_loads_on_every_hardware_profile(profile):
    sentinel = object()
    assert server.load_voice({"profile": profile}, factory=lambda: sentinel, engine="nano") is sentinel


def test_default_engine_and_explicit_f5_are_preserved(tmp_path, monkeypatch):
    monkeypatch.setattr(server, "HERE", tmp_path)
    assert server.default_tts_engine() == "nano"
    (tmp_path / "tts-engine.txt").write_text("f5\n")
    assert server.default_tts_engine() == "f5"
    (tmp_path / "tts-engine.txt").write_text("unknown")
    with pytest.raises(ValueError, match="Invalid"):
        server.default_tts_engine()


def test_download_only_fetches_nano_on_cpu_and_propagates_failure(monkeypatch):
    calls = []
    monkeypatch.setattr(nano, "download_models", lambda: calls.append("nano"))
    args = SimpleNamespace(no_stt=True, no_tts=False, tts_engine="nano")
    server.download(args, {"profile": "cpu"})
    assert calls == ["nano"]
    monkeypatch.setattr(nano, "download_models", lambda: (_ for _ in ()).throw(RuntimeError("offline")))
    with pytest.raises(SystemExit) as error:
        server.download(args, {"profile": "cpu"})
    assert error.value.code == 1
