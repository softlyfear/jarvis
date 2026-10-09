# jarvis — правила работы

Форк [Priler/jarvis](https://github.com/Priler/jarvis): офлайн-голосовой ассистент для Windows 11 на Rust. Для любого пользователя Windows: голосом открывать и закрывать программы, запускать игры, управлять звуком и файлами, разговаривать. Голос ответа — записанные фразы Джарвиса из дубляжа; произвольный текст озвучивается через TTS.

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
1. Vosk ловит только слово активации (грамматика `get_wake_grammar`). Пока голосовой сервер отвечает, фразу режет по паузам `endpoint.rs` (порог шума подстраивается под микрофон, пауза 0,6 с, максимум 15 с), а распознаёт сервер (`whisper::ServerListener`, `stt::recognize_with_server`); распознаватель речи Vosk не работает. Фраза, закончившаяся вместе со словом активации (не раньше `PREFED_MAX_AGE_SAMPLES`, 1 с), — команда; в ней слова до имени отбрасываются (`text::after_address`, имя узнаётся нечётко: «Дарвис», «Чарли», «дар из»). Сервер не ответил — эту фразу распознаёт Vosk по её звуку (`vosk::recognize_audio`), и следующие `retry_after_secs` Vosk слушает сам. Затем `intent-classifier` или нечёткое сравнение выбирает команду из `resources/commands/*/command.toml`.
2. Команды `type = "action"` вызывают нативные действия из `jarvis-core/src/actions/` (программы, игры Steam, громкость, медиа, папки, файлы, питание, поиск; `input.rs` — окна, сочетания клавиш из `NAMED_HOTKEYS`, печать текста; клавиши и текст не уходят в окно самого Джарвиса — `key_target` берёт следующее окно, `focus_app` выводит программу вперёд). Тест паков запрещает одну фразу в двух командах.
3. Если команда не найдена или действие не нашло объект (`ActionError::NotFound`), фраза уходит агенту (`agent/mod.rs`): по умолчанию `[agent] backend = "direct"` — LLM `jarvis-core/src/llm.rs`; необязательно `"openclaw"` — внешний агент OpenClaw (`agent/openclaw.rs`, MCP-сервер `jarvis-pc` в `agent/mcp.rs`, вопросы агента — `agent/bridge.rs`, установка — `agent/managed.rs`, `docs/OPENCLAW-RU.md`), при его недоступности — прямой путь. Оба ходят в действия через `agent::execute_action` (одна блокировка, отмена по «Джарвис»). Шлюзы (OpenAI-совместимые, одни и те же модели `KILO_PAID_MODELS`: Claude Haiku 5.5 — основная по выбору заказчика 08.10.2026 → Gemini 3.5 Flash-Lite → Gemini 3.5 Flash → DeepSeek V4 Flash по прежнему бенчмарку): **Polza AI** (`name = "polza"`, ключ `sk-…`, из России без VPN, рубли) и **Kilo** (`name = "kilo"`, ключ JWT `eyJ…`, один на аккаунт; из России только через VPN — без него 403 на всё). Шлюз ключа — `gateway_of_key`; выбранный в окне идёт первым, второй следом; 402 (нет денег) усыпляет ключ на час, два 403 подряд — провайдер отдыхает 2 минуты; последними без ключа — бесплатные модели (`KILO_FREE_MODELS`, `[llm] free_fallback`, добавляются в `assistant_config::parse` и в старые конфиги); `free_only` оставляет только их. Окно и установщик пишут ключи Polza и Kilo. Блоки `gemini` из конфигов старых версий `parse` игнорирует, окно при сохранении и `configure.ps1` вырезают их из файла вместе с ключами и комментарием. Провайдер, упавший по сети (таймаут), 2 минуты пропускается; модель вызывает те же действия как tools (`llm/tools.rs`) и знает активное окно. Разговор (`memory_minutes`) хранится в `llm-history.json` и переживает перезапуск; встроенные команды записываются туда же (`llm::remember_command`). Устаревшее правило обращения в `extra_prompt` вырезается (`without_address_rule`), английские рассуждения вместо ответа — ошибка модели, отвечает следующая.
4. Опасные действия (`Action::is_dangerous`: удаление, выключение, перезагрузка, сон, очистка корзины) сначала спрашивают «да/нет» голосом (`actions/confirm.rs`). Закрытие программы — как крестик одного окна (несколько окон — уточнение, процессы не убиваются); диалог «Сохранить?» читается через UI Automation (`actions/dialog.rs` + `dialog.ps1`) и ждёт голосового ответа «сохранить / не сохранять / отмена».
5. Ответ LLM озвучивается `tts.rs` (`backend = "http"` или `"none"`) на голосовом сервере. Единственный голос — **Jarvis New** (пак `jarvis-remaster`, `voices::VOICE_ID`), единственный синтезатор — **F5-TTS** (веса ESpeech RL-V2, 16 шагов, ударения RUAccent в `models/ruaccent`, сбой ударений не мешает речи): образец — клипы и точный текст из `[tts.ru]` в `voice.toml` (не больше 12 с вместе). Синтез только на CUDA/ROCm; на CPU его нет — остаются записанные отклики, ошибка видна в `/health` и окне. XTTS, SAPI и MOSS-TTS-Nano удалены (06.10.2026); модели берутся из кеша, сеть — только если файла нет (`hub_file`). Первое предложение ответа синтезируется заранее, пока LLM дописывает остальное (`agent/speech.rs`).
6. Голосовой сервер `tools/voice-server/` (Python: распознавание **GigaAM v3 CTC** int8 через `onnx-asr` на процессоре у всех пользователей — `GigaAMRecognizer`, файлы в `models/gigaam-v3`, только русский; синтез F5; `/stt`, `/tts`, `/health`) Джарвис запускает в фоне сам (`voice_server.rs`), если он установлен `setup.bat`. Бенчмарк 09.10.2026 (`~/Documents/jarvis-stt-bench/RESULTS.md`, Golos + FLEURS): на командах GigaAM v3 ошибается в 3,9% слов (0,14 с на фразу на CPU), Whisper large-v3-turbo — в 16,8%, Vosk small 0.22 — в 14,4%; на длинном чтении Whisper и GigaAM равны (5,6% и 6,7%). Whisper остаётся по `--stt-engine whisper|faster-whisper|whispercpp`. Зависимости: `requirements.txt` — распознавание (`onnx-asr[cpu,hub]`, faster-whisper), `requirements-tts.txt` и `f5-tts==1.1.22 --no-deps` — синтез; `torch` 2.8 (cu128 для Blackwell, cu126 для прежних NVIDIA). `torchaudio.load` подменён (`patch_torchaudio_load`), поэтому ROCm-сборки torch ≥ 2.9 работают без FFmpeg.
7. Видеокарта (`gpu.py`): профиль `cuda` | `rocm` | `vulkan` | `cpu` определяется при установке (WMI + OpenCL → gfx-таргет) и сохраняется в `gpu-profile.json`; без файла сервер определяет сам. Распознавание от профиля не зависит (GigaAM на CPU); профиль выбирает F5 и, если включён Whisper, его движок: NVIDIA — faster-whisper на CUDA, остальные — whisper.cpp (`whispercpp/whisper-server.exe`, собирается в CI с Vulkan, `GGML_NATIVE=OFF` + все CPU-варианты) и F5 на ROCm (индекс AMD `stable.repo.amd.com/rocm/whl-next`, `torch[device-gfxNNNN]`, Python 3.12); без ROCm/CUDA — только распознавание. `requirements.txt` держит `transformers<5` для RUAccent; `f5-tts` ставится `--no-deps` (его полный список тянет gradio, bitsandbytes, torchcodec).
8. Всё в папке программы: `install.ps1` ставит встраиваемый Python 3.12 в `tools/voice-server/python` (`._pth`: `import site` и `..`), pip только с `--no-cache-dir`, кеши библиотек сервер направляет в `models/cache`. Старый `.venv` удаляется при переустановке; `voice_server::python_path` ищет `python\python.exe`, затем `.venv`. `install.ps1 -RuntimeOnly` (тихое обновление) обновляет F5 и torch, сохраняя остальное. Деинсталлятор спрашивает, удалить ли `%APPDATA%`/`%LOCALAPPDATA%\com.priler.jarvis`. Каждый GPU-путь при ошибке откатывается на CPU; реальное железо AMD/Intel в CI не проверяется.

Пользовательские настройки (ключи LLM, TTS, псевдонимы программ, игр и папок, разрешённые папки) лежат в `assistant.toml` в каталоге конфигурации (`%APPDATA%\com.priler.jarvis\`). Шаблон — `crates/jarvis-core/assets/assistant.default.toml`, он создаётся при первом запуске.

## Сборка и проверки

Целевая платформа — **Windows x64**. И контейнер Claude Code on the web, и локальная машина разработчика (Ubuntu) — Linux, поэтому (`DOCS_RS=1` уже задан в `env` в `.claude/settings.json`; локально тулчейн ставится без sudo: rustup в `~/.cargo`, Node 22 и LSP в `~/.local`; `libasound2-dev`, `libssl-dev` (без него `openssl-sys` — build-зависимость `ort-sys` — валит `cargo check`), `pkg-config` и mingw — через `apt`. Если `libssl-dev` не установлен, его заголовки лежат в `~/.local/opt/openssl-dev` (`apt-get download` + `dpkg-deb -x`): `export OPENSSL_LIB_DIR=~/.local/opt/openssl-dev/usr/lib/x86_64-linux-gnu OPENSSL_INCLUDE_DIR=~/.local/opt/openssl-dev/usr/include CPATH=~/.local/opt/openssl-dev/usr/include/x86_64-linux-gnu`. Тесты `jarvis-gui` на Linux не собираются (нужен GTK): `cargo check --tests -p jarvis-gui --target x86_64-pc-windows-gnu`, запуск — в CI):

```
# проверка типов под Linux (ort-sys не может скачать бинарники через прокси — DOCS_RS=1 отключает линковку)
DOCS_RS=1 cargo check -p jarvis-core
# кросс-проверка под Windows (нужны target x86_64-pc-windows-gnu и mingw, ставит SessionStart-хук)
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
# unit-тесты новых модулей (без vosk/ort, на Linux)
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib
# расширенные: IPC и Lua (как в CI)
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest,lua,ipc --lib
# голосовой сервер: модели подменяются фейками, нужны только numpy и pytest (как в CI, Python 3.12)
cd tools/voice-server && uv run --no-project --python 3.12 --with pytest --with numpy --with soundfile python -m pytest -q
```

Настоящая сборка `.exe` и установщика `JarvisSetup.exe` (Inno Setup, `installer/jarvis.iss` + `installer/configure.ps1`) — GitHub Actions (`.github/workflows/windows.yml`, `windows-latest`), артефакт скачивается со страницы запуска. whisper.cpp собирается там же (Vulkan SDK, кеш `whispercpp-<ref>-vulkan-…`), смоук-тест гоняет `WhisperCppRecognizer` на тестовой модели на CPU. Установщик запускает `install.ps1 -Installer` без консоли через `ExecAndLogOutput` (страница с прогрессом, строки `==> шаг` и pip `--progress-bar raw`), вывод сохраняет в `voice-install.log`. Звук, микрофон и действия Windows проверяются только на реальном ПК: не выдавай их за проверенные.

## GUI

Главная страница окна — шар `frontend/src/components/elements/VoiceOrb.svelte` (canvas). Правки `frontend/` проверять скриншотами: скил `gui-check` (Playwright, мок Tauri и jarvis-app). Данные: `IpcEvent::AudioLevel` (уровень и 16 полос спектра из `visual.rs`, ~30/с) и `IpcEvent::Speaking`. Скрипты PowerShell проверяются парсером `pwsh` (`[System.Management.Automation.Language.Parser]::ParseFile`), UTF-8 с BOM и CRLF — иначе Windows PowerShell 5.1 портит кириллицу.

## Журналы и обновления

- `%APPDATA%\com.priler.jarvis\`: `log.txt` (jarvis-app, debug; в начале версия и сборка), `gui-log.txt` (окно: клики, маршруты, ошибки UI через `ui_log`), `voice-server.log`. Кнопка «Собрать логи» (`collect_logs`) пакует их в zip на рабочий стол, ключи маскируются (`mask_secrets`: строки `keys`, префиксы `AIza`, `AQ.`, `eyJ`, `sk-`). Разбор бага начинать с этих файлов.
- Версия: CI задаёт `JARVIS_VERSION=0.2.<run_number>` и `JARVIS_BUILD=<sha>` (зашиваются через `option_env!`), публикует `version.json` в релиз `latest`. Окно сравнивает версии (`check_update`) и запускает новый `JarvisSetup.exe /SILENT`; установщик в тихом режиме не трогает голосовой сервер и выбор голоса и сам перезапускает Джарвиса.
- Один `jarvis-app` на сессию: именованный мьютекс `Local\JarvisVoiceAssistantApp` (новая копия ждёт 8 с, пока старая выйдет); окно перезапускает и выключает его командами `restart_jarvis_app` / `stop_jarvis_app` (kill процесса), а не через IPC — на странице настроек IPC отключён. Голосовой сервер занимает порт эксклюзивно до загрузки моделей и сам выходит через 20 с после исчезновения `jarvis-app` (`--exit-with-app`).
- Обращение `[assistant] address` (сэр / мисс / любое слово): в системный промпт LLM и, если оно не «сэр» и `tts.backend = "http"`, в отклики — `phrases.rs` синтезирует их на голосовом сервере в `%APPDATA%\com.priler.jarvis\phrases\` (FNV-хеш голоса и текста) и `voices::play` берёт их вместо записанных. Установщик передаёт `configure.ps1 -Address sir|miss`; `configure.ps1` пишет ключ в блок Kilo (`eyJ…`) или Polza (остальные, блок встаёт первым), склеивая перенесённые строки, и отчёт без ключей в `configure.log`.
- Пока Джарвис говорит (`audio::hold_microphone`, по длительности звука), слушатель пропускает микрофон: иначе он распознаёт свои ответы как команды. Исключение — ответ клоном голоса: пока он звучит, `app::wait_for_wake_word` слушает только слово активации, и «Джарвис» обрывает речь (`audio::stop_speaking`, `[tts] barge_in`); ответ, где он сам называет себя, не прерывается.
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

- В репозитории параллельно работает GPT/Codex: его правила, планы и доказательства — в `.codex/` (`.codex/AGENTS.md` полезно прочитать для сверки архитектуры). **`.codex/` не изменять** — договорённость заказчика. Чужие незакоммиченные файлы (например, `Jarvis_OpenClaw_TZ.md`) не трогать и в свои коммиты не включать; `git add` — только своих путей.
- Работа идёт прямо в ветке по умолчанию `master` форка `softlyfear/jarvis`. Upstream `Priler/jarvis` не трогать (никаких PR туда без явной просьбы).
- **Самостоятельно:** закрыв задачу или вопрос, сам коммить и пушь после зелёных проверок, не спрашивая разрешения; после пуша проверить запуск CI (`gh run watch`). Решения по ходу (подход, имена, объём правки, что взять, а что нет) принимать самому и коротко объяснять в отчёте; спрашивать только то, что без заказчика не решить (деньги, ключи, удаление его данных, проверка на реальном ПК).
- Conventional commits: `feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`, `ci:`. Заголовок и одна-две строки тела.
- Автор и коммиттер — заказчик (`softlyfear`). Коммиты ИИ помечаются трейлером без почты: `Co-Authored-By: Claude Opus 5.5` (модель, реально делавшая работу). Почты ассистентов (`noreply@anthropic.com` и т. п.) в истории запрещены.
- Одна законченная задача — один коммит; незавершённое не коммитится.
