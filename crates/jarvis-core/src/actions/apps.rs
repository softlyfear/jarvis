// Opening and closing programs by spoken name.
// Lookup order for "open": folders from config -> app aliases from config ->
// Start Menu / Desktop shortcuts -> Steam games.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use super::text::{normalize, similarity};
use super::{dialog, files, input, platform, steam, ActionError, ActionOutcome};
use crate::assistant_config::{self, expand_env};

const ALIAS_MIN_SCORE: f64 = 82.0;
const SHORTCUT_MIN_SCORE: f64 = 78.0;
const PROCESS_MIN_SCORE: f64 = 80.0;

// never closed by voice, whatever the name match says
const PROTECTED_PROCESSES: &[&str] = &[
    "system", "idle", "registry", "smss", "csrss", "wininit", "winlogon", "services", "lsass",
    "svchost", "dwm", "explorer", "fontdrvhost", "sihost", "ctfmon", "conhost", "runtimebroker",
    "searchhost", "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "spoolsv",
    "audiodg", "securityhealthservice", "msmpeng", "taskhostw", "dllhost", "wudfhost",
    "jarvis-app", "jarvis-gui", "jarvis",
];

// Editors may retain background processes after all document windows have closed.
const NO_FORCE_KILL: &[&str] = &[
    "winword", "excel", "powerpnt", "notepad", "mspaint", "code", "wordpad", "onenote",
    "photoshop", "blender", "obs64",
];
// Only known tray applications may be terminated after ignoring WM_CLOSE.
const TRAY_APPS: &[&str] = &["steam", "discord", "telegram", "spotify", "epicgameslauncher", "battle.net"];

const PROCESS_ALIASES: &[(&str, &str)] = &[
    ("visual studio code", "code"), ("визуал студио код", "code"),
    ("визуал студия код", "code"), ("вирус студия кода", "code"),
    ("vs code", "code"), ("вс код", "code"), ("блокнот", "notepad"),
    ("notepad plus plus", "notepad++"), ("телеграм", "telegram"),
    ("дискорд", "discord"), ("стим", "steam"),
    ("google chrome", "chrome"), ("гугл хром", "chrome"), ("хром", "chrome"),
];

fn builtin_process(spoken: &str, running: &[String]) -> Option<String> {
    let (_, name) = PROCESS_ALIASES.iter().find(|(alias, name)| normalize(alias) == spoken && running.iter().any(|p| p == name))?;
    Some(name.to_string())
}

#[derive(Debug, Clone)]
pub struct Shortcut {
    pub name: String,
    pub path: PathBuf,
}

static SHORTCUTS: Lazy<Mutex<Option<(Instant, Vec<Shortcut>)>>> = Lazy::new(|| Mutex::new(None));
const SHORTCUTS_TTL: Duration = Duration::from_secs(600);

fn shortcut_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for base in ["%ProgramData%", "%APPDATA%"] {
        let expanded = expand_env(base);
        if expanded != base {
            dirs.push(PathBuf::from(expanded).join("Microsoft\\Windows\\Start Menu\\Programs"));
        }
    }
    for base in ["%USERPROFILE%\\Desktop", "%PUBLIC%\\Desktop", "%USERPROFILE%\\OneDrive\\Desktop"] {
        let expanded = expand_env(base);
        if expanded != base {
            dirs.push(PathBuf::from(expanded));
        }
    }
    dirs
}

fn is_launchable(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("lnk") | Some("url") | Some("exe") | Some("appref-ms")
    )
}

fn is_junk_shortcut(name: &str) -> bool {
    let n = name.to_lowercase();
    ["uninstall", "удал", "readme", "help", "справка", "license", "website", "documentation", "manual"]
        .iter()
        .any(|w| n.contains(w))
}

fn scan_dir(dir: &Path, depth: usize, out: &mut Vec<Shortcut>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                scan_dir(&path, depth - 1, out);
            }
        } else if is_launchable(&path) {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if !is_junk_shortcut(stem) {
                    out.push(Shortcut { name: stem.to_string(), path });
                }
            }
        }
    }
}

pub fn shortcuts() -> Vec<Shortcut> {
    let mut cache = SHORTCUTS.lock();
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < SHORTCUTS_TTL {
            return list.clone();
        }
    }
    let mut list = Vec::new();
    for dir in shortcut_dirs() {
        scan_dir(&dir, 3, &mut list);
    }
    info!("Indexed {} shortcuts", list.len());
    *cache = Some((Instant::now(), list.clone()));
    list
}

