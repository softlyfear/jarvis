---
name: windows-build
description: Сборка, упаковка и выпуск Джарвиса под Windows — GitHub Actions (.github/workflows/windows.yml), разбор упавшей сборки, состав архива, кросс-проверка из Linux-контейнера. Использовать при красном CI, при изменении зависимостей, DLL, ресурсов или состава поставки.
---

# Сборка под Windows

## Где что происходит

| Шаг | Где |
|---|---|
| Unit-тесты на Linux и Windows | job `test` и шаг `Tests on Windows` |
| Фронтенд настроек | `frontend/`: `npm ci` + `npm test` + `npm run build`; `npm run check` сначала генерирует маршруты через `routify build`, затем проверяет типы. Собственная страница `[...404].svelte` устраняет ошибку типов шаблона Routify |
| Бинарники | `cargo build --release -p jarvis-app`, затем отдельно `-p jarvis-gui` (MSVC). Вместе нельзя: объединение features `jarvis-core` заставит GUI линковаться с Vosk |
| Vosk | линкуется с `lib/windows/amd64/libvosk.lib` (путь задаёт `crates/jarvis-app/build.rs`), DLL кладутся рядом с exe |
| ONNX Runtime | `ort` с `download-binaries` скачивает сам на раннере |
| Архив | шаг `Package`: exe + DLL + `resources` (только русская модель Vosk) + `tools` + `ИНСТРУКЦИЯ.md` |
| Установщик | `installer/jarvis.iss` (Inno Setup 6, `choco install innosetup`), `ISCC /DAppVersion=…`; первичная настройка ключей — `installer/configure.ps1` (ключи через временный файл, не через командную строку) |
| Публикация | пре-релиз `latest` (`softprops/action-gh-release`), скачивается без логина |

## Разбор падения

1. Лог шага для `softlyfear/jarvis`: `gh run list --repo softlyfear/jarvis`, затем `gh run view <run-id> --repo softlyfear/jarvis --log-failed` (или доступный GitHub MCP с чтением запусков и логов). Не предполагать наличие Claude-инструментов `mcp__github__actions_list` / `mcp__github__get_job_logs`.
2. Воспроизвести локально, насколько позволяет Linux:
   - `DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu` ловит ошибки типов и `cfg(windows)`;
   - линковку MSVC, DLL и упаковку проверяет только CI.
3. «Flake» не причина: повторный запуск — только если упало до сборки (checkout, скачивание).

## Нельзя

- Коммитить `*.exe`, `target/`, `frontend/dist` (они в `.gitignore`).
- Класть в архив модели en/uk без нужды: +330 МБ.
- Удалять `lib/windows/amd64/*.dll`: без них exe не стартует.
