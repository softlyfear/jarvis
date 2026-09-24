// Keyboard shortcuts, typing and the foreground window: "закрой вкладку", "сверни окно",
// "напечатай привет". Keys go to whatever window is in front, as if pressed by hand.

use super::ActionError;

// the shortcuts a voice command (command.toml args.keys) or the LLM may press
pub const NAMED_HOTKEYS: &[(&str, &str)] = &[
    ("new_tab", "ctrl+t"),
    ("close_tab", "ctrl+w"),
    ("reopen_tab", "ctrl+shift+t"),
    ("next_tab", "ctrl+tab"),
    ("previous_tab", "ctrl+shift+tab"),
    ("refresh", "f5"),
    ("fullscreen", "f11"),
    ("copy", "ctrl+c"),
    ("paste", "ctrl+v"),
    ("cut", "ctrl+x"),
    ("undo", "ctrl+z"),
    ("redo", "ctrl+y"),
    ("select_all", "ctrl+a"),
    ("save", "ctrl+s"),
    ("zoom_in", "ctrl+plus"),
    ("zoom_out", "ctrl+minus"),
    ("zoom_reset", "ctrl+0"),
    ("switch_window", "alt+tab"),
    ("task_view", "win+tab"),
    ("snap_left", "win+left"),
    ("snap_right", "win+right"),
    ("emoji", "win+period"),
    ("clipboard_history", "win+v"),
    ("task_manager", "ctrl+shift+esc"),
    ("game_bar", "win+g"),
    ("record_last", "win+alt+g"),
    ("record_toggle", "win+alt+r"),
    ("enter", "enter"),
    ("escape", "esc"),
    ("space", "space"),
    ("page_down", "pagedown"),
    ("page_up", "pageup"),
];

pub fn named_hotkey(name: &str) -> Option<&'static str> {
    NAMED_HOTKEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

fn key_code(name: &str) -> Option<u8> {
    let code = match name {
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        "win" => 0x5B,
        "tab" => 0x09,
        "enter" => 0x0D,
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "printscreen" => 0x2C,
        "plus" => 0xBB,
        "minus" => 0xBD,
        "period" => 0xBE,
        _ => {
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                // letters and digits share their ASCII upper-case codes
                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u8,
                (Some('f'), Some(_)) => match name[1..].parse::<u8>() {
                    Ok(n @ 1..=12) => 0x6F + n,
                    _ => return None,
                },
                _ => return None,
            }
        }
    };
    Some(code)
}

// "ctrl+shift+t" -> virtual key codes, modifiers first as written
pub fn parse_combo(combo: &str) -> Result<Vec<u8>, ActionError> {
    let keys: Option<Vec<u8>> = combo.to_lowercase().split('+').map(|k| key_code(k.trim())).collect();
    match keys {
        Some(k) if !k.is_empty() => Ok(k),
        _ => Err(ActionError::Failed(format!("unknown key combination: {}", combo))),
    }
}

fn keys(r: Result<(), String>) -> Result<(), ActionError> {
    r.map_err(|e| if cfg!(windows) { ActionError::Failed(e) } else { ActionError::Unsupported })
}

// a named shortcut ("close_tab") or a combination ("ctrl+w")
pub fn press(hotkey: &str) -> Result<(), ActionError> {
    let combo = named_hotkey(hotkey).unwrap_or(hotkey);
    let codes = parse_combo(combo)?;
    keys(super::platform::press_combo(&codes))
}

pub const WINDOW_ACTIONS: &[&str] = &["minimize", "maximize", "restore", "close"];

// the window in front: the one the user is looking at
pub fn window(action: &str) -> Result<(), ActionError> {
    if !WINDOW_ACTIONS.contains(&action) {
        return Err(ActionError::Failed(format!("unknown window action: {}", action)));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, PostMessageW, ShowWindow, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, WM_CLOSE,
        };
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return Err(ActionError::NotFound("нет активного окна".into()));
            }
            match action {
                "minimize" => {
                    ShowWindow(hwnd, SW_MINIMIZE);
                }
                "maximize" => {
                    ShowWindow(hwnd, SW_MAXIMIZE);
                }
                "restore" => {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                // the polite request: the program may still ask to save
                _ => {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(ActionError::Unsupported)
    }
}

const MAX_TYPED_CHARS: usize = 2000;

// Whisper's text comes lower-case and without the final dot; a sentence reads better capitalized
pub fn sentence(text: &str) -> String {
    let t = text.trim();
    let mut chars = t.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

// types text into the window in front (Unicode input, any keyboard layout)
pub fn type_text(text: &str) -> Result<(), ActionError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что напечатать".into()));
    }
    if text.chars().count() > MAX_TYPED_CHARS {
        return Err(ActionError::Denied("слишком длинный текст".into()));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
        };
        let key = |unit: u16, flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: 0, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        };
        for unit in text.encode_utf16() {
            let pair = [key(unit, KEYEVENTF_UNICODE), key(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)];
            let sent = unsafe { SendInput(2, pair.as_ptr(), std::mem::size_of::<INPUT>() as i32) };
            if sent != 2 {
                return Err(ActionError::Failed("Windows не принял ввод текста".into()));
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(ActionError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_parse_to_virtual_keys() {
        assert_eq!(parse_combo("ctrl+shift+t").unwrap(), vec![0x11, 0x10, 0x54]);
        assert_eq!(parse_combo("Win+Alt+R").unwrap(), vec![0x5B, 0x12, 0x52]);
        assert_eq!(parse_combo("f5").unwrap(), vec![0x74]);
        assert_eq!(parse_combo("f12").unwrap(), vec![0x7B]);
        assert_eq!(parse_combo("ctrl+0").unwrap(), vec![0x11, 0x30]);
        assert!(parse_combo("ctrl+f13").is_err());
        assert!(parse_combo("ctrl+жжж").is_err());
        assert!(parse_combo("").is_err());
    }

    #[test]
    fn every_named_hotkey_parses() {
        for (name, combo) in NAMED_HOTKEYS {
            assert!(parse_combo(combo).is_ok(), "{} = {}", name, combo);
        }
        assert_eq!(named_hotkey("close_tab"), Some("ctrl+w"));
        assert_eq!(named_hotkey("format_c"), None);
    }

    #[test]
    fn typed_text_is_checked() {
        assert!(matches!(type_text("  "), Err(ActionError::NotFound(_))));
        assert!(matches!(type_text(&"а".repeat(MAX_TYPED_CHARS + 1)), Err(ActionError::Denied(_))));
        assert_eq!(sentence(" привет мир"), "Привет мир");
        assert_eq!(sentence(""), "");
        assert!(window("format").is_err());
    }
}
