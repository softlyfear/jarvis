// Standard editor dialogs, bound to the inspected window and button identity.
use std::time::{Duration, Instant};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Deserialize;
use super::{input, text::normalize, ActionError, ActionOutcome};

pub const CHOICES: &[&str] = &["save", "dont_save", "cancel", "yes", "no", "ok"];
const TIMEOUT: Duration = Duration::from_secs(120);
static PENDING: Lazy<Mutex<Option<(Snapshot, Instant)>>> = Lazy::new(|| Mutex::new(None));

#[derive(Clone, Debug, Deserialize)]
struct Button { name: String, id: String }

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    handle: isize,
    pid: u32,
    title: String,
    buttons: Vec<Button>,
    #[serde(default)]
    closing_processes: Vec<String>,
}

fn choice(name: &str) -> Option<&'static str> {
    let name = normalize(&name.replace('&', "").replace(['’', '‘'], "'"));
    match name.as_str() {
        "сохранить" | "save" => Some("save"),
        "не сохранять" | "don't save" | "don t save" | "do not save" | "dont save" => Some("dont_save"),
        "отмена" | "cancel" => Some("cancel"),
        "да" | "yes" => Some("yes"),
        "нет" | "no" => Some("no"),
        "ok" | "ок" | "окей" => Some("ok"),
        _ => None,
    }
}

impl Snapshot {
    fn button(&self, wanted: &str) -> Result<&Button, ActionError> {
        let mut buttons = self.buttons.iter().filter(|b| choice(&b.name) == Some(wanted));
        let button = buttons.next().ok_or_else(|| ActionError::NotFound("такой кнопки в диалоге нет".into()))?;
        if buttons.next().is_some() {
            return Err(ActionError::Denied("в окне несколько одинаковых кнопок; уточните диалог".into()));
        }
        Ok(button)
    }

    fn save_question(&self) -> bool {
        self.button("save").is_ok() && self.button("dont_save").is_ok() && self.button("cancel").is_ok()
    }
}

