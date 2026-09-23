// Folders and files: open by name, search, delete to the Recycle Bin.
// Every file operation is limited to [safety].allowed_dirs from assistant.toml.

use std::path::{Path, PathBuf};

use super::text::{normalize, similarity};
use super::{platform, ActionError};
use crate::assistant_config::{self, expand_env};

const FOLDER_MIN_SCORE: f64 = 80.0;
const FILE_MIN_SCORE: f64 = 70.0;
const MAX_SCAN_DEPTH: usize = 3;
const MAX_SCAN_ENTRIES: usize = 20_000;

#[derive(Debug, Clone)]
pub struct FoundFile {
    pub path: PathBuf,
    pub score: f64,
}

// lexical normalization without touching the file system: resolves "." and ".."
fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn lower(p: &Path) -> String {
    let s = p.to_string_lossy().to_lowercase().replace('/', "\\");
    // canonicalize() on Windows returns verbatim paths (\\?\C:\...)
    s.strip_prefix("\\\\?\\").map(|x| x.to_string()).unwrap_or(s)
}

// true when `path` is strictly inside one of the allowed folders (never the folder itself)
pub fn is_inside(path: &Path, allowed: &[PathBuf]) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| clean_path(path));
    let p = lower(&path);
    allowed.iter().any(|dir| {
        let dir = std::fs::canonicalize(dir).unwrap_or_else(|_| clean_path(dir));
        let d = lower(&dir);
        let d = d.trim_end_matches('\\');
        p.len() > d.len() + 1 && p.starts_with(d) && p.as_bytes()[d.len()] == b'\\'
    })
}

pub fn resolve_folder(spoken: &str) -> Option<(String, PathBuf)> {
    let spoken = normalize(spoken);
    let cfg = assistant_config::get();
    cfg.folders
        .iter()
        .map(|(k, v)| (similarity(&spoken, k), k, v))
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .filter(|(score, _, _)| *score >= FOLDER_MIN_SCORE)
        .map(|(_, k, v)| (k.clone(), PathBuf::from(expand_env(v))))
}

pub fn open_folder(spoken_or_path: &str) -> Result<String, ActionError> {
    let as_path = PathBuf::from(expand_env(spoken_or_path));
    if as_path.is_absolute() && as_path.is_dir() {
        platform::open_target(&as_path.to_string_lossy()).map_err(ActionError::Failed)?;
        return Ok(as_path.to_string_lossy().to_string());
    }
    match resolve_folder(spoken_or_path) {
        Some((name, path)) => {
            platform::open_target(&path.to_string_lossy()).map_err(ActionError::Failed)?;
            Ok(name)
        }
        None => Err(ActionError::NotFound(format!("не знаю папку «{}»", spoken_or_path))),
    }
}

fn walk(dir: &Path, depth: usize, budget: &mut usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.starts_with('.') || n.eq_ignore_ascii_case("desktop.ini"))
            .unwrap_or(true);
        if hidden {
            continue;
        }
        out.push(path.clone());
        if depth > 0 && path.is_dir() {
            walk(&path, depth - 1, budget, out);
        }
    }
}

// search files and folders by name inside the allowed folders (or one folder by name)
pub fn find(query: &str, folder: Option<&str>) -> Result<Vec<FoundFile>, ActionError> {
    let query = normalize(query);
    if query.is_empty() {
        return Err(ActionError::NotFound("не расслышал имя файла".into()));
    }
    let allowed = assistant_config::allowed_dirs();

    let roots: Vec<PathBuf> = match folder.filter(|f| !f.trim().is_empty()) {
        Some(f) => match resolve_folder(f) {
            Some((_, p)) if allowed.iter().any(|a| lower(a) == lower(&p)) || is_inside(&p, &allowed) => vec![p],
            Some(_) => return Err(ActionError::Denied("эта папка вне разрешённых".into())),
            None => allowed.clone(),
        },
        None => allowed.clone(),
    };

    let mut all = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    for root in &roots {
        walk(root, MAX_SCAN_DEPTH, &mut budget, &mut all);
    }

    let mut found: Vec<FoundFile> = all
        .into_iter()
        .filter_map(|p| {
            let stem = p.file_stem()?.to_str()?.to_string();
            let score = similarity(&query, &stem);
            (score >= FILE_MIN_SCORE).then_some(FoundFile { path: p, score })
        })
        .collect();
    found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    found.truncate(5);
    Ok(found)
}

// check a path before a destructive operation; returns the path to act on
pub fn check_deletable(path: &str) -> Result<PathBuf, ActionError> {
    let p = PathBuf::from(expand_env(path));
    if !p.is_absolute() {
        return Err(ActionError::Denied("нужен полный путь к файлу".into()));
    }
    if !p.exists() {
        return Err(ActionError::NotFound(format!("файл не найден: {}", p.display())));
    }
    if !is_inside(&p, &assistant_config::allowed_dirs()) {
        return Err(ActionError::Denied(format!("удалять можно только в разрешённых папках: {}", p.display())));
    }
    Ok(p)
}

// move to the Recycle Bin (recoverable), never a permanent delete
pub fn delete_to_recycle_bin(path: &Path) -> Result<(), ActionError> {
    if !cfg!(windows) {
        return Err(ActionError::Unsupported);
    }
    let script = r#"
Add-Type -AssemblyName Microsoft.VisualBasic
$p = $env:JARVIS_TARGET
if (Test-Path -LiteralPath $p -PathType Container) {
  [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin')
} else {
  [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin')
}
"#;
    platform::powershell(script, &[("JARVIS_TARGET", &path.to_string_lossy())])
        .map(|_| ())
        .map_err(ActionError::Failed)
}

pub fn create_folder(path: &str) -> Result<PathBuf, ActionError> {
    let p = PathBuf::from(expand_env(path));
    if !p.is_absolute() {
        return Err(ActionError::Denied("нужен полный путь".into()));
    }
    if !is_inside(&p, &assistant_config::allowed_dirs()) {
        return Err(ActionError::Denied("создавать папки можно только в разрешённых местах".into()));
    }
    std::fs::create_dir_all(&p).map_err(|e| ActionError::Failed(e.to_string()))?;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_check_rejects_escape_and_root_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let inner = root.join("a.txt");
        std::fs::write(&inner, "x").unwrap();
        let allowed = vec![root.clone()];

        assert!(is_inside(&inner, &allowed));
        assert!(!is_inside(&root, &allowed), "the allowed folder itself must not be deletable");
        assert!(!is_inside(&root.join("..").join("etc"), &allowed));
        let sibling = PathBuf::from(format!("{}-other", root.display())).join("x");
        assert!(!is_inside(&sibling, &allowed), "prefix of a sibling folder must not match");
    }

    #[test]
    fn verbatim_prefix_is_ignored() {
        assert_eq!(lower(Path::new(r"\\?\C:\Users\Me")), r"c:\users\me");
        assert_eq!(lower(Path::new("C:/Users/Me")), r"c:\users\me");
    }

    #[test]
    fn walk_skips_hidden_and_respects_budget() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".hidden"), "x").unwrap();
        std::fs::write(tmp.path().join("visible.txt"), "x").unwrap();
        let mut out = Vec::new();
        let mut budget = 100;
        walk(tmp.path(), 1, &mut budget, &mut out);
        assert_eq!(out.len(), 1);

        let mut out = Vec::new();
        let mut budget = 0;
        walk(tmp.path(), 1, &mut budget, &mut out);
        assert!(out.is_empty());
    }
}
