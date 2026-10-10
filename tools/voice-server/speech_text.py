"""Pronunciation for the Russian F5 voice; never used for STT or action arguments."""
import re

# Whole names only: avoid replacing "Steam" inside another word.
NAMES = {
    "powershell": "Пауэршелл", "windows": "Виндоус", "discord": "Дискорд",
    "steam": "Стим", "chrome": "Хром", "google": "Гугл", "gemini": "Джемини",
    "telegram": "Телеграм", "youtube": "Ютуб", "github": "Гитхаб",
    "python": "Пайтон", "whisper": "Виспер", "gigaam": "Гига эм",
    "openclaw": "Опен кло", "nvidia": "Энвидиа", "cuda": "Куда",
    "rocm": "Рок эм", "jarvis": "Джарвис", "flash": "Флэш", "lite": "Лайт",
    "api": "эй пи ай", "cmd": "си эм ди", "uv": "ю ви", "init": "инит",
    "cpu": "си пи ю", "gpu": "джи пи ю", "amd": "эй эм ди",
    "tts": "ти ти эс", "stt": "эс ти ти", "f5": "эф пять",
}
LETTERS = dict(zip("ABCDEFGHIJKLMNOPQRSTUVWXYZ", (
    "эй", "би", "си", "ди", "и", "эф", "джи", "эйч", "ай", "джей", "кей",
    "эл", "эм", "эн", "оу", "пи", "кью", "ар", "эс", "ти", "ю", "ви",
    "дабл ю", "экс", "уай", "зед",
)))


def for_speech(text):
    def replace(match):
        word = match.group()
        known = NAMES.get(word.lower())
        if known:
            return known
        if word.isalpha() and word.isupper() and 2 <= len(word) <= 6:
            return " ".join(LETTERS[c] for c in word)
        return word

    return re.sub(r"(?<![\w])[A-Za-z][A-Za-z0-9]*(?![\w])", replace, text)
