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
2. Команды `type = "action"` вызывают нативные действия из `jarvis-core/src/actions/` (программы, игры Steam, громкость, медиа, папки, файлы, питание, поиск).
3. Если команда не найдена или действие не нашло объект (`ActionError::NotFound`), фраза уходит в LLM: `jarvis-core/src/llm.rs`. Провайдер по умолчанию — только Gemini; `models = ["auto"]` (`llm/models.rs`) берёт список моделей у API и ранжирует их. Ключи чередуются; модель вызывает те же действия как tools (`llm/tools.rs`).
4. Опасные действия (`Action::is_dangerous`: удаление, выключение, перезагрузка, сон, очистка корзины) сначала спрашивают «да/нет» голосом (`actions/confirm.rs`).
5. Ответ LLM озвучивается `tts.rs`: SAPI Windows или клон голоса на том же сервере.
6. Голосовой сервер `tools/voice-server/` (Python: faster-whisper + coqui-tts, `/stt`, `/tts`, `/health`) Джарвис запускает в фоне сам (`voice_server.rs`), если он установлен `setup.bat`. Версии закреплены в `requirements.txt` и `setup.bat`: `torch` 2.8 (с 2.9 нужен FFmpeg), `transformers<5` (5.x ломает coqui-tts).

Пользовательские настройки (ключи LLM, TTS, псевдонимы программ, игр и папок, разрешённые папки) лежат в `assistant.toml` в каталоге конфигурации (`%APPDATA%\com.priler.jarvis\`). Шаблон — `crates/jarvis-core/assets/assistant.default.toml`, он создаётся при первом запуске.

## Сборка и проверки

Целевая платформа — **Windows x64**. Контейнер Claude Code on the web — Linux, поэтому:

```
# проверка типов под Linux (ort-sys не может скачать бинарники через прокси — DOCS_RS=1 отключает линковку)
DOCS_RS=1 cargo check -p jarvis-core
# кросс-проверка под Windows (нужны target x86_64-pc-windows-gnu и mingw, ставит SessionStart-хук)
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
# unit-тесты новых модулей (без vosk/ort, на Linux)
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib
# голосовой сервер: модели подменяются фейками, нужны только numpy и pytest
cd tools/voice-server && python -m pytest -q
```

Настоящая сборка `.exe` и установщика `JarvisSetup.exe` (Inno Setup, `installer/jarvis.iss` + `installer/configure.ps1`) — GitHub Actions (`.github/workflows/windows.yml`, `windows-latest`), артефакт скачивается со страницы запуска. Звук, микрофон и действия Windows проверяются только на реальном ПК: не выдавай их за проверенные.

## GUI

Главная страница окна — шар `frontend/src/components/elements/VoiceOrb.svelte` (canvas). Данные: `IpcEvent::AudioLevel` (уровень и 16 полос спектра из `visual.rs`, ~30/с) и `IpcEvent::Speaking`. Скрипты PowerShell проверяются парсером `pwsh` (`[System.Management.Automation.Language.Parser]::ParseFile`), UTF-8 с BOM и CRLF — иначе Windows PowerShell 5.1 портит кириллицу.

## Журналы и обновления

- `%APPDATA%\com.priler.jarvis\`: `log.txt` (jarvis-app, debug; в начале версия и сборка), `gui-log.txt` (окно: клики, маршруты, ошибки UI через `ui_log`), `voice-server.log`. Кнопка «Собрать логи» (`collect_logs`) пакует их в zip на рабочий стол, ключи маскируются (`mask_secrets`). Разбор бага начинать с этих файлов.
- Версия: CI задаёт `JARVIS_VERSION=0.2.<run_number>` и `JARVIS_BUILD=<sha>` (зашиваются через `option_env!`), публикует `version.json` в релиз `latest`. Окно сравнивает версии (`check_update`) и запускает новый `JarvisSetup.exe /SILENT`; установщик в тихом режиме не трогает голосовой сервер и выбор голоса и сам перезапускает Джарвиса.
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
