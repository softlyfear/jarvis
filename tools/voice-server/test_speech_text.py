from speech_text import for_speech
import server


def test_application_names_and_acronyms_are_prepared_for_russian_voice():
    assert for_speech("PowerShell, Discord и Steam. Google Gemini Flash Lite API.") == (
        "Пауэршелл, Дискорд и Стим. Гугл Джемини Флэш Лайт эй пи ай."
    )
    assert for_speech("В cmd: uv init. F5-TTS, HTTP, RTX.") == (
        "В си эм ди: ю ви инит. эф пять-ти ти эс, эйч ти ти пи, ар ти экс."
    )
    assert for_speech("Steamship, discordant, abc123, Открыл.") == "Steamship, discordant, abc123, Открыл."


def test_names_are_normalized_before_stress_and_even_without_ruaccent():
    voice = object.__new__(server.F5Voice)
    voice.accent = None
    assert voice._stressed("PowerShell открыт.") == "Пауэршелл открыт."

    class Accent:
        def process_all(self, text):
            assert "PowerShell" not in text
            return text + "+"

    voice.accent = Accent()
    assert voice._stressed("PowerShell открыт.") == "Пауэршелл открыт.+"

    class BrokenAccent:
        def process_all(self, text):
            raise RuntimeError("failed")

    voice.accent = BrokenAccent()
    assert voice._stressed("Discord") == "Дискорд"


def test_names_from_log_are_spoken_without_losing_file_extensions():
    assert for_speech("Сэр, файл resume.pdf открыт. Канал MoscowPython, BETA.") == (
        "Сэр, файл резюме точка пи ди эф открыт. Канал Москоу Пайтон, бета."
    )
    assert for_speech("отчёт.PDF, notes.txt, приложение.exe") == (
        "отчёт точка пи ди эф, notes точка тэ икс тэ, приложение точка экзэ"
    )
    assert for_speech("pdfium, text.docxml, 0.2.88") == "pdfium, text.docxml, 0.2.88"


def test_f5_preserves_batch_edges_and_prepares_text_before_inference(monkeypatch):
    import sys
    import types
    import numpy as np

    def infer(audio, ref_text, text, model, vocoder, **options):
        assert (audio, ref_text, text) == ("ref.wav", "Образец.", "резюме точка пи ди эф")
        # The upstream F5 stitcher concatenates instead of overlapping with this value.
        assert options["cross_fade_duration"] == 0.0
        return np.array([0.1, 0.2, 0.3], dtype=np.float32), server.TTS_SAMPLE_RATE, None

    monkeypatch.setitem(sys.modules, "f5_tts.infer.utils_infer", types.SimpleNamespace(infer_process=infer))
    voice = object.__new__(server.F5Voice)
    voice.dev, voice.model, voice.vocoder, voice.accent = "cuda", object(), object(), None
    output = voice._infer("resume.pdf", ("ref.wav", "Образец."))
    np.testing.assert_array_equal(output, np.array([0.1, 0.2, 0.3], dtype=np.float32))
