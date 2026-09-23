// Installed Steam games: found via libraryfolders.vdf + appmanifest_*.acf,
// launched with steam://rungameid/<appid>.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use super::platform;
use super::text::similarity;
use crate::assistant_config;

#[derive(Debug, Clone, PartialEq)]
pub struct SteamGame {
    pub appid: String,
    pub name: String,
}

static GAMES: Lazy<Mutex<Option<(Instant, Vec<SteamGame>)>>> = Lazy::new(|| Mutex::new(None));
const GAMES_TTL: Duration = Duration::from_secs(600);

// not games: runtimes and tools Steam installs as apps
const NOT_GAMES: &[&str] = &[
    "steamworks common redistributables", "proton", "steam linux runtime", "steamvr",
    "dedicated server", "sdk", "soundtrack",
];

fn steam_root() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let out = platform::hidden_command("reg")
            .args(["query", "HKCU\\Software\\Valve\\Steam", "/v", "SteamPath"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(idx) = line.find("REG_SZ") {
                let path = line[idx + "REG_SZ".len()..].trim().replace('/', "\\");
                if !path.is_empty() {
                    return Some(PathBuf::from(path));
                }
            }
        }
        let fallback = PathBuf::from("C:\\Program Files (x86)\\Steam");
        if fallback.exists() {
            return Some(fallback);
        }
        None
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var("HOME").ok()?;
        let p = PathBuf::from(home).join(".steam/steam");
        p.exists().then_some(p)
    }
}

// value of the first `"key"  "value"` pair on a line, VDF escapes resolved
fn vdf_value<'a>(line: &'a str, key: &str) -> Option<String> {
    let line = line.trim();
    let quoted_key = format!("\"{}\"", key);
    if !line.to_lowercase().starts_with(&quoted_key.to_lowercase()) {
        return None;
    }
    let rest = line[quoted_key.len()..].trim();
    let rest = rest.strip_prefix('"')?;
    let end = rest.rfind('"')?;
    Some(rest[..end].replace("\\\\", "\\"))
}

pub fn parse_library_folders(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|l| vdf_value(l, "path"))
        .map(PathBuf::from)
        .collect()
}

pub fn parse_manifest(acf: &str) -> Option<SteamGame> {
    let mut appid = None;
    let mut name = None;
    for line in acf.lines() {
        if appid.is_none() {
            appid = vdf_value(line, "appid");
        }
        if name.is_none() {
            name = vdf_value(line, "name");
        }
    }
    Some(SteamGame { appid: appid?, name: name? })
}

fn is_game(game: &SteamGame) -> bool {
    let n = game.name.to_lowercase();
    game.appid != "228980" && !NOT_GAMES.iter().any(|w| n.contains(w))
}

fn scan_library(lib: &Path, out: &mut Vec<SteamGame>) {
    let apps = lib.join("steamapps");
    let Ok(entries) = std::fs::read_dir(&apps) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_manifest = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"))
            .unwrap_or(false);
        if !is_manifest {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Some(game) = parse_manifest(&content) {
                if is_game(&game) && !out.iter().any(|g| g.appid == game.appid) {
                    out.push(game);
                }
            }
        }
    }
}

pub fn games() -> Vec<SteamGame> {
    let mut cache = GAMES.lock();
    if let Some((at, list)) = cache.as_ref() {
        if at.elapsed() < GAMES_TTL {
            return list.clone();
        }
    }

    let mut list = Vec::new();
    if let Some(root) = steam_root() {
        let mut libs = vec![root.clone()];
        if let Ok(vdf) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            for lib in parse_library_folders(&vdf) {
                if !libs.contains(&lib) {
                    libs.push(lib);
                }
            }
        }
        for lib in &libs {
            scan_library(lib, &mut list);
        }
        info!("Steam: {} game(s) in {} library folder(s)", list.len(), libs.len());
    } else {
        info!("Steam is not installed");
    }

    *cache = Some((Instant::now(), list.clone()));
    list
}

// best matching installed game; spoken aliases from [games] in assistant.toml are applied first
pub fn find_game(spoken: &str) -> Option<(f64, SteamGame)> {
    let list = games();
    if list.is_empty() {
        return None;
    }

    let cfg = assistant_config::get();
    let mut queries = vec![spoken.to_string()];
    if let Some((score, target)) = cfg
        .games
        .iter()
        .map(|(k, v)| (similarity(spoken, k), v))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    {
        if score >= 85.0 {
            // alias may point to an appid directly
            if let Some(g) = list.iter().find(|g| &g.appid == target) {
                return Some((100.0, g.clone()));
            }
            queries.push(target.clone());
        }
    }

    queries
        .iter()
        .flat_map(|q| list.iter().map(move |g| (similarity(q, &g.name), g)))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(s, g)| (s, g.clone()))
}

pub fn launch(game: &SteamGame) -> Result<(), String> {
    info!("Launching Steam game {} ({})", game.name, game.appid);
    platform::open_target(&format!("steam://rungameid/{}", game.appid))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIBRARY_VDF: &str = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"228980"		"0"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
}
"#;

    const MANIFEST: &str = r#"
"AppState"
{
	"appid"		"570"
	"Universe"		"1"
	"name"		"Dota 2"
	"StateFlags"		"4"
}
"#;

    #[test]
    fn library_folders_are_parsed() {
        let libs = parse_library_folders(LIBRARY_VDF);
        assert_eq!(libs, vec![PathBuf::from("C:\\Program Files (x86)\\Steam"), PathBuf::from("D:\\SteamLibrary")]);
    }

    #[test]
    fn manifest_is_parsed() {
        let g = parse_manifest(MANIFEST).unwrap();
        assert_eq!(g, SteamGame { appid: "570".into(), name: "Dota 2".into() });
        assert!(is_game(&g));
    }

    #[test]
    fn redistributables_are_not_games() {
        let g = SteamGame { appid: "228980".into(), name: "Steamworks Common Redistributables".into() };
        assert!(!is_game(&g));
        let p = SteamGame { appid: "1493710".into(), name: "Proton Experimental".into() };
        assert!(!is_game(&p));
    }
}
