---
name: add-voice-command
description: Добавить или изменить голосовую команду Джарвиса — пак в resources/commands (action, lua, voice), нативное действие в actions/ и инструмент для LLM. Использовать, когда просят «научи Джарвиса …», «добавь команду …», «сделай, чтобы по фразе … происходило …».
---

# Новая голосовая команда

## 1. Выбрать механизм

| Нужно | Механизм |
|---|---|
| Открыть программу, игру, сайт, папку по названию | ничего не писать: псевдоним в `[apps]`, `[games]` или `[folders]` шаблона `crates/jarvis-core/assets/assistant.default.toml` |
| Существующее нативное действие по новой фразе | добавить фразы в `resources/commands/<pack>/command.toml` |
| Новое действие ОС | новый вариант `Action` в `crates/jarvis-core/src/actions.rs` + `from_voice_command` + (если полезно нейросети) tool в `llm/tools.rs` |
| Сетевой запрос, состояние, логика без Windows API | Lua-скрипт (`type = "lua"`, пример — `resources/commands/weather`) |
| Только реакция записанным голосом | `type = "voice"` + `sounds.ru = [...]` (имена файлов из `resources/sound/voices/*/ru`) |

## 2. Формат пака

```toml
[[commands]]
id = "уникальный_id"          # уникален среди всех паков
type = "action"
action = "open_app"            # id из actions::from_voice_command
args = { action = "next" }     # необязательные аргументы действия
sounds.ru = ["ok1", "ok2"]     # реакция при успехе; без неё — тишина
phrases.ru = ["открой {app}", "запусти {app}"]   # {x} — объект, извлекается text::extract_object
phrases.en = ["open {app}"]
```

- `phrases` — обязательно карта по языкам (`phrases.ru`), не массив: иначе весь пак молча не загрузится.
- Фразы учат intent-classifier: давать 4–8 разных формулировок, включая разговорные.
- Не пересекаться с чужими фразами («закрой {app}» и «закрой корзину» конфликтуют).

## 3. Новое действие

1. Вариант в `enum Action`, ветка в `Action::execute`, при необходимости — в `is_dangerous` и `confirmation_question`.
2. Реализация в подходящем модуле `actions/*.rs`; вызовы Windows — за `#[cfg(windows)]`, ввод пользователя только через аргументы процесса или env (`platform::powershell(script, env)`).
3. Разбор фразы в `from_voice_command`, tool в `llm/tools.rs` (`definitions` + `to_action`).
4. Тесты: `from_voice_command` / `to_action` для нового id. Тест `bundled_command_packs_parse_and_reference_known_actions` проверит пак.

## 4. Проверка

```
DOCS_RS=1 cargo test -p jarvis-core --no-default-features --features reqwest --lib -- actions assistant_config llm tts whisper voice_server
DOCS_RS=1 cargo check -p jarvis-app --target x86_64-pc-windows-gnu
```

Изменение фраз меняет хеш команд: классификатор переобучится при следующем запуске сам.
