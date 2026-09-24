// Fork additions: assistant.toml settings (Kilo key, free-only mode, speech recognition, answer voice),
// voice server status and the command list grouped by pack.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use jarvis_core::assistant_config::{self, EditableSettings};
use jarvis_core::commands;
use serde::Serialize;

fn config_path() -> Result<std::path::PathBuf, String> {
    assistant_config::path().ok_or_else(|| "config directory is not set".to_string())
}

#[tauri::command]
pub fn assistant_settings_read() -> Result<EditableSettings, String> {
    assistant_config::read_editable_from(&config_path()?)
}

#[tauri::command]
pub fn assistant_settings_write(settings: EditableSettings) -> Result<(), String> {
    assistant_config::write_editable_to(&config_path()?, &settings)
}

// open assistant.toml in Notepad for aliases and advanced options
#[tauri::command]
pub fn open_assistant_config() -> Result<(), String> {
    let p = config_path()?;
    assistant_config::read_editable_from(&p)?; // creates the file if missing
    let program = if cfg!(windows) { "notepad.exe" } else { "xdg-open" };
    std::process::Command::new(program)
        .arg(&p)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Default)]
pub struct VoiceServerStatus {
    pub installed: bool,
    pub running: bool,
    // from /health: the graphics card and what Whisper and the voice run on
    pub gpu: Option<String>,
    pub stt_engine: Option<String>,
    pub tts_device: Option<String>,
}

// async + blocking pool: the health request must not freeze the window
#[tauri::command]
pub async fn voice_server_status() -> VoiceServerStatus {
    tauri::async_runtime::spawn_blocking(voice_server_status_blocking).await.unwrap_or_default()
}

fn voice_server_status_blocking() -> VoiceServerStatus {
    // same lookup as jarvis_core::voice_server::python_path (that module needs the "reqwest" feature)
    let dir = jarvis_core::APP_DIR.join("tools").join("voice-server");
    let installed = if cfg!(windows) {
        dir.join("python").join("python.exe").exists() || dir.join(".venv").join("Scripts").join("python.exe").exists()
    } else {
        dir.join("python").join("bin").join("python3").exists() || dir.join(".venv").join("bin").join("python").exists()
    };
    let addr: SocketAddr = "127.0.0.1:5055".parse().expect("valid address");
    let mut status = VoiceServerStatus {
        installed,
        running: TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_ok(),
        ..Default::default()
    };
    if status.running {
        let health = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(1500))
            .build()
            .ok()
            .and_then(|c| c.get("http://127.0.0.1:5055/health").send().ok())
            .and_then(|r| r.json::<serde_json::Value>().ok());
        if let Some(h) = health {
            let field = |k: &str| h.get(k).and_then(|v| v.as_str()).map(str::to_string);
            status.gpu = field("gpu");
            status.stt_engine = field("stt_engine");
            status.tts_device = field("tts_device");
        }
        log::info!("voice server: gpu={:?} stt={:?} tts={:?}", status.gpu, status.stt_engine, status.tts_device);
    }
    status
}

#[derive(Serialize)]
pub struct CommandInfo {
    pub id: String,
    pub kind: String,
    pub phrases: Vec<String>,
}

#[derive(Serialize)]
pub struct CommandPack {
    pub pack: String,
    pub commands: Vec<CommandInfo>,
}

#[tauri::command]
pub fn get_command_packs() -> Vec<CommandPack> {
    let lang = jarvis_core::i18n::get_language();
    let mut packs: Vec<CommandPack> = commands::parse_commands()
        .unwrap_or_default()
        .iter()
        .map(|list| CommandPack {
            pack: list.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            commands: list
                .commands
                .iter()
                .map(|c| CommandInfo {
                    id: c.id.clone(),
                    kind: c.cmd_type.clone(),
                    phrases: c.get_phrases(&lang).iter().cloned().collect(),
                })
                .collect(),
        })
        .collect();
    packs.sort_by(|a, b| a.pack.cmp(&b.pack));
    packs
}

// frontend events: clicks, navigation, UI errors
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    let message: String = message.chars().take(2000).collect();
    match level.as_str() {
        "error" => error!("[ui] {}", message),
        "warn" => warn!("[ui] {}", message),
        _ => info!("[ui] {}", message),
    }
}

