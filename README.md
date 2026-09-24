# Джарвис — голосовой помощник для Windows 11

![We are NOT limited by the technology of our time!](poster.jpg)

[![build](https://github.com/softlyfear/jarvis/actions/workflows/windows.yml/badge.svg)](https://github.com/softlyfear/jarvis/actions/workflows/windows.yml)

Говорите «Джарвис», после отклика — команду: «открой телеграм», «запусти доту», «громкость пятьдесят», «удали вчерашний скриншот». Отвечает записанным голосом Джарвиса из русского дубляжа, а на свободные вопросы — клоном того же голоса.

Форк [Priler/jarvis](https://github.com/Priler/jarvis): ядро на Rust (Tauri) от Abraham Tugalov, доработанное для повседневного использования.

**Скачать:** [Releases → latest](https://github.com/softlyfear/jarvis/releases/tag/latest) · **Установка и настройка:** [docs/INSTALL-RU.md](docs/INSTALL-RU.md)

## Возможности

| | Без интернета | Чем сделано |
|---|---|---|
| Открыть и закрыть любую программу | ✔ | меню «Пуск», ярлыки, псевдонимы из настроек; русские названия сопоставляются с английскими |
| Запустить игру по названию или прозвищу | ✔ | библиотеки Steam находятся автоматически, `steam://rungameid` |
| Громкость, пауза, треки | ✔ | медиаклавиши Windows |
| Папки, поиск и удаление файлов | ✔ | только в разрешённых папках и только в Корзину |
| Скриншот, свернуть окна, блокировка, сон, выключение | ✔ | опасное переспрашивает «да или нет» |
| Разговор, сложные просьбы, цепочки действий | — | Gemini / OpenRouter / Groq с ротацией ключей или локальная Ollama; нейросеть сама вызывает действия ПК |
| Точное распознавание речи | ✔ | Whisper `large-v3-turbo` на видеокарте; без неё — Vosk |
| Ответы голосом Джарвиса | ✔ | XTTS-v2 на видеокарте; без неё — голос Windows |

## Как устроено

```
микрофон ──► Vosk: слово «Джарвис» и конец фразы
                 │
                 ▼
           Whisper (голосовой сервер, GPU) ──нет сервера──► текст Vosk
                 │
                 ▼
     команда из resources/commands ── найдена ──► нативное действие ──► фраза Джарвиса
                 │                                   (не нашла объект)
                 └── не найдена ──────────────┐            │
                                              ▼            ▼
                                  нейросеть (tools = те же действия)
                                              │
                                              ▼
                               ответ ──► клон голоса / голос Windows
```

- **Команды** — паки `resources/commands/*/command.toml`. Фразы обучают классификатор намерений, `type = "action"` вызывает нативное действие из `crates/jarvis-core/src/actions/`.
- **Нейросеть** (`llm.rs`) — любой OpenAI-совместимый API. Провайдеры перебираются по порядку, ключи — по кругу; ключ, упёршийся в лимит, временно пропускается.
- **Голосовой сервер** (`tools/voice-server`) — Python: faster-whisper + coqui-tts. Джарвис запускает его в фоне сам и работает без него, если сервер не установлен.
- **Настройки пользователя** — `%APPDATA%\com.priler.jarvis\assistant.toml`: ключи, псевдонимы программ, игр и папок, разрешённые папки, голос. Шаблон с комментариями — [`assistant.default.toml`](crates/jarvis-core/assets/assistant.default.toml).

## Требования

| | Минимум | Для Whisper и клона голоса |
|---|---|---|
| ОС | Windows 10/11 x64 | — |
| Видеокарта | не нужна | NVIDIA, от 6 ГБ видеопамяти (Whisper ~1,5 ГБ + голос 2–4 ГБ, оценка) |
| Место | ~400 МБ | ещё ~7 ГБ: Python-окружение и модели |
| Интернет | не нужен | только для нейросети и первой загрузки моделей |

## Сборка из исходников

Проверенный путь — GitHub Actions ([`.github/workflows/windows.yml`](.github/workflows/windows.yml)), шаг `Package` собирает архив. Те же шаги локально на Windows:

```powershell
# Rust (MSVC), Node.js 22
cd frontend; npm ci; npx vite build; cd ..
cargo build --release -p jarvis-app     # голосовой цикл и трей
cargo build --release -p jarvis-gui     # окно настроек (отдельной командой, см. ниже)
```

Затем разложите рядом с exe ресурсы и DLL, как в шаге `Package` workflow.

Пакеты собираются **раздельно**: при общей сборке cargo объединяет features `jarvis-core`, и окно настроек начинает требовать библиотеку Vosk.

Тесты работают и на Linux:

```bash
cargo test -p jarvis-core --no-default-features --features reqwest --lib -- actions assistant_config llm tts whisper voice_server
cd tools/voice-server && python -m pytest -q      # нужны только numpy и pytest
```

## Структура

| Путь | Что |
|---|---|
| `crates/jarvis-app` | главный цикл: слово активации → распознавание → команда; трей |
| `crates/jarvis-core` | ядро: STT, команды, действия, нейросеть, голос, настройки |
| `crates/jarvis-gui`, `frontend` | окно настроек (Tauri 2 + Svelte) |
| `resources/commands` | голосовые команды |
| `resources/sound/voices` | записанные фразы Джарвиса |
| `resources/vosk` | модели Vosk |
| `tools/voice-server` | Whisper + клон голоса |
| `docs/INSTALL-RU.md` | инструкция для пользователя |

## Отличия от upstream

- Паки команд переписаны с AutoHotkey и старого YAML на нативные действия (в upstream большая часть паков не загружалась). Исправлен пак погоды.
- Добавлены: нейросетевой фолбэк с инструментами, голосовое подтверждение опасных действий, Whisper, озвучка произвольного текста, автозапуск голосового сервера, сборка и выпуск через GitHub Actions.

## Лицензия и авторы

Исходный проект — © Abraham Tugalov ([Priler](https://github.com/Priler)), [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/): только некоммерческое использование, с указанием автора и под той же лицензией. Доработки форка распространяются на тех же условиях. Подробности — в [LICENSE.txt](LICENSE.txt).

Используемые модели: [Vosk](https://alphacephei.com/vosk/), [Whisper](https://github.com/openai/whisper) через [faster-whisper](https://github.com/SYSTRAN/faster-whisper), [XTTS-v2](https://huggingface.co/coqui/XTTS-v2) (Coqui Public Model License, некоммерческая).
