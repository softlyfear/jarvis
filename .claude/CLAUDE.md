# jarvis — правила работы

Форк [Priler/jarvis](https://github.com/Priler/jarvis): офлайн-голосовой ассистент для Windows 11 на Rust. Делается для одного пользователя (сестра заказчика): голосом открывать и закрывать программы, запускать игры, управлять звуком и файлами, разговаривать. Голос ответа — записанные фразы Джарвиса из дубляжа; произвольный текст озвучивается через TTS.

## Язык

- Общение с заказчиком, отчёты, README для пользователя, тексты в `assistant.toml` и всё, что произносится вслух, — **русский**.
- Комментарии и doc-комментарии в Rust — **английский** (как в исходном коде Priler).
- Дословно: идентификаторы, пути, команды, имена крейтов.

## Архитектура

| Что | Где |
|---|---|
| Главный цикл: wake word → STT → команда → действие | `crates/jarvis-app/src/app.rs` |
| Запуск, инициализация подсистем, трей | `crates/jarvis-app/src/main.rs`, `tray*.rs` |
| Ядро (всё остальное) | `crates/jarvis-core/src/` |
| GUI настроек (Tauri 2 + Svelte) | `crates/jarvis-gui`, `frontend/` |
| Паки команд (`command.toml` + Lua) | `resources/commands/*/` |
| Звуки голоса Джарвиса | `resources/sound/voices/<voice>/` (`voice.toml` + mp3/wav) |
| Модели Vosk | `resources/vosk/` |

**Гибрид команд** (добавлено в форке):
1. Vosk ловит слово активации и конец фразы; звук фразы перераспознаёт Whisper через локальный голосовой сервер (`whisper.rs`, `stt.rs`: буфер фразы, `stt::feed` для фонового прослушивания). Сервер недоступен — остаётся текст Vosk, Whisper пропускается на `retry_after_secs`. Затем `intent-classifier` или нечёткое сравнение выбирает команду из `resources/commands/*/command.toml`.
2. Команды `type = "action"` вызывают нативные действия из `jarvis-core/src/actions/` (программы, игры Steam, громкость, медиа, папки, файлы, питание, поиск; `input.rs` — окна, сочетания клавиш из `NAMED_HOTKEYS`, печать текста). Тест паков запрещает одну фразу в двух командах.
3. Если команда не найдена или действие не нашло объект (`ActionError::NotFound`), фраза уходит в LLM: `jarvis-core/src/llm.rs`. Провайдер — только шлюз **Kilo** (OpenAI-совместимый, без VPN): блок `name = "kilo"` с необязательным ключом (один на аккаунт, JWT `eyJ…`; `KILO_PAID_MODELS` по бенчмарку: Gemini 3.5 Flash-Lite → Gemini 3.5 Flash → DeepSeek V4 Flash) идёт первым, 402 (нет денег) усыпляет ключ на час; последними без ключа — бесплатные модели (`KILO_FREE_MODELS`, `[llm] free_fallback`, добавляются в `assistant_config::parse` и в старые конфиги); `free_only` оставляет только их. Окно и установщик пишут только ключ Kilo. Gemini убран полностью: блоки старых версий `parse` игнорирует, окно при сохранении и `configure.ps1` вырезают их из файла вместе с ключами и комментарием. Провайдер, упавший по сети (таймаут), 2 минуты пропускается; модель вызывает те же действия как tools (`llm/tools.rs`).
4. Опасные действия (`Action::is_dangerous`: удаление, выключение, перезагрузка, сон, очистка корзины) сначала спрашивают «да/нет» голосом (`actions/confirm.rs`).
5. Ответ LLM озвучивается `tts.rs`: SAPI Windows или клон голоса на том же сервере. `/tts` получает `voice` — id выбранного пака, сервер клонирует его образцы (WAV/MP3) с настройками модели XTTS и кеширует латенты по паку.
6. Голосовой сервер `tools/voice-server/` (Python: faster-whisper или whisper.cpp + coqui-tts, `/stt`, `/tts`, `/health`) Джарвис запускает в фоне сам (`voice_server.rs`), если он установлен `setup.bat`. Версии закреплены в `requirements.txt` и `install.ps1`: `torch` 2.8 для CUDA и CPU, `transformers<5` (5.x ломает coqui-tts). `torchaudio.load` не используется (`patch_xtts_audio_loader`), поэтому ROCm-сборки torch ≥ 2.9 работают без FFmpeg.
7. Видеокарта (`gpu.py`): профиль `cuda` | `rocm` | `vulkan` | `cpu` определяется при установке (WMI + OpenCL → gfx-таргет) и сохраняется в `gpu-profile.json`; без файла сервер определяет сам. NVIDIA — faster-whisper + XTTS на CUDA. Остальные — whisper.cpp (`whispercpp/whisper-server.exe`, собирается в CI с Vulkan, `GGML_NATIVE=OFF` + все CPU-варианты) и XTTS на ROCm (индекс AMD `stable.repo.amd.com/rocm/whl-next`, `torch[device-gfxNNNN]`, Python 3.12) или на CPU.
8. Всё в папке программы: `install.ps1` ставит встраиваемый Python 3.12 в `tools/voice-server/python` (`._pth`: `import site` и `..`), pip только с `--no-cache-dir`, кеши библиотек сервер направляет в `models/cache`. Старый `.venv` удаляется при переустановке; `voice_server::python_path` ищет `python\python.exe`, затем `.venv`. Деинсталлятор спрашивает, удалить ли `%APPDATA%`/`%LOCALAPPDATA%\com.priler.jarvis`. Каждый GPU-путь при ошибке откатывается на CPU; реальное железо AMD/Intel в CI не проверяется.

Пользовательские настройки (ключи LLM, TTS, псевдонимы программ, игр и папок, разрешённые папки) лежат в `assistant.toml` в каталоге конфигурации (`%APPDATA%\com.priler.jarvis\`). Шаблон — `crates/jarvis-core/assets/assistant.default.toml`, он создаётся при первом запуске.

## Сборка и проверки

Целевая платформа — **Windows x64**. И контейнер Claude Code on the web, и локальная машина разработчика (Ubuntu) — Linux, поэтому (`DOCS_RS=1` уже задан в `env` в `.claude/settings.json`; локально тулчейн ставится без sudo: rustup в `~/.cargo`, Node 22 и LSP в `~/.local`; `libasound2-dev`, `pkg-config` и mingw — через `apt`):

```
# проверка типов под Linux (ort-sys не может скачать бинарники через прокси — DOCS_RS=1 отключает линковку)
DOCS_RS=1 cargo check -p jarvis-core
# кросс-проверка под Windows (нужны target x86_64-pc-windows-gnu и mingw, ставит SessionStart-хук)
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
# unit-тесты новых модулей (без vosk/ort, на Linux)
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib
# голосовой сервер: модели подменяются фейками, нужны только numpy и pytest (как в CI, Python 3.12)
cd tools/voice-server && uv run --no-project --python 3.12 --with pytest --with numpy python -m pytest -q
```

Настоящая сборка `.exe` и установщика `JarvisSetup.exe` (Inno Setup, `installer/jarvis.iss` + `installer/configure.ps1`) — GitHub Actions (`.github/workflows/windows.yml`, `windows-latest`), артефакт скачивается со страницы запуска. whisper.cpp собирается там же (Vulkan SDK, кеш `whispercpp-<ref>-vulkan-…`), смоук-тест гоняет `WhisperCppRecognizer` на тестовой модели на CPU. Установщик запускает `install.ps1 -Installer` без консоли через `ExecAndLogOutput` (страница с прогрессом, строки `==> шаг` и pip `--progress-bar raw`), вывод сохраняет в `voice-install.log`. Звук, микрофон и действия Windows проверяются только на реальном ПК: не выдавай их за проверенные.

## GUI

Главная страница окна — шар `frontend/src/components/elements/VoiceOrb.svelte` (canvas). Данные: `IpcEvent::AudioLevel` (уровень и 16 полос спектра из `visual.rs`, ~30/с) и `IpcEvent::Speaking`. Скрипты PowerShell проверяются парсером `pwsh` (`[System.Management.Automation.Language.Parser]::ParseFile`), UTF-8 с BOM и CRLF — иначе Windows PowerShell 5.1 портит кириллицу.

## Журналы и обновления

- `%APPDATA%\com.priler.jarvis\`: `log.txt` (jarvis-app, debug; в начале версия и сборка), `gui-log.txt` (окно: клики, маршруты, ошибки UI через `ui_log`), `voice-server.log`. Кнопка «Собрать логи» (`collect_logs`) пакует их в zip на рабочий стол, ключи маскируются (`mask_secrets`: строки `keys`, префиксы `AIza`, `AQ.`, `eyJ`). Разбор бага начинать с этих файлов.
- Версия: CI задаёт `JARVIS_VERSION=0.2.<run_number>` и `JARVIS_BUILD=<sha>` (зашиваются через `option_env!`), публикует `version.json` в релиз `latest`. Окно сравнивает версии (`check_update`) и запускает новый `JarvisSetup.exe /SILENT`; установщик в тихом режиме не трогает голосовой сервер и выбор голоса и сам перезапускает Джарвиса.
- Один `jarvis-app` на сессию: именованный мьютекс `Local\JarvisVoiceAssistantApp` (новая копия ждёт 8 с, пока старая выйдет); окно перезапускает и выключает его командами `restart_jarvis_app` / `stop_jarvis_app` (kill процесса), а не через IPC — на странице настроек IPC отключён. Голосовой сервер занимает порт эксклюзивно до загрузки моделей и сам выходит через 20 с после исчезновения `jarvis-app` (`--exit-with-app`).
- Обращение `[assistant] address` (сэр / мисс / любое слово): в системный промпт LLM и, если оно не «сэр» и `tts.backend = "http"`, в отклики — `phrases.rs` синтезирует их на голосовом сервере в `%APPDATA%\com.priler.jarvis\phrases\` (FNV-хеш голоса и текста) и `voices::play` берёт их вместо записанных. Установщик передаёт `configure.ps1 -Address sir|miss`; `configure.ps1` пишет ключ Kilo (склеивая перенесённые строки) и отчёт без ключей в `configure.log`.
- Пока Джарвис говорит (`audio::hold_microphone`, по длительности звука; SAPI — на время вызова), слушатель пропускает микрофон: иначе он распознаёт свои ответы как команды.
- Библиотеки с путём по умолчанию из `OUT_DIR` (как `pv_recorder`) на ПК пользователя не работают: путь к DLL задаётся явно относительно `APP_DIR`.

## Правила кода

- **Минимальный diff к upstream.** Код Priler не отформатирован `rustfmt`: **не запускать `cargo fmt`** по всему проекту. Новые файлы оформлять аккуратно, чужие — не переформатировать.
- Новая логика — в новых модулях (`actions`, `llm`, `tts`, `assistant_config`); правки в upstream-файлах — точечные.
- Всё, что зависит от Windows API, — за `#[cfg(windows)]` с fallback, чтобы крейт собирался и тестировался на Linux.
- Пользовательский ввод (распознанная речь, аргументы от LLM) не подставляется в текст shell или PowerShell-скрипта: только через аргументы процесса или переменные окружения (`platform::powershell(script, env)`).
- Файлы — только внутри `[safety].allowed_dirs`, удаление — только в Корзину. Системные процессы закрывать нельзя (`PROTECTED_PROCESSES`).
- Секреты (API-ключи) не логировать: в логах только имя провайдера и номер ключа.
- Каждое новое поведение покрывается unit-тестом, который работает на Linux (моки HTTP — `std::net::TcpListener`, см. тесты `llm.rs`).
- Проверить поведение библиотеки (mlua, kira, reqwest, tauri, vosk) — через MCP `context7`, версию брать из `Cargo.lock`.

## Лицензия

Upstream распространяется под **CC BY-NC-SA 4.0** (`LICENSE.txt`): некоммерческое использование, указание автора (Abraham Tugalov / Priler), та же лицензия для производных. Коммерческие сценарии не предлагать.

## Git

- Работа идёт прямо в ветке по умолчанию `master` форка `softlyfear/jarvis`, пуш без отдельного запроса после зелёных проверок. Upstream `Priler/jarvis` не трогать (никаких PR туда без явной просьбы).
- Conventional commits: `feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`, `ci:`. Заголовок и одна-две строки тела.
- Автор и коммиттер — заказчик (`softlyfear`). Коммиты ИИ помечаются трейлером без почты: `Co-Authored-By: Claude Opus 5.5` (модель, реально делавшая работу). Почты ассистентов (`noreply@anthropic.com` и т. п.) в истории запрещены.
- Одна законченная задача — один коммит; незавершённое не коммитится.
