// Self-update: compare with version.json of the "latest" GitHub release, download
// JarvisSetup.exe and run it silently. The installer closes Jarvis, keeps settings and
// starts it again.

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

// streams the installer to disk (no 100 MB buffer in memory); returns its size
fn download(url: &str, path: &std::path::Path) -> Result<u64, String> {
    let mut resp = client()?.get(url).send().and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let expected = resp.content_length();
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let size = resp.copy_to(&mut file).map_err(|e| format!("обрыв связи: {}", e))?;
    if expected.is_some_and(|n| n != size) {
        return Err(format!("скачано {} из {} байт", size, expected.unwrap_or(0)));
    }
    if size < 1_000_000 {
        return Err("скачанный установщик слишком мал".into());
    }
    Ok(size)
}

#[tauri::command(async)]
pub fn install_update() -> Result<(), String> {
    let remote = fetch_remote()?;
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
    fn version_json_parses() {
        let v: RemoteVersion = serde_json::from_str(r#"{"version":"0.2.14","build":"abc","setup":"https://github.com/softlyfear/jarvis/releases/download/latest/JarvisSetup.exe"}"#).unwrap();
        assert_eq!(v.version, "0.2.14");
    }
}