// best (score, item) by similarity of the spoken name to `name_of(item)`
pub fn best_match<'a, T>(spoken: &str, items: &'a [T], name_of: impl Fn(&T) -> &str) -> Option<(f64, &'a T)> {
    items
        .iter()
        .map(|it| (similarity(spoken, name_of(it)), it))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
}

fn config_alias<'a>(spoken: &str, map: &'a std::collections::HashMap<String, String>) -> Option<(f64, &'a String, &'a String)> {
    map.iter()
        .map(|(k, v)| (similarity(spoken, k), k, v))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
}

const BUILTIN_APPS: &[(&str, &str)] = &[
    ("мой компьютер", "explorer.exe"),
    ("этот компьютер", "explorer.exe"),
    ("компьютер", "explorer.exe"),
    ("мои файлы", "explorer.exe"),
    ("файлы", "explorer.exe"),
    ("проводник", "explorer.exe"),
    ("корзина", "shell:RecycleBinFolder"),
    // Windows settings pages
    ("настройки звука", "ms-settings:sound"),
    ("звук", "ms-settings:sound"),
    ("настройки экрана", "ms-settings:display"),
    ("экран", "ms-settings:display"),
    ("ночной свет", "ms-settings:nightlight"),
    ("блютуз", "ms-settings:bluetooth"),
    ("bluetooth", "ms-settings:bluetooth"),
    ("настройки блютуз", "ms-settings:bluetooth"),
    ("вай фай", "ms-settings:network-wifi"),
    ("wi fi", "ms-settings:network-wifi"),
    ("настройки интернета", "ms-settings:network-status"),
    ("настройки сети", "ms-settings:network-status"),
    ("обновления", "ms-settings:windowsupdate"),
    ("обновление windows", "ms-settings:windowsupdate"),
    ("установленные приложения", "ms-settings:appsfeatures"),
    ("удаление программ", "ms-settings:appsfeatures"),
    ("настройки мыши", "ms-settings:mousetouchpad"),
    ("принтеры", "ms-settings:printers"),
    ("память", "ms-settings:storagesense"),
    ("хранилище", "ms-settings:storagesense"),
    ("обои", "ms-settings:personalization-background"),
    ("персонализация", "ms-settings:personalization"),
    ("темы", "ms-settings:themes"),
    ("уведомления", "ms-settings:notifications"),
    ("не беспокоить", "ms-settings:notifications"),
    ("настройки микрофона", "ms-settings:sound"),
    ("язык", "ms-settings:regionlanguage"),
    ("клавиатура", "ms-settings:typing"),
    ("игровой режим", "ms-settings:gaming-gamemode"),
    ("батарея", "ms-settings:batterysaver"),
    ("электропитание", "ms-settings:powersleep"),
];

// popular sites, asked only after programs and games: "яндекс" must stay the Yandex Browser
// when it is installed
const BUILTIN_SITES: &[(&str, &str)] = &[
    ("ютуб", "https://www.youtube.com"),
    ("youtube", "https://www.youtube.com"),
    ("вконтакте", "https://vk.com"),
    ("вк", "https://vk.com"),
    ("твич", "https://www.twitch.tv"),
    ("яндекс", "https://ya.ru"),
    ("гугл", "https://www.google.com"),
    ("почта", "https://mail.yandex.ru"),
    ("яндекс почта", "https://mail.yandex.ru"),
    ("гмейл", "https://mail.google.com"),
    ("яндекс музыка", "https://music.yandex.ru"),
    ("кинопоиск", "https://www.kinopoisk.ru"),
    ("википедия", "https://ru.wikipedia.org"),
    ("карты", "https://yandex.ru/maps"),
    ("переводчик", "https://translate.yandex.ru"),
    ("погода", "https://yandex.ru/pogoda"),
    ("яндекс диск", "https://disk.yandex.ru"),
    ("госуслуги", "https://www.gosuslugi.ru"),
    ("озон", "https://www.ozon.ru"),
    ("вайлдберриз", "https://www.wildberries.ru"),
    ("авито", "https://www.avito.ru"),
    ("дзен", "https://dzen.ru"),
    ("пинтерест", "https://www.pinterest.com"),
];

