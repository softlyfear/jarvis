// Starts the local voice server (tools/voice-server: Whisper + voice clone) in the
// background together with Jarvis, when it has been installed with setup.bat.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use crate::actions::platform;
use crate::assistant_config::{self, VoiceServerConfig};
use crate::{APP_CONFIG_DIR, APP_DIR};

pub const LOG_FILE_NAME: &str = "voice-server.log";

pub fn server_dir() -> PathBuf {
    APP_DIR.join("tools").join("voice-server")
}

// the private Python the installer puts next to the server; .venv for installs made before it
pub fn python_path(dir: &Path) -> PathBuf {
    let candidates = if cfg!(windows) {
        [dir.join("python").join("python.exe"), dir.join(".venv").join("Scripts").join("python.exe")]
    } else {
        [dir.join("python").join("bin").join("python3"), dir.join(".venv").join("bin").join("python")]
    };
    candidates.iter().find(|p| p.exists()).cloned().unwrap_or_else(|| candidates[0].clone())
}

pub fn is_running(cfg: &VoiceServerConfig) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(800))
        .build()
        .ok()
        .and_then(|c| c.get(&cfg.health_url).send().ok())
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

// returns what happened, for the log
pub fn start() -> String {
    start_with(&assistant_config::get().voice_server, &server_dir())
}

fn start_with(cfg: &VoiceServerConfig, dir: &Path) -> String {
    if !cfg.autostart {
        return "autostart is off".into();
    }
    let python = python_path(dir);
    if !python.exists() {
        return format!("not installed ({} not found; run setup.bat)", python.display());
    }
    if is_running(cfg) {
        return "already running".into();
    }

    let log = APP_CONFIG_DIR
        .get()
        .map(|d| d.join(LOG_FILE_NAME))
        .unwrap_or_else(|| dir.join(LOG_FILE_NAME));
    let (stdout, stderr) = match std::fs::File::create(&log).and_then(|f| Ok((f.try_clone()?, f))) {
        Ok((a, b)) => (Stdio::from(a), Stdio::from(b)),
        Err(_) => (Stdio::null(), Stdio::null()),
    };

    let mut cmd = platform::hidden_command(&python.to_string_lossy());
    cmd.current_dir(dir)
        .arg("-u")
        .arg("server.py")
        .args(&cfg.args)
        .stdout(stdout)
        .stderr(stderr)
        .stdin(Stdio::null());

    match cmd.spawn() {
        Ok(child) => format!("started (pid {}), log: {}", child.id(), log.display()),
        Err(e) => format!("failed to start: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_nothing_when_not_installed_or_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = VoiceServerConfig::default();
        assert!(start_with(&cfg, tmp.path()).starts_with("not installed"));

        let off = VoiceServerConfig { autostart: false, ..VoiceServerConfig::default() };
        assert_eq!(start_with(&off, tmp.path()), "autostart is off");
    }

    #[test]
    fn prefers_the_private_python_over_an_old_venv() {
        let tmp = tempfile::tempdir().unwrap();
        let (private, venv) = if cfg!(windows) {
            (tmp.path().join("python").join("python.exe"), tmp.path().join(".venv").join("Scripts").join("python.exe"))
        } else {
            (tmp.path().join("python").join("bin").join("python3"), tmp.path().join(".venv").join("bin").join("python"))
        };
        assert_eq!(python_path(tmp.path()), private); // nothing installed: where it will be
        std::fs::create_dir_all(venv.parent().unwrap()).unwrap();
        std::fs::write(&venv, b"").unwrap();
        assert_eq!(python_path(tmp.path()), venv);
        std::fs::create_dir_all(private.parent().unwrap()).unwrap();
        std::fs::write(&private, b"").unwrap();
        assert_eq!(python_path(tmp.path()), private);
    }

    #[test]
    fn health_check_fails_fast_without_server() {
        // a port nobody listens on
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let cfg = VoiceServerConfig { health_url: format!("http://127.0.0.1:{}/health", port), ..VoiceServerConfig::default() };
        assert!(!is_running(&cfg));
    }
}
