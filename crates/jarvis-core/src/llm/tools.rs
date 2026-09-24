// Tool (function) definitions offered to the LLM and their mapping to native actions.

use serde_json::{json, Value};

use crate::actions::{clock, input, Action, ActionError};

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
            }
        }
    })
}

fn no_args(name: &str, description: &str) -> Value {
    tool(name, description, json!({}), &[])
}

pub fn definitions() -> Value {
    json!([
        tool("open_app", "Открыть программу, сайт из закладок, приложение Windows или игру по названию.",
            json!({"name": {"type": "string", "description": "Название, как его назвал пользователь, например «телеграм» или «Discord»"}}), &["name"]),
        tool("close_app", "Закрыть запущенную программу или игру по названию.",
            json!({"name": {"type": "string"}}), &["name"]),
        tool("launch_game", "Запустить установленную игру (Steam или ярлык).",
            json!({"name": {"type": "string", "description": "Название игры"}}), &["name"]),
        no_args("list_games", "Список установленных игр Steam."),
        tool("open_folder", "Открыть папку в проводнике: «загрузки», «документы», «рабочий стол», «картинки» или полный путь.",
            json!({"name": {"type": "string"}}), &["name"]),
        tool("find_files", "Найти файлы и папки по имени в разрешённых папках пользователя. Возвращает полные пути.",
            json!({
                "query": {"type": "string", "description": "Часть имени файла"},
                "folder": {"type": "string", "description": "Необязательно: где искать, например «загрузки»"}
            }), &["query"]),
        tool("delete_file", "Переместить файл или папку в Корзину. Нужен полный путь из find_files. Пользователь подтвердит голосом.",
            json!({"path": {"type": "string"}}), &["path"]),
        tool("create_folder", "Создать папку по полному пути внутри разрешённых папок.",
            json!({"path": {"type": "string"}}), &["path"]),
        tool("set_volume", "Установить громкость системы в процентах.",
            json!({"level": {"type": "integer", "minimum": 0, "maximum": 100}}), &["level"]),
        tool("change_volume", "Сделать громче (положительное число) или тише (отрицательное) на указанное количество процентов.",
            json!({"delta": {"type": "integer", "minimum": -100, "maximum": 100}}), &["delta"]),
        no_args("toggle_mute", "Выключить или включить звук."),
        tool("media", "Управление музыкой и видео.",
            json!({"action": {"type": "string", "enum": ["play_pause", "next", "previous", "stop"]}}), &["action"]),
        no_args("screenshot", "Сделать скриншот всего экрана (сохраняется в Изображения)."),
        no_args("snip", "Выделить область экрана для скриншота."),
        no_args("show_desktop", "Свернуть все окна и показать рабочий стол."),
        no_args("lock_computer", "Заблокировать компьютер."),
        tool("power", "Выключить, перезагрузить, усыпить компьютер или отменить выключение. Пользователь подтвердит голосом.",
            json!({"action": {"type": "string", "enum": ["shutdown", "restart", "sleep", "cancel"]}}), &["action"]),
        no_args("empty_recycle_bin", "Очистить корзину. Пользователь подтвердит голосом."),
        tool("web_search", "Открыть в браузере поиск Яндекса по запросу.",
            json!({"query": {"type": "string"}}), &["query"]),
        tool("open_url", "Открыть веб-страницу (только http/https).",
            json!({"url": {"type": "string"}}), &["url"]),
        tool("press_keys", "Нажать сочетание клавиш в активном окне: вкладки, буфер обмена, масштаб, запись игры (Xbox Game Bar) и т. п.",
            json!({"name": {"type": "string", "enum": input::NAMED_HOTKEYS.iter().map(|(n, _)| *n).collect::<Vec<_>>()}}), &["name"]),
        tool("window", "Свернуть, развернуть, восстановить или закрыть активное окно.",
            json!({"action": {"type": "string", "enum": input::WINDOW_ACTIONS}}), &["action"]),
        tool("type_text", "Напечатать текст в активном окне, как с клавиатуры.",
            json!({"text": {"type": "string"}}), &["text"]),
        tool("set_timer", "Поставить таймер или напоминание через указанное число минут. С текстом — напоминание, без — таймер.",
            json!({
                "minutes": {"type": "number", "description": "Через сколько минут, можно дробное"},
                "text": {"type": "string", "description": "Необязательно: о чём напомнить"}
            }), &["minutes"]),
        tool("set_alarm", "Поставить будильник (или напоминание с текстом) на время суток.",
            json!({
                "time": {"type": "string", "description": "Время ЧЧ:ММ, 24-часовой формат"},
                "text": {"type": "string", "description": "Необязательно: о чём напомнить"}
            }), &["time"]),
        tool("timers", "Сколько осталось до ближайшего таймера или отменить все таймеры, будильники и напоминания.",
            json!({"action": {"type": "string", "enum": ["left", "cancel"]}}), &["action"]),
    ])
}

