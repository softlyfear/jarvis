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