#[cfg(windows)]
fn automation(handle: isize, pid: u32, button: Option<&Button>) -> Result<String, ActionError> {
    use std::process::Stdio;
    let mut cmd = super::platform::hidden_command("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-MTA", "-Command", include_str!("dialog.ps1").trim_start_matches('\u{feff}')]);
    cmd.env("JARVIS_WINDOW", handle.to_string()).env("JARVIS_PID", if pid == 0 { String::new() } else { pid.to_string() });
    // Override inherited values as well as supplying the inspected identity.
    cmd.env("JARVIS_BUTTON_ID", button.map(|b| b.id.as_str()).unwrap_or(""));
    cmd.env("JARVIS_BUTTON_NAME", button.map(|b| b.name.as_str()).unwrap_or(""));
    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()
        .map_err(|e| ActionError::Failed(format!("не удалось прочитать диалог: {}", e)))?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ActionError::Failed("диалог не ответил вовремя".into()));
            }
        }
    }
    let output = child.wait_with_output().map_err(|e| ActionError::Failed(e.to_string()))?;
    if !output.status.success() {
        return Err(ActionError::Failed("диалог изменился или не поддерживает управление кнопками".into()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(not(windows))]
fn automation(_handle: isize, _pid: u32, _button: Option<&Button>) -> Result<String, ActionError> {
    Err(ActionError::Unsupported)
}

pub fn inspect(handle: isize) -> Result<Snapshot, ActionError> {
    let text = automation(handle, 0, None)?;
    let mut snapshot: Snapshot = serde_json::from_str(&text).map_err(|_| ActionError::Failed("не удалось прочитать кнопки окна".into()))?;
    snapshot.handle = handle;
    Ok(snapshot)
}

pub fn inspect_active() -> Result<ActionOutcome, ActionError> {
    let window = input::target_window()?;
    let snapshot = inspect(window.handle)?;
    if snapshot.save_question() { return Ok(request(snapshot)); }
    Ok(ActionOutcome::done(format!("окно «{}»; доступные кнопки: {}", snapshot.title,
        snapshot.buttons.iter().take(30).map(|b| b.name.as_str()).collect::<Vec<_>>().join(", "))))
}

pub fn request(snapshot: Snapshot) -> ActionOutcome {
    let question = format!("В окне «{}» есть несохранённые изменения. Сохранить? Скажите «сохранить», «не сохранять» или «отмена».", snapshot.title);
    *PENDING.lock() = Some((snapshot, Instant::now()));
    ActionOutcome { chain: true, speech: Some(question.clone()), report: question }
}

pub fn has_pending() -> bool {
    PENDING.lock().as_ref().is_some_and(|(_, at)| at.elapsed() < TIMEOUT)
}

pub fn clear() {
    *PENDING.lock() = None;
}

fn spoken_choice(text: &str) -> Option<&'static str> {
    let t = normalize(text);
    let t = t.strip_prefix("джарвис ").unwrap_or(&t).trim();
    let t = t.strip_prefix("нажми кнопку ").or_else(|| t.strip_prefix("нажми "))
        .or_else(|| t.strip_prefix("выбери ")).unwrap_or(t);
    match t {
        "сохранить" | "сохрани" | "сохраняй" | "да" | "да сохрани" | "сохрани изменения" | "сохрани файл" => Some("save"),
        "не сохранять" | "не сохраняй" | "без сохранения" | "нет" | "нет не сохраняй" | "закрой без сохранения" | "не сохраняй изменения" => Some("dont_save"),
        "отмена" | "отмени" | "отмени закрытие" | "не закрывай" | "отмена диалога" => Some("cancel"),
        _ => None,
    }
}

pub fn answer(text: &str) -> Option<Result<ActionOutcome, ActionError>> {
    let (snapshot, at) = PENDING.lock().take()?;
    if at.elapsed() >= TIMEOUT { return None; }
    let wanted = spoken_choice(text)?;
    Some(invoke(snapshot, wanted))
}

pub fn press(wanted: &str) -> Result<ActionOutcome, ActionError> {
    if !CHOICES.contains(&wanted) { return Err(ActionError::Denied("неизвестный вариант диалога".into())); }
    let pending = PENDING.lock().take().filter(|(_, at)| at.elapsed() < TIMEOUT);
    let snapshot = match pending {
        Some((snapshot, _)) => snapshot,
        None => inspect(input::target_window()?.handle)?,
    };
    invoke(snapshot, wanted)
}

fn invoke(snapshot: Snapshot, wanted: &str) -> Result<ActionOutcome, ActionError> {
    let button = snapshot.button(wanted)?;
    automation(snapshot.handle, snapshot.pid, Some(button))?;
    // Invoke acknowledges input, not a completed save/close. Reinspect after the transition.
    std::thread::sleep(Duration::from_millis(200));
    let windows = input::windows_on_screen();
    if !snapshot.closing_processes.is_empty() {
        if windows.iter().all(|w| !snapshot.closing_processes.contains(&w.process)) {
            return Ok(ActionOutcome::said(format!("Нажал «{}». Окна программы закрыты.", button.name)));
        }
        if wanted != "cancel" {
            if let Some(question) = save_prompt_for(&snapshot.closing_processes) { return Ok(question); }
        }
    }
    let speech = if windows.iter().all(|w| w.handle != snapshot.handle) {
        format!("Нажал «{}». Диалог закрыт; закрытие программы не подтверждено.", button.name)
    } else if let Ok(next) = inspect(snapshot.handle) {
        if next.save_question() { return Ok(request(next)); }
        format!("Нажал «{}». Сейчас открыто окно «{}»; сохранение и закрытие ещё не подтверждены.", button.name, next.title)
    } else {
        format!("Нажал «{}». Результат проверить не удалось.", button.name)
    };
    Ok(ActionOutcome::said(speech))
}

pub fn save_prompt_for(processes: &[String]) -> Option<ActionOutcome> {
    for window in input::windows_on_screen().iter().filter(|w| processes.contains(&w.process)).take(5) {
        if let Ok(mut snapshot) = inspect(window.handle) {
            if snapshot.save_question() {
                snapshot.closing_processes = processes.to_vec();
                return Some(request(snapshot));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(names: &[&str]) -> Snapshot {
        Snapshot { handle: 123, pid: 456, title: "Блокнот".into(), buttons: names.iter().enumerate().map(|(i, n)| Button { name: n.to_string(), id: i.to_string() }).collect(), closing_processes: Vec::new() }
    }
    #[test]
    fn localized_buttons_are_exact_and_unambiguous() {
        let s = snapshot(&["&Save", "Don’t Save", "Cancel"]);
        assert!(s.save_question());
        assert_eq!(s.button("dont_save").unwrap().id, "1");
        assert!(snapshot(&["Сохранить", "Не сохранять", "Отмена"]).save_question());
        assert!(!snapshot(&["Сохранить как", "Отмена"]).save_question());
        assert!(snapshot(&["Save", "&Save"]).button("save").is_err());
        assert!(snapshot(&["Save all", "Discard changes"]).button("save").is_err());
    }
    #[test]
    fn negative_save_answer_cannot_become_save_or_a_new_command() {
        assert_eq!(spoken_choice("не сохраняй"), Some("dont_save"));
        assert_eq!(spoken_choice("да"), Some("save"));
        assert_eq!(spoken_choice("джарвис отмена"), Some("cancel"));
        assert_eq!(spoken_choice("да открой браузер"), None);
        assert_eq!(spoken_choice("сохрани новую заметку"), None);
        assert_eq!(spoken_choice("не закрывай"), Some("cancel"));
        for phrase in ["нажми не сохранять", "джарвис выбери не сохранять", "закрой без сохранения", "не сохраняй изменения"] {
            assert_eq!(spoken_choice(phrase), Some("dont_save"));
        }
        assert_eq!(spoken_choice("нажми кнопку сохранить"), Some("save"));
        assert_eq!(spoken_choice("нажми отмена"), Some("cancel"));
        assert_eq!(spoken_choice("нажми сохранить и открой браузер"), None);
    }

    #[test]
    fn pending_dialog_expires_and_unrelated_commands_do_not_choose_a_button() {
        *PENDING.lock() = Some((snapshot(&["Save", "Don't Save", "Cancel"]), Instant::now() - TIMEOUT));
        assert!(!has_pending());
        assert!(answer("да").is_none());
        request(snapshot(&["Save", "Don't Save", "Cancel"]));
        assert!(has_pending());
        assert!(answer("да открой браузер").is_none());
        assert!(!has_pending());
    }

    #[cfg(windows)]
    #[test]
    fn real_uia_reads_buttons_and_rejects_a_changed_owner() {
        use std::process::Stdio;
        use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("dialog-test.ps1");
        let title = format!("Jarvis UIA smoke test {}", dir.path().file_name().unwrap().to_string_lossy());
        let title_wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
        let source = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.MessageBox]::Show('Choose a button', $env:JARVIS_TEST_TITLE, [System.Windows.Forms.MessageBoxButtons]::YesNoCancel)
"#;
        std::fs::write(&script, format!("\u{feff}{}", source.replace('\n', "\r\n"))).unwrap();
        let mut child = super::super::platform::hidden_command("powershell")
            .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script)
            .env("JARVIS_TEST_TITLE", &title)
            .stdout(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
        let result = (|| {
            let start = Instant::now();
            let window = loop {
                // Locate our fixture directly; process enumeration is unrelated to
                // this UIA test and competes with parallel PowerShell fixtures.
                let handle = unsafe { FindWindowW(std::ptr::null(), title_wide.as_ptr()) };
                if !handle.is_null() {
                    break handle as isize;
                }
                if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                    return Err(format!("Native test dialog exited before appearing: {}", status));
                }
                if start.elapsed() >= Duration::from_secs(30) {
                    return Err("Native test dialog did not appear".to_string());
                }
                std::thread::sleep(Duration::from_millis(50));
            };
            let s = inspect(window).map_err(|e| e.to_string())?;
            if s.pid != child.id() || s.button("yes").is_err() || s.button("cancel").is_err() {
                let diagnostic = automation(window, s.pid, None).map_err(|e| e.to_string())?;
                return Err(format!("Unexpected native dialog controls: {:?}; {}", s, diagnostic));
            }
            let button = s.button("no").map_err(|e| e.to_string())?;
            if automation(window, s.pid + 1, Some(button)).is_ok() {
                return Err("Changed window owner was accepted".to_string());
            }
            automation(window, s.pid, Some(button)).map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })();
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        if let Err(error) = result {
            panic!("{}; stderr: {}", error, String::from_utf8_lossy(&output.stderr));
        }
    }
}
