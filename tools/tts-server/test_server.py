"""Tests for the parts of server.py that do not need the model (run: python -m pytest)."""

import io
import wave

import server


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
