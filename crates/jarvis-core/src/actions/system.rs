// Volume, media keys, power, screenshots, web.

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

use super::platform::{self, vk};
use super::ActionError;

// one media key press changes the master volume by 2%
const VOLUME_STEP: u32 = 2;

fn keys(r: Result<(), String>) -> Result<(), ActionError> {
    r.map_err(|e| if cfg!(windows) { ActionError::Failed(e) } else { ActionError::Unsupported })
}

pub fn volume_up(percent: u32) -> Result<(), ActionError> {
    keys(platform::press_key(vk::VOLUME_UP, (percent / VOLUME_STEP).max(1)))
}

pub fn volume_down(percent: u32) -> Result<(), ActionError> {
    keys(platform::press_key(vk::VOLUME_DOWN, (percent / VOLUME_STEP).max(1)))
}

// absolute level: slide to zero, then up; no audio COM API needed
pub fn set_volume(level: u32) -> Result<(), ActionError> {
    let level = level.min(100);
    keys(platform::press_key(vk::VOLUME_DOWN, 100 / VOLUME_STEP))?;
    if level > 0 {
        keys(platform::press_key(vk::VOLUME_UP, level / VOLUME_STEP))?;
    }
    Ok(())
}

pub fn toggle_mute() -> Result<(), ActionError> {
    keys(platform::press_key(vk::VOLUME_MUTE, 1))
}

pub fn media(action: &str) -> Result<(), ActionError> {
    let key = match action {
        "play_pause" | "play" | "pause" => vk::MEDIA_PLAY_PAUSE,
        "next" => vk::MEDIA_NEXT,
        "previous" | "prev" => vk::MEDIA_PREV,
        "stop" => vk::MEDIA_STOP,
        other => return Err(ActionError::Failed(format!("unknown media action: {}", other))),
    };
    keys(platform::press_key(key, 1))
}

// Win+PrintScreen saves to Pictures\Screenshots
pub fn screenshot() -> Result<(), ActionError> {
    keys(platform::press_combo(&[vk::LWIN, vk::SNAPSHOT]))
}

// Win+Shift+S: area snip to clipboard
pub fn snip() -> Result<(), ActionError> {
    keys(platform::press_combo(&[vk::LWIN, vk::SHIFT, vk::S]))
}

pub fn show_desktop() -> Result<(), ActionError> {
    keys(platform::press_combo(&[vk::LWIN, vk::D]))
}

fn run(program: &str, args: &[&str]) -> Result<(), ActionError> {
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    platform::hidden_command(program)
        .args(args)
        .spawn()
        .map(|_| ())
        .map_err(|e| ActionError::Failed(format!("{}: {}", program, e)))
}

pub fn lock() -> Result<(), ActionError> {
    run("rundll32.exe", &["user32.dll,LockWorkStation"])
}

pub fn sleep() -> Result<(), ActionError> {
    run("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"])
}

pub fn shutdown() -> Result<(), ActionError> {
    run("shutdown", &["/s", "/t", "10"])
}

pub fn restart() -> Result<(), ActionError> {
    run("shutdown", &["/r", "/t", "10"])
}

pub fn cancel_shutdown() -> Result<(), ActionError> {
    run("shutdown", &["/a"])
}

pub fn empty_recycle_bin() -> Result<(), ActionError> {
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    platform::powershell("Clear-RecycleBin -Force -ErrorAction SilentlyContinue", &[])
        .map(|_| ())
        .map_err(ActionError::Failed)
}

pub fn search_url(query: &str) -> String {
    format!(
        "https://yandex.ru/search/?text={}",
        utf8_percent_encode(query.trim(), NON_ALPHANUMERIC)
    )
}

pub fn web_search(query: &str) -> Result<(), ActionError> {
    if query.trim().is_empty() {
        return Err(ActionError::NotFound("не расслышал, что искать".into()));
    }
    platform::open_target(&search_url(query)).map_err(ActionError::Failed)
}

// only http(s) links: the model must not open arbitrary URIs or local files this way
pub fn open_url(url: &str) -> Result<(), ActionError> {
    let url = url.trim();
    let lower = url.to_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err(ActionError::Denied("открываю только ссылки http/https".into()));
    }
    if url.chars().any(|c| c.is_whitespace() || c == '"') {
        return Err(ActionError::Denied("некорректная ссылка".into()));
    }
    platform::open_target(url).map_err(ActionError::Failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_is_encoded() {
        assert_eq!(search_url("погода"), "https://yandex.ru/search/?text=%D0%BF%D0%BE%D0%B3%D0%BE%D0%B4%D0%B0");
        assert!(search_url("a b&c").ends_with("a%20b%26c"));
    }

    #[test]
    fn open_url_rejects_non_http() {
        assert!(matches!(open_url("file:///C:/Windows"), Err(ActionError::Denied(_))));
        assert!(matches!(open_url("calc.exe"), Err(ActionError::Denied(_))));
        assert!(matches!(open_url("https://x.ru/a b"), Err(ActionError::Denied(_))));
    }
}
