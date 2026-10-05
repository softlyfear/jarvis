// Optional, isolated OpenClaw installation and profile lifecycle.
use crate::agent_config::{AgentConfig, BackendKind};
use crate::assistant_config::{self, EditableSettings};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn root() -> Result<PathBuf, String> {
    crate::APP_CONFIG_DIR
        .get()
        .map(|p| p.join("openclaw"))
        .ok_or_else(|| "Не найден каталог настроек Джарвиса.".into())
}
fn node(root: &std::path::Path) -> PathBuf {
    root.join("runtime")
        .join("node-v24.16.0-win-x64")
        .join("node.exe")
}
fn helper(root: &std::path::Path, operation: &str) -> Command {
    let mut c = Command::new(node(root));
    c.arg(crate::APP_DIR.join("tools/openclaw/profile.mjs"))
        .arg(operation)
        .env("JARVIS_OPENCLAW_DIR", root)
        .env("JARVIS_APP_DIR", &*crate::APP_DIR);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

pub fn install(settings: &EditableSettings) -> Result<AgentConfig, String> {
    if !cfg!(windows) {
        return Err("Установка из окна доступна на Windows.".into());
    }
    let root = root()?;
    let mut c = Command::new("powershell.exe");
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ])
    .arg(crate::APP_DIR.join("tools/openclaw/install.ps1"))
    .arg("-ProfileDir")
    .arg(&root)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    if !c
        .status()
        .map_err(|_| "Не удалось запустить установку OpenClaw.")?
        .success()
    {
        return Err("Установка OpenClaw не завершена. Проверьте интернет и openclaw/setup.log в папке настроек.".into());
    }
    let path = assistant_config::path().ok_or("Не найден файл настроек.")?;
    // Read editable changes without altering the on-disk configuration on failure.
    let temp = tempfile::NamedTempFile::new().map_err(|_| "Не удалось подготовить настройки.")?;
    if path.exists() {
        std::fs::copy(&path, temp.path()).map_err(|_| "Не удалось прочитать настройки.")?;
    }
    assistant_config::write_editable_to(temp.path(), settings)
        .map_err(|_| "Не удалось прочитать настройки модели. Проверьте файл настроек.")?;
    let cfg = assistant_config::parse(
        &std::fs::read_to_string(temp.path()).map_err(|_| "Не удалось прочитать настройки.")?,
    )
    .map_err(|_| "Не удалось прочитать настройки модели. Проверьте файл настроек.")?;
    let providers: Vec<Value> = cfg.llm.active_providers().iter().map(|p| json!({
        "enabled": p.enabled, "base_url": p.base_url, "models": p.models, "keys": p.keys, "keyless": p.keyless
    })).collect();
    let mut child = helper(&root, "prepare")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Не удалось создать профиль OpenClaw.")?;
    child
        .stdin
        .take()
        .ok_or("Не удалось передать настройки.")?
        .write_all(&serde_json::to_vec(&json!({"providers": providers})).unwrap())
        .map_err(|_| "Не удалось передать настройки OpenClaw.")?;
    let output = child
        .wait_with_output()
        .map_err(|_| "Не удалось создать профиль OpenClaw.")?;
    if !output.status.success() {
        return Err(
            "Не удалось создать профиль. Добавьте ключ модели или включите бесплатные модели."
                .into(),
        );
    }
    let connection: Value =
        serde_json::from_slice(&output.stdout).map_err(|_| "Не удалось прочитать профиль.")?;
    let mut result = settings.agent.clone();
    result.backend = BackendKind::Openclaw;
    result.mcp_enabled = true;
    result.openclaw.base_url = connection["base_url"]
        .as_str()
        .ok_or("Не найден адрес Gateway.")?
        .into();
    result.openclaw.api_key = connection["api_key"]
        .as_str()
        .ok_or("Не найден токен Gateway.")?
        .into();
    result.openclaw.agent = "jarvis".into();
    result.openclaw.model.clear();
    result.openclaw.vision_model.clear();
    start(&result)?;
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if super::check_connection(&result).is_ok() {
            return Ok(result);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err("Gateway не запустился. Проверьте openclaw/gateway.log; настройку можно повторить.".into())
}

pub fn owns(cfg: &AgentConfig) -> bool {
    let Ok(root) = root() else {
        return false;
    };
    let Ok(text) = std::fs::read_to_string(root.join("openclaw.json")) else {
        return false;
    };
    let Ok(profile) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    profile["gateway"]["auth"]["token"].as_str() == Some(&cfg.openclaw.api_key)
        && profile["gateway"]["port"]
            .as_u64()
            .is_some_and(|p| cfg.openclaw.base_url == format!("http://127.0.0.1:{}", p))
        && cfg.openclaw.agent == "jarvis"
}
pub fn start(cfg: &AgentConfig) -> Result<(), String> {
    if !owns(cfg) {
        return Ok(());
    }
    let root = root()?;
    if !helper(&root, "start")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| "Не удалось запустить Gateway OpenClaw.")?
        .success()
    {
        return Err(
            "Не удалось запустить Gateway. Проверьте openclaw/gateway.log и занятый порт.".into(),
        );
    }
    Ok(())
}
pub fn models(cfg: &AgentConfig) -> Result<Vec<String>, String> {
    if !owns(cfg) {
        return Err("Для внешнего Gateway список моделей находится в его настройках.".into());
    }
    let output = helper(&root()?, "models")
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Не удалось прочитать модели.")?;
    if !output.status.success() {
        return Err("Не удалось прочитать модели профиля.".into());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|_| "Не удалось прочитать модели профиля.".into())
}

pub fn config_path(_cfg: &AgentConfig) -> Result<PathBuf, String> {
    let path = root()?.join("openclaw.json");
    if !path.is_file() {
        return Err("Сначала настройте локальный OpenClaw. Внешний Gateway настраивается на своём компьютере.".into());
    }
    Ok(path)
}

pub fn open_dashboard(cfg: &AgentConfig) -> Result<(), String> {
    if !owns(cfg) {
        return Err("Откройте настройки внешнего Gateway на компьютере с OpenClaw.".into());
    }
    let output = helper(&root()?, "dashboard")
        .stderr(Stdio::null())
        .output()
        .map_err(|_| "Не удалось открыть настройки OpenClaw.")?;
    let data: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "Gateway не готов. Проверьте подключение.")?;
    let url = data["browserUrl"]
        .as_str()
        .ok_or("Не удалось получить ссылку настроек.")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "Некорректная ссылка настроек.")?;
    let expected =
        reqwest::Url::parse(&cfg.openclaw.base_url).map_err(|_| "Некорректный адрес Gateway.")?;
    if parsed.origin() != expected.origin() {
        return Err("Gateway вернул другой адрес настроек.".into());
    }
    // Never send the one-time browser credential to the generic URL logger.
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
        let result = unsafe {
            let initialized = CoInitializeEx(
                std::ptr::null(),
                (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32,
            ) >= 0;
            let result = ShellExecuteW(
                std::ptr::null_mut(),
                std::ptr::null(),
                wide.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            ) as isize;
            if initialized {
                CoUninitialize();
            }
            result
        };
        if result > 32 {
            Ok(())
        } else {
            Err("Не удалось открыть браузер.".into())
        }
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map(|_| ())
            .map_err(|_| "Не удалось открыть браузер.".into())
    }
}