fn str_arg(args: &Value, key: &str) -> Result<String, ActionError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ActionError::Failed(format!("missing argument «{}»", key)))
}

fn int_arg(args: &Value, key: &str) -> Result<i64, ActionError> {
    let v = args.get(key).ok_or_else(|| ActionError::Failed(format!("missing argument «{}»", key)))?;
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f.round() as i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .ok_or_else(|| ActionError::Failed(format!("argument «{}» must be a number", key)))
}

pub fn to_action(name: &str, args: &Value) -> Result<Action, ActionError> {
    let action = match name {
        "open_app" => Action::OpenApp { name: str_arg(args, "name")? },
        "close_app" => Action::CloseApp { name: str_arg(args, "name")? },
        "launch_game" => Action::LaunchGame { name: str_arg(args, "name")? },
        "list_games" => Action::ListGames,
        "open_folder" => Action::OpenFolder { name: str_arg(args, "name")? },
        "find_files" => Action::FindFiles {
            query: str_arg(args, "query")?,
            folder: str_arg(args, "folder").ok(),
        },
        "delete_file" => Action::DeleteFile { path: str_arg(args, "path")? },
        "create_folder" => Action::CreateFolder { path: str_arg(args, "path")? },
        "set_volume" => Action::SetVolume { level: int_arg(args, "level")?.clamp(0, 100) as u32 },
        "change_volume" => {
            let delta = int_arg(args, "delta")?.clamp(-100, 100);
            if delta >= 0 {
                Action::VolumeUp { percent: delta as u32 }
            } else {
                Action::VolumeDown { percent: (-delta) as u32 }
            }
        }
        "toggle_mute" => Action::ToggleMute,
        "media" => {
            let a = str_arg(args, "action")?;
            if !["play_pause", "next", "previous", "stop"].contains(&a.as_str()) {
                return Err(ActionError::Failed(format!("unknown media action {}", a)));
            }
            Action::Media { action: a }
        }
        "screenshot" => Action::Screenshot,
        "snip" => Action::Snip,
        "show_desktop" => Action::ShowDesktop,
        "lock_computer" => Action::Lock,
        "power" => match str_arg(args, "action")?.as_str() {
            "shutdown" => Action::Shutdown,
            "restart" => Action::Restart,
            "sleep" => Action::Sleep,
            "cancel" => Action::CancelShutdown,
            other => return Err(ActionError::Failed(format!("unknown power action {}", other))),
        },
        "empty_recycle_bin" => Action::EmptyRecycleBin,
        "web_search" => Action::WebSearch { query: str_arg(args, "query")? },
        "open_url" => Action::OpenUrl { url: str_arg(args, "url")? },
        // only named shortcuts: the model does not get arbitrary key combinations
        "press_keys" => {
            let n = str_arg(args, "name")?;
            if input::named_hotkey(&n).is_none() {
                return Err(ActionError::Failed(format!("unknown shortcut {}", n)));
            }
            Action::Hotkey { keys: n }
        }
        "window" => {
            let a = str_arg(args, "action")?;
            if !input::WINDOW_ACTIONS.contains(&a.as_str()) {
                return Err(ActionError::Failed(format!("unknown window action {}", a)));
            }
            Action::Window { action: a }
        }
        "type_text" => Action::TypeText { text: str_arg(args, "text")? },
        "set_timer" => {
            let minutes = args.get("minutes").and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
            let seconds = minutes.filter(|m| *m > 0.0).map(|m| (m * 60.0).round() as u64)
                .ok_or_else(|| ActionError::Failed("argument «minutes» must be a positive number".into()))?;
            let text = str_arg(args, "text").unwrap_or_default();
            let kind = if text.is_empty() { clock::Kind::Timer } else { clock::Kind::Reminder };
            Action::SetTimer { kind, seconds, text }
        }
        "set_alarm" => {
            let time = str_arg(args, "time")?;
            let (h, m) = time
                .split_once(':')
                .and_then(|(h, m)| Some((h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?)))
                .filter(|(h, m)| *h < 24 && *m < 60)
                .ok_or_else(|| ActionError::Failed(format!("time must be HH:MM, got {}", time)))?;
            let seconds = clock::seconds_until(chrono::Local::now().naive_local(), h, m);
            let text = str_arg(args, "text").unwrap_or_default();
            let kind = if text.is_empty() { clock::Kind::Alarm } else { clock::Kind::Reminder };
            Action::SetTimer { kind, seconds, text }
        }
        "timers" => match str_arg(args, "action")?.as_str() {
            a @ ("left" | "cancel") => Action::Clock { what: a.into() },
            other => return Err(ActionError::Failed(format!("unknown timers action {}", other))),
        },
        other => return Err(ActionError::Failed(format!("unknown tool {}", other))),
    };
    Ok(action)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_defined_tool_maps_to_an_action() {
        let defs = definitions();
        let sample = json!({
            "name": "x", "query": "x", "path": "C:\\x", "level": 10, "delta": -5,
            "action": "shutdown", "url": "https://x.ru"
        });
        for d in defs.as_array().unwrap() {
            let name = d.pointer("/function/name").unwrap().as_str().unwrap();
            let args = match name {
                "media" => json!({"action": "next"}),
                "press_keys" => json!({"name": "close_tab"}),
                "window" => json!({"action": "minimize"}),
                "type_text" => json!({"text": "привет"}),
                "set_timer" => json!({"minutes": 5}),
                "set_alarm" => json!({"time": "07:30"}),
                "timers" => json!({"action": "left"}),
                _ => sample.clone(),
            };
            assert!(to_action(name, &args).is_ok(), "tool {} has no mapping", name);
        }
    }

    #[test]
    fn arguments_are_validated() {
        assert!(to_action("open_app", &json!({})).is_err());
        assert!(to_action("open_app", &json!({"name": "  "})).is_err());
        assert_eq!(to_action("set_volume", &json!({"level": "150"})).unwrap(), Action::SetVolume { level: 100 });
        assert_eq!(to_action("change_volume", &json!({"delta": -20})).unwrap(), Action::VolumeDown { percent: 20 });
        assert!(to_action("media", &json!({"action": "rm"})).is_err());
        assert!(to_action("format_c", &json!({})).is_err());
        assert!(to_action("press_keys", &json!({"name": "alt+f4"})).is_err());
        assert!(to_action("window", &json!({"action": "shutdown"})).is_err());
        assert_eq!(
            to_action("set_timer", &json!({"minutes": 1.5, "text": "чай"})).unwrap(),
            Action::SetTimer { kind: clock::Kind::Reminder, seconds: 90, text: "чай".into() }
        );
        assert!(to_action("set_timer", &json!({"minutes": -1})).is_err());
        assert!(to_action("set_alarm", &json!({"time": "25:00"})).is_err());
        assert!(matches!(to_action("set_alarm", &json!({"time": "7:05"})).unwrap(), Action::SetTimer { kind: clock::Kind::Alarm, .. }));
    }
}