fn builtin_map(list: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
    list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

// open whatever the spoken name refers to; returns a human-readable name of what was opened
pub fn open(spoken: &str) -> Result<String, ActionError> {
    // Older agent histories may still send paths to open_app. Validate before normalization.
    let path = PathBuf::from(expand_env(spoken.trim()));
    if path.is_absolute() {
        return files::open_file(spoken.trim()).map(|p| p.display().to_string());
    }
    let spoken = normalize(spoken);
    if spoken.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что открыть".into()));
    }
    let cfg = assistant_config::get();

    if let Some((score, name, path)) = config_alias(&spoken, &cfg.folders) {
        if score >= ALIAS_MIN_SCORE {
            let path = expand_env(path);
            platform::open_target(&path).map_err(ActionError::Failed)?;
            return Ok(name.clone());
        }
    }

    if let Some((score, name, target)) = config_alias(&spoken, &cfg.apps) {
        if score >= ALIAS_MIN_SCORE {
            platform::open_target(&expand_env(target)).map_err(ActionError::Failed)?;
            return Ok(name.clone());
        }
    }

    // Windows places people call by name, whatever the config file says
    let builtin = builtin_map(BUILTIN_APPS);
    if let Some((score, name, target)) = config_alias(&spoken, &builtin) {
        if score >= ALIAS_MIN_SCORE {
            platform::open_target(target).map_err(ActionError::Failed)?;
            return Ok(name.clone());
        }
    }

    let list = shortcuts();
    let shortcut = best_match(&spoken, &list, |s| s.name.as_str());
    let game = steam::find_game(&spoken);

    // prefer whichever matches better; a Steam game wins ties (URI launch is more reliable)
    let shortcut = shortcut.filter(|(score, _)| *score >= SHORTCUT_MIN_SCORE);
    let game = game.filter(|(score, _)| *score >= SHORTCUT_MIN_SCORE);
    let use_game = match (&shortcut, &game) {
        (Some((s_score, _)), Some((g_score, _))) => g_score >= s_score,
        (None, Some(_)) => true,
        _ => false,
    };

    if use_game {
        let (_, g) = game.expect("checked above");
        steam::launch(&g).map_err(ActionError::Failed)?;
        return Ok(g.name);
    }
    if let Some((_, s)) = shortcut {
        platform::open_target(&s.path.to_string_lossy()).map_err(ActionError::Failed)?;
        return Ok(s.name.clone());
    }
    let sites = builtin_map(BUILTIN_SITES);
    if let Some((score, name, url)) = config_alias(&spoken, &sites) {
        if score >= ALIAS_MIN_SCORE {
            platform::open_target(url).map_err(ActionError::Failed)?;
            return Ok(name.clone());
        }
    }
    Err(ActionError::NotFound(format!("не нашёл программу «{}»", spoken)))
}

// Steam first, then shortcuts (non-Steam games are usually on the desktop)
pub fn launch_game(spoken: &str) -> Result<String, ActionError> {
    let spoken = normalize(spoken);
    if spoken.is_empty() {
        return Err(ActionError::NotFound("не расслышал название игры".into()));
    }
    if let Some((score, game)) = steam::find_game(&spoken) {
        if score >= SHORTCUT_MIN_SCORE {
            steam::launch(&game).map_err(ActionError::Failed)?;
            return Ok(game.name);
        }
    }
    open(&spoken)
}

fn running_processes() -> Vec<String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut names: HashSet<String> = HashSet::new();
    for p in sys.processes().values() {
        let name = p.name().to_string_lossy().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name).to_string();
        names.insert(name);
    }
    names.into_iter().collect()
}

fn is_protected(process: &str) -> bool {
    PROTECTED_PROCESSES.contains(&process)
}

// which running processes the spoken name refers to
pub fn resolve_processes(spoken: &str) -> Vec<String> {
    let spoken = normalize(spoken);
    let running = running_processes();
    let cfg = assistant_config::get();

    let mut targets: Vec<String> = Vec::new();

    // explicit mapping from config
    let best_cfg = cfg
        .processes
        .iter()
        .map(|(k, v)| (similarity(&spoken, k), v))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((score, names)) = best_cfg {
        if score >= ALIAS_MIN_SCORE {
            for n in names {
                let n = n.to_lowercase();
                if running.contains(&n) {
                    targets.push(n);
                }
            }
            if !targets.is_empty() {
                return targets;
            }
        }
    }

    // fuzzy match against running process names
    if let Some(name) = builtin_process(&spoken, &running) { return vec![name]; }
    if let Some((score, name)) = best_match(&spoken, &running, |s| s.as_str()) {
        if score >= PROCESS_MIN_SCORE && !is_protected(name) {
            targets.push(name.clone());
        }
    }
    targets
}

