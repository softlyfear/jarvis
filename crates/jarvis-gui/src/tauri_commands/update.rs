// Self-update: compare with version.json of the "latest" GitHub release, download
// JarvisSetup.exe and run it silently. The installer closes Jarvis, keeps settings and
// starts it again.

use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const VERSION_URL: &str = "https://github.com/softlyfear/jarvis/releases/download/latest/version.json";

#[derive(Deserialize, Debug, Clone)]
pub struct RemoteVersion {
    pub version: String,
    #[serde(default)]
    pub build: String,
    pub setup: String,
}

#[derive(Serialize, Debug)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub available: bool,
}

fn current_version() -> String {
    jarvis_core::config::APP_VERSION.unwrap_or("0.0.0").to_string()
}

// "0.2.14" > "0.2.9"; non-numeric parts count as 0
pub fn is_newer(remote: &str, local: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.trim().trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    let (r, l) = (parse(remote), parse(local));
    for i in 0..r.len().max(l.len()) {
        let (a, b) = (r.get(i).copied().unwrap_or(0), l.get(i).copied().unwrap_or(0));
        if a != b {
            return a > b;
        }
    }
    false
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .connect_timeout(Duration::from_secs(10))
        .user_agent("jarvis-updater")
        .build()
        .map_err(|e| e.to_string())
}

fn fetch_remote() -> Result<RemoteVersion, String> {
    let resp = client()?.get(VERSION_URL).send().map_err(|e| format!("нет связи с GitHub: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub ответил {}", resp.status()));
    }
    resp.json::<RemoteVersion>().map_err(|e| format!("не удалось прочитать version.json: {}", e))
}

#[tauri::command(async)]
pub fn check_update() -> Result<UpdateInfo, String> {
    let current = current_version();
    let remote = fetch_remote()?;
    let available = is_newer(&remote.version, &current);
    info!("Update check: current {}, latest {} (build {}), available: {}", current, remote.version, remote.build, available);
    Ok(UpdateInfo { current, latest: remote.version, available })
}

const DOWNLOAD_ATTEMPTS: u32 = 3;

// The download runs in the background, so leaving the settings page or pressing the button
// again neither stops it nor starts a second one; the window polls update_status.
#[derive(Serialize, Debug, Clone, Default, PartialEq)]
pub struct UpdateStatus {
    // "idle" | "downloading" | "starting" | "failed"
    pub phase: String,
    pub version: String,
    pub done: u64,
    // 0 while unknown
    pub total: u64,
    pub error: String,
}

static STATUS: Mutex<Option<UpdateStatus>> = Mutex::new(None);

fn set_status(f: impl FnOnce(&mut UpdateStatus)) {
    let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    f(s.get_or_insert_with(|| UpdateStatus { phase: "idle".into(), ..Default::default() }));
}

#[tauri::command]
pub fn update_status() -> UpdateStatus {
    STATUS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or(UpdateStatus { phase: "idle".into(), ..Default::default() })
}

// true when this call owns the download; false when one is already running
fn begin_download() -> bool {
    let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
    if s.as_ref().is_some_and(|s| s.phase == "downloading" || s.phase == "starting") {
        return false;
    }
    *s = Some(UpdateStatus { phase: "downloading".into(), ..Default::default() });
    true
}

// streams the installer to disk (no 100 MB buffer in memory), reporting progress; returns its size
fn download(url: &str, path: &std::path::Path) -> Result<u64, String> {
    let mut resp = client()?.get(url).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let expected = resp.content_length();
    set_status(|s| {
        s.done = 0;
        s.total = expected.unwrap_or(0);
    });
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut size: u64 = 0;
    loop {
        let n = resp.read(&mut buf).map_err(|e| format!("обрыв связи: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        size += n as u64;
        set_status(|s| s.done = size);
    }
    if expected.is_some_and(|n| n != size) {
        return Err(format!("скачано {} из {} байт", size, expected.unwrap_or(0)));
    }
    if size < 1_000_000 {
        return Err("скачанный установщик слишком мал".into());
    }
    Ok(size)
}

// starts the download in the background and returns at once
#[tauri::command(async)]
pub fn install_update() -> Result<(), String> {
    if !begin_download() {
        info!("Update is already downloading");
        return Ok(());
    }
    std::thread::spawn(|| {
        if let Err(e) = download_and_run() {
            warn!("Update failed: {}", e);
            set_status(|s| {
                s.phase = "failed".into();
                s.error = e;
            });
        }
    });
    Ok(())
}

fn download_and_run() -> Result<(), String> {
    let remote = fetch_remote()?;
    set_status(|s| s.version = remote.version.clone());
    if !remote.setup.starts_with("https://github.com/softlyfear/jarvis/") {
        return Err("неожиданный адрес установщика".into());
    }
    let path = std::env::temp_dir().join(format!("JarvisSetup-{}.exe", remote.version));
    info!("Downloading update {} to {}", remote.version, path.display());
    // a dropped connection midway ("error decoding response body") is common on slow links
    let mut last_error = String::new();
    for attempt in 1..=DOWNLOAD_ATTEMPTS {
        match download(&remote.setup, &path) {
            Ok(size) => {
                info!("Downloaded {} bytes (attempt {})", size, attempt);
                last_error.clear();
                break;
            }
            Err(e) => {
                warn!("Update download attempt {} failed: {}", attempt, e);
                last_error = e;
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    if !last_error.is_empty() {
        return Err(format!("не удалось скачать установщик ({} попытки): {}. Проверьте интернет и нажмите ещё раз", DOWNLOAD_ATTEMPTS, last_error));
    }

    info!("Starting silent update");
    set_status(|s| s.phase = "starting".into());
    std::process::Command::new(&path)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/SP-"])
        .spawn()
        .map_err(|e| format!("не удалось запустить установщик: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(is_newer("0.2.14", "0.2.9"));
        assert!(is_newer("0.3.0", "0.2.99"));
        assert!(!is_newer("0.2.9", "0.2.9"));
        assert!(!is_newer("0.2.8", "0.2.9"));
        assert!(is_newer("0.2.1", "0.1.0"));
        assert!(is_newer("v1.0", "0.9.9"));
    }

    #[test]
    fn only_one_download_at_a_time() {
        assert_eq!(update_status().phase, "idle");
        assert!(begin_download());
        assert!(!begin_download()); // a second click or a return to the page
        set_status(|s| {
            s.phase = "failed".into();
            s.error = "обрыв".into();
        });
        assert_eq!(update_status().error, "обрыв");
        assert!(begin_download()); // after a failure the button tries again
        assert_eq!(update_status(), UpdateStatus { phase: "downloading".into(), ..Default::default() });
    }

    #[test]
    fn version_json_parses() {
        let v: RemoteVersion = serde_json::from_str(r#"{"version":"0.2.14","build":"abc","setup":"https://github.com/softlyfear/jarvis/releases/download/latest/JarvisSetup.exe"}"#).unwrap();
        assert_eq!(v.version, "0.2.14");
    }
}