// hide API keys before logs leave the computer
pub fn mask_secrets(text: &str) -> String {
    text.lines()
        .map(|line| if line.trim_start().starts_with("keys") && line.contains('=') { mask_quoted(line) } else { mask_prefixed(line) })
        .collect::<Vec<_>>()
        .join("\n")
}

fn masked(key: &str) -> String {
    let tail: String = key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let head: String = key.chars().take(4).collect();
    format!("{}…{}", head, tail)
}

// keys = ["...", "..."]: every string, whatever format Google uses this year
fn mask_quoted(line: &str) -> String {
    let parts: Vec<&str> = line.split('"').collect();
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| if i % 2 == 1 && p.chars().count() > 8 { masked(p) } else { p.to_string() })
        .collect::<Vec<_>>()
        .join("\"")
}

// keys elsewhere (logs): Gemini "AIza..." / "AQ." and Kilo's JWT "eyJ..."
fn mask_prefixed(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..chars.len().min(i + 4)].iter().collect();
        if rest.starts_with("AIza") || rest.starts_with("AQ.") || rest.starts_with("eyJ") {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || matches!(chars[j], '_' | '-' | '.')) {
                j += 1;
            }
            let key: String = chars[i..j].iter().collect();
            out.push_str(&masked(&key));
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

// all logs + settings (keys masked) into one zip on the Desktop; returns its path
#[tauri::command]
pub fn collect_logs() -> Result<String, String> {
    let config_dir = jarvis_core::APP_CONFIG_DIR.get().ok_or("config directory is not set")?.clone();
    let stamp = chrono_like_stamp();
    let staging = std::env::temp_dir().join(format!("jarvis-logs-{}", stamp));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let mut copied = 0;
    if let Ok(entries) = std::fs::read_dir(&config_dir) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let wanted = name.ends_with(".txt") || name.ends_with(".log") || name == "assistant.toml" || name == "app.db";
            if !p.is_file() || !wanted {
                continue;
            }
            match std::fs::read(&p) {
                Ok(bytes) => {
                    let text = mask_secrets(&String::from_utf8_lossy(&bytes));
                    let _ = std::fs::write(staging.join(&name), text);
                    copied += 1;
                }
                Err(e) => warn!("collect_logs: {}: {}", name, e),
            }
        }
    }
    info!("collect_logs: {} file(s) from {}", copied, config_dir.display());

    let desktop = std::env::var("USERPROFILE")
        .map(|h| std::path::PathBuf::from(h).join("Desktop"))
        .unwrap_or_else(|_| std::env::temp_dir());
    let zip = desktop.join(format!("jarvis-logs-{}.zip", stamp));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", "Compress-Archive -Path \"$env:JARVIS_SRC\\*\" -DestinationPath $env:JARVIS_DST -Force"])
            .env("JARVIS_SRC", &staging)
            .env("JARVIS_DST", &zip)
            .creation_flags(0x0800_0000)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("Compress-Archive failed: {:?}", status.code()));
        }
        let _ = std::fs::remove_dir_all(&staging);
        Ok(zip.to_string_lossy().to_string())
    }
    #[cfg(not(windows))]
    {
        let _ = zip;
        Ok(staging.to_string_lossy().to_string())
    }
}

fn chrono_like_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_masked() {
        let t = "keys = [\"AIzaSyA1234567890abcdefghijk_LMNO\", \"AIzaXY\"]";
        let m = mask_secrets(t);
        assert!(!m.contains("1234567890"));
        assert!(m.contains("AIza…LMNO"));
        assert_eq!(mask_secrets("no keys here"), "no keys here");
        // the newer key format, in the config and in a log line
        let m = mask_secrets("keys = [\"AQ.Ab8RN6Jv8m9NgOO29qFpTt_secret_tail\"]\nkey AQ.Ab8RN6Jv8m9NgOO29qFpTt_secret_tail used");
        assert!(!m.contains("secret"), "{}", m);
        assert!(m.contains("AQ.A…tail"));
        assert_eq!(mask_secrets("a\nb"), "a\nb");
        let m = mask_secrets("error for eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig_secret_tail here");
        assert!(!m.contains("eyJzdWIi") && m.contains("here"), "{}", m);
    }
}
