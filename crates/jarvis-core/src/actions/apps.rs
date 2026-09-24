// Opening and closing programs by spoken name.
// Lookup order for "open": folders from config -> app aliases from config ->
// Start Menu / Desktop shortcuts -> Steam games.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use super::text::{normalize, similarity};
use super::{platform, steam, ActionError};
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

// closed only gracefully: force-killing them may lose unsaved documents
const NO_FORCE_KILL: &[&str] = &[
    "winword", "excel", "powerpnt", "notepad", "mspaint", "code", "wordpad", "onenote",
    "photoshop", "blender", "obs64",
];

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
];

// open whatever the spoken name refers to; returns a human-readable name of what was opened
pub fn open(spoken: &str) -> Result<String, ActionError> {
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
    let builtin: std::collections::HashMap<String, String> =
        BUILTIN_APPS.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
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
    cmd.args(["/IM", &image, "/T"]);
    if force {
        cmd.arg("/F");
    }
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

fn still_running(process: &str) -> bool {
    running_processes().iter().any(|p| p == process)
}

pub fn close(spoken: &str) -> Result<String, ActionError> {
    let spoken_n = normalize(spoken);
    if spoken_n.is_empty() {
        return Err(ActionError::NotFound("не расслышал, что закрыть".into()));
    }
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }

    let targets = resolve_processes(&spoken_n);
    if targets.is_empty() {
        return Err(ActionError::NotFound(format!("не нашёл запущенную программу «{}»", spoken_n)));
    }

    for t in &targets {
        if is_protected(t) {
            continue;
        }
        info!("Closing process: {}", t);
        taskkill(t, false);
    }

    // apps that hide to tray (Steam, Discord) ignore the polite request
    std::thread::sleep(Duration::from_millis(2500));
    for t in &targets {
        if is_protected(t) || NO_FORCE_KILL.contains(&t.as_str()) {
            continue;
        }
        if still_running(t) {
            info!("Process {} still running, forcing", t);
            taskkill(t, true);
        }
    }

    Ok(targets.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protected_processes_are_never_targets() {
        assert!(is_protected("explorer"));
        assert!(is_protected("csrss"));
        assert!(!is_protected("discord"));
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