fn taskkill(process: &str, force: bool) -> bool {
    let image = format!("{}.exe", process);
    let mut cmd = platform::hidden_command("taskkill");
    cmd.args(["/IM", &image]);
    if force {
        cmd.arg("/F");
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

// "закрой проводник": explorer.exe is also the taskbar, so its windows are closed, not the process
pub fn is_file_explorer(spoken: &str) -> bool {
    ["проводник", "explorer", "окна проводника", "папки", "окна папок"]
        .iter()
        .any(|n| similarity(spoken, n) >= ALIAS_MIN_SCORE)
}

fn close_explorer_windows() -> Result<String, ActionError> {
    let script = "$n = 0; foreach ($w in (New-Object -ComObject Shell.Application).Windows()) { \
        if ($w.FullName -like '*\\explorer.exe') { $w.Quit(); $n++ } }; $n";
    let closed = platform::powershell(script, &[]).map_err(ActionError::Failed)?;
    match closed.trim().parse::<u32>() {
        Ok(0) | Err(_) => Err(ActionError::NotFound("открытых окон проводника нет".into())),
        Ok(_) => Ok("окна проводника".into()),
    }
}

pub fn close(spoken: &str) -> Result<ActionOutcome, ActionError> {
    let spoken_n = normalize(spoken);
    if spoken_n.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что закрыть".into()));
    }
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    if is_file_explorer(&spoken_n) {
        return close_explorer_windows().map(|n| ActionOutcome::done(format!("закрыто: {}", n)));
    }

    let targets: Vec<_> = resolve_processes(&spoken_n).into_iter().filter(|t| !is_protected(t)).collect();
    if targets.is_empty() {
        return Err(ActionError::NotFound(format!("не нашёл запущенную программу «{}»", spoken_n)));
    }

    let windows: Vec<_> = input::windows_on_screen().into_iter().filter(|w| targets.contains(&w.process)).collect();
    for window in &windows { request_close(window.handle)?; }
    for t in &targets {
        if TRAY_APPS.contains(&t.as_str()) && !windows.iter().any(|w| &w.process == t) { taskkill(t, false); }
    }
    let start = Instant::now();
    let mut inspected = false;
    loop {
        let running = running_processes();
        let remaining = input::windows_on_screen();
        if closed(&targets, &running, &windows, &remaining) {
            return Ok(ActionOutcome::done(format!("закрыто: {}", spoken_n)));
        }
        // Inspect only after the editor had a chance to present its modal dialog.
        if !inspected && start.elapsed() >= Duration::from_millis(200) && targets.iter().any(|t| !TRAY_APPS.contains(&t.as_str())) {
            inspected = true;
            if let Some(question) = dialog::save_prompt_for(&targets) { return Ok(question); }
        }
        if start.elapsed() >= Duration::from_millis(1200) { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    for t in &targets {
        if TRAY_APPS.contains(&t.as_str()) && running_processes().contains(t) {
            info!("Process {} still running, forcing", t);
            taskkill(t, true);
        }
    }

    // taskkill returning does not imply that Windows has finished removing the process.
    if wait_for_close(|| closed(&targets, &running_processes(), &windows, &input::windows_on_screen()), Duration::from_millis(1200)) {
        Ok(ActionOutcome::done(format!("закрыто: {}", spoken_n)))
    } else {
        Err(ActionError::Failed("программа ещё открыта: возможно, ждёт сохранения или подтверждения. Завершение не подтверждено".into()))
    }
}

fn wait_for_close(mut check: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if check() { return true; }
        if start.elapsed() >= timeout { return false; }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn closed(targets: &[String], running: &[String], original: &[input::WindowInfo], remaining: &[input::WindowInfo]) -> bool {
    targets.iter().all(|t| {
        !running.contains(t) || (NO_FORCE_KILL.contains(&t.as_str())
            && original.iter().any(|w| &w.process == t)
            && !remaining.iter().any(|w| &w.process == t))
    })
}

pub fn request_close(handle: isize) -> Result<(), ActionError> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        if unsafe { PostMessageW(handle as windows_sys::Win32::Foundation::HWND, WM_CLOSE, 0, 0) } != 0 { return Ok(()); }
        Err(ActionError::Failed("окно не приняло запрос закрытия".into()))
    }
    #[cfg(not(windows))]
    { let _ = handle; Err(ActionError::Unsupported) }
}

pub fn close_window(window: input::WindowInfo) -> Result<ActionOutcome, ActionError> {
    request_close(window.handle)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(800) {
        if input::windows_on_screen().iter().all(|w| w.handle != window.handle) {
            return Ok(ActionOutcome::done("окно закрыто"));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(question) = dialog::save_prompt_for(&[window.process]) { return Ok(question); }
    Err(ActionError::Failed("окно ещё открыто; возможно, ожидает подтверждения".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_dialog_is_not_a_successful_close() {
        let targets = vec!["notepad".into()];
        let window = input::WindowInfo { handle: 1, process: "notepad".into(), title: "Блокнот".into(), minimized: false };
        assert!(!closed(&targets, &targets, &[window.clone()], &[window.clone()]));
        assert!(closed(&targets, &targets, &[window], &[]));
        assert!(!closed(&targets, &targets, &[], &[]));
        assert!(closed(&targets, &[], &[], &[]));
        assert!(!TRAY_APPS.contains(&"notepad"));
        assert!(!TRAY_APPS.contains(&"code"));
    }

    #[test]
    fn process_exit_is_observed_after_a_delay() {
        let mut polls = 0;
        assert!(wait_for_close(|| { polls += 1; polls >= 2 }, Duration::from_millis(200)));
        assert_eq!(polls, 2);
        assert!(!wait_for_close(|| false, Duration::ZERO));
        let targets = vec!["telegram".into()];
        assert!(!closed(&targets, &targets, &[], &[]));
        assert!(closed(&targets, &[], &[], &[]));
    }

    #[test]
    fn vscode_aliases_work_without_a_new_user_config() {
        let running = vec!["code".into(), "notepad".into()];
        assert_eq!(builtin_process("visual studio code", &running), Some("code".into()));
        assert_eq!(builtin_process("визуал студия код", &running), Some("code".into()));
        assert_eq!(builtin_process("visual studio", &running), None);
        assert_eq!(builtin_process("visual studio code", &[]), None);
        let running = vec!["chrome".into()];
        assert_eq!(builtin_process("google chrome", &running), Some("chrome".into()));
        assert_eq!(builtin_process("гугл хром", &running), Some("chrome".into()));
        assert_eq!(builtin_process("google chrome", &[]), None);
    }

    #[test]
    fn protected_processes_are_never_targets() {
        assert!(is_protected("explorer"));
        assert!(is_protected("csrss"));
        assert!(!is_protected("discord"));
        assert!(TRAY_APPS.iter().all(|p| !is_protected(p) && !NO_FORCE_KILL.contains(p)));
    }

    #[test]
    fn file_explorer_is_recognized_and_builtins_resolve() {
        assert!(is_file_explorer("проводник"));
        assert!(is_file_explorer("проводника"));
        assert!(is_file_explorer("explorer"));
        assert!(!is_file_explorer("телеграм"));
        assert!(!is_file_explorer("стим"));
        let apps = builtin_map(BUILTIN_APPS);
        let (score, name, target) = config_alias("блютус", &apps).unwrap();
        assert!(score >= ALIAS_MIN_SCORE, "{} {}", name, score);
        assert_eq!(target, "ms-settings:bluetooth");
        let sites = builtin_map(BUILTIN_SITES);
        assert_eq!(config_alias("ютуб", &sites).unwrap().2, "https://www.youtube.com");
        assert!(BUILTIN_APPS.iter().all(|(_, t)| t.contains(':') || t.ends_with(".exe")));
        assert!(BUILTIN_SITES.iter().all(|(_, u)| u.starts_with("https://")));
    }

    #[test]
    fn junk_shortcuts_are_skipped() {
        assert!(is_junk_shortcut("Uninstall Discord"));
        assert!(is_junk_shortcut("Удалить Steam"));
        assert!(!is_junk_shortcut("Discord"));
    }

    #[test]
    fn best_match_picks_closest() {
        let items = vec!["Steam".to_string(), "Discord".to_string(), "Telegram".to_string()];
        let (score, name) = best_match("дискорд", &items, |s| s.as_str()).unwrap();
        assert_eq!(name, "Discord");
        assert!(score >= 80.0);
    }
}
