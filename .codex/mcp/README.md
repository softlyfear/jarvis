# MCP для работы над Джарвисом

Проверено 05.10.2026 на Linux, Codex CLI 0.160.0. Это инструменты разработки;
пользовательскому установщику Джарвиса они не нужны.

| Инструмент | Для чего | Подключение |
|---|---|---|
| Serena 1.7.0 | Символы Rust/Python/TypeScript/Svelte, места вызовов, поиск по коду | Проектный stdio MCP, только шесть инструментов чтения |
| Playwright MCP 0.0.83 | Снимок страницы, клики, скриншоты, консоль и сеть | Проектный stdio MCP, отдельный временный браузер |
| Context7 | Документация библиотек перед изменением интеграций | Уже подключённый App; resolve/query проверены |
| GitHub | Чтение репозитория, задач и изменений | Уже подключённый App; поиск `softlyfear/jarvis` проверен |
| Hugging Face | Поиск моделей и чтение карточек | App читает карточки; актуальный поиск — прямой HTTP MCP |

Serena использует закреплённый набор Python-зависимостей, Playwright — npm lockfile
и соответствующий Chromium headless shell. Rust, Node, Bash, Git, uv и
`pyright-langserver` должны быть доступны в PATH. Pyright берётся из подготовленного
окружения (при проверке 1.1.414): повторный `uvx` не нужен. Серверы TypeScript и
Svelte Serena устанавливает автоматически внутри своего проектного кеша.
Версии этих языковых серверов задаются пакетом Serena; при его обновлении
нужно повторить проверку. Скрипты рассчитаны на Linux/WSL, как среда разработки.

```bash
python3 .codex/mcp/setup.py
codex mcp list
.codex/.cache/mcp-serena/venv/bin/python .codex/tools/check_mcp.py --output .codex/.tmp/mcp-check.json
```

Установка повторяема; `--server serena` или `--server playwright` выбирает одну
часть. Конфигурация уже записана в `.codex/config.toml`; клиенту, который читает
проектные настройки, нужен новый сеанс для загрузки новых инструментов.
Настройки Claude, глобальная конфигурация Codex и токены Apps не изменяются.

Запуск определяет Git-корень даже из подкаталога. Конфигурация Serena копируется
атомарно; её home, настройки проекта и языковые кеши лежат в `.codex/.cache/`.
Проект работает на чтение: MCP не предоставляет редактирование, shell и запись
памяти. Основные правила остаются в `.codex/AGENTS.md`.

Rust-анализатор настроен на Windows GNU target; автоматические Cargo build scripts,
proc macros и check-on-save отключены. Это навигация по коду без фоновой сборки
Tauri/GTK на Linux. Проверки типов и тесты выполняются отдельными командами
из `AGENTS.md`; результаты навигации не заменяют их.

Playwright работает с `--headless --isolated --no-webmcp`, без подключения к
личному браузеру. Штатный клиент видит только перечисленные в `enabled_tools`
инструменты; `browser_run_code_unsafe` не включён. Скриншоты с явным именем нужно
сохранять по абсолютному пути внутри `.codex/.tmp/`; параметр `--output-dir`
действует лишь на автоматически именованные файлы. Для интерфейса Джарвиса
используется навык `gui-check` с моками Tauri; MCP сам по себе не даёт доступа
к микрофону и окнам Windows.
В текущем Linux окружении Chromium не смог создать свою песочницу, поэтому
браузер запускается с `--no-sandbox`; политики и разрешения самого Codex не
изменяются. Браузер использует пустой профиль, без пользовательских cookies.

Проверка `check_mcp.py` запускает **настоящие серверы из проектной конфигурации**,
согласует протокол и список инструментов, находит Rust-функцию и место вызова,
открывает локальную тестовую страницу, нажимает кнопку и проверяет новый текст,
сохраняет скриншот и читает ошибки консоли. Это проверка MCP на Linux;
она не закрывает аппаратную приёмку Джарвиса. `--discover` только показывает
схемы инструментов и сохраняет статус `discovered`, а не успешную приёмку.

На этой машине прямое HTTP-подключение `https://mcp.context7.com/mcp` не ответило,
но подключённый Context7 App выполнил resolve и query. Это различие сохранено
в доказательствах; внешний доступ не объявляется рабочим только по конфигурации.
Прямой сервер сохранён в конфигурации с `enabled = false`, чтобы не тратить
время запуска на неработающий транспорт; используется подключённый App.
У Hugging Face App чтение `hub_repo_details` работает, но старые `model_search`
и `hf_doc_fetch` возвращают `Tool not found`. Поэтому добавлен официальный
`https://huggingface.co/mcp`: согласование протокола обнаружило сервер 0.4.28
и актуальные `hub_repo_search`, `hub_repo_details`, `hf_fs`. Поиск публичных
моделей работает без токена; Apps и их права не изменены. Для текущего сеанса,
где новые инструменты ещё не загружены в клиент, есть прямой MCP-клиент:

```bash
.codex/.cache/mcp-serena/venv/bin/python .codex/tools/hf_mcp.py --discover --output .codex/.tmp/hf-tools.json
.codex/.cache/mcp-serena/venv/bin/python .codex/tools/hf_mcp.py --tool hub_repo_search --arguments '{"query":"ESpeech-TTS-1_RL-V2","repo_types":["model"],"limit":2}' --output .codex/.tmp/hf-search.json
```

Windows-MCP и MCP для живого Tauri не добавлены: доступного Windows-стенда сейчас
нет. Отдельные файловый/Git/shell MCP не нужны — эти действия уже доступны
штатным инструментам Codex.

Первичные источники:
[Serena](https://github.com/oraios/serena),
[Playwright MCP](https://github.com/microsoft/playwright-mcp),
[MCP Python SDK](https://github.com/modelcontextprotocol/python-sdk),
[Hugging Face MCP](https://huggingface.co/docs/hub/en/agents-mcp),
[настройки MCP Codex](https://developers.openai.com/codex/mcp/).
