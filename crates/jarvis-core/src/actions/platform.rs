// Thin OS layer: opening targets, spawning helpers, key presses, notifications.
// Real behaviour is Windows-only; other platforms get best-effort fallbacks so the
// crate still builds and unit tests run on Linux CI.

use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

// CREATE_NO_WINDOW: do not flash a console window for helper processes
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

// open a file, folder, program, URL or URI with the default handler
pub fn open_target(target: &str) -> Result<(), String> {
    info!("Opening: {}", target);

    let lower = target.to_lowercase();
    let is_web = lower.starts_with("http://") || lower.starts_with("https://");

    let result = if cfg!(windows) && is_web {
        // no cmd.exe parsing for URLs (% and & are common in query strings)
        hidden_command("rundll32.exe").args(["url.dll,FileProtocolHandler", target]).spawn()
    } else if cfg!(windows) {
        // `start "" "<target>"` handles .lnk, .url, URIs (steam://, ms-settings:) and plain exe names
        hidden_command("cmd").args(["/C", "start", "", target]).spawn()
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(target).spawn()
    } else {
        Command::new("xdg-open").arg(target).spawn()
    };

    result.map(|_| ()).map_err(|e| format!("cannot open {}: {}", target, e))
}

// run a PowerShell script; the payload is passed through an environment variable,
// never interpolated into the script text
pub fn powershell(script: &str, env: &[(&str, &str)]) -> Result<String, String> {
    let mut cmd = hidden_command("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().map_err(|e| format!("powershell: {}", e))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(format!(
            "powershell exited with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

pub mod vk {
    pub const VOLUME_MUTE: u8 = 0xAD;
    pub const VOLUME_DOWN: u8 = 0xAE;
    pub const VOLUME_UP: u8 = 0xAF;
    pub const MEDIA_NEXT: u8 = 0xB0;
    pub const MEDIA_PREV: u8 = 0xB1;
    pub const MEDIA_STOP: u8 = 0xB2;
    pub const MEDIA_PLAY_PAUSE: u8 = 0xB3;
    pub const LWIN: u8 = 0x5B;
    pub const SHIFT: u8 = 0x10;
    pub const SNAPSHOT: u8 = 0x2C;
    pub const D: u8 = 0x44;
    pub const S: u8 = 0x53;
}

// press and release a key `times` times
pub fn press_key(key: u8, times: u32) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
        for _ in 0..times {
            unsafe {
                keybd_event(key, 0, 0, 0);
                keybd_event(key, 0, KEYEVENTF_KEYUP, 0);
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (key, times);
        Err("key presses are supported on Windows only".into())
    }
}

// press a key combination, e.g. [LWIN, D]
pub fn press_combo(keys: &[u8]) -> Result<(), String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
        unsafe {
            for k in keys {
                keybd_event(*k, 0, 0, 0);
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            for k in keys.iter().rev() {
                keybd_event(*k, 0, KEYEVENTF_KEYUP, 0);
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = keys;
        Err("key presses are supported on Windows only".into())
    }
}

pub fn notify(title: &str, message: &str) {
    info!("NOTIFY: {} - {}", title, message);

    #[cfg(windows)]
    {
        use winrt_notification::{Duration as ToastDuration, Toast};
        if let Err(e) = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(message)
            .duration(ToastDuration::Short)
            .show()
        {
            warn!("Toast notification failed: {}", e);
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send").args([title, message]).spawn();
    }
}
