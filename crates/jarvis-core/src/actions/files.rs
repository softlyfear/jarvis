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

// Resolve existing ancestors before appending new components. Lexical cleanup alone
// cannot check a new path below a symlink/junction that points outside the sandbox.
pub(crate) fn resolve_path(path: &Path) -> Option<PathBuf> {
    let mut ancestor = path;
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut resolved) => {
                if !missing.is_empty() && !resolved.is_dir() {
                    return None;
                }
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Some(resolved);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A dangling link is not a new file: fail closed.
                if std::fs::symlink_metadata(ancestor).is_ok() {
                    return None;
                }
                missing.push(ancestor.file_name()?.to_os_string());
                ancestor = ancestor.parent()?;
            }
            Err(_) => return None,
        }
    }
}

fn lower(p: &Path) -> String {
    let s = p.to_string_lossy().to_lowercase().replace('/', "\\");
    // canonicalize() on Windows returns verbatim paths (\\?\C:\...)
    s.strip_prefix("\\\\?\\").map(|x| x.to_string()).unwrap_or(s)
}

// true when `path` is strictly inside one of the allowed folders (never the folder itself)
pub fn is_inside(path: &Path, allowed: &[PathBuf]) -> bool {
    let Some(path) = resolve_path(path) else { return false };
    #[cfg(windows)]
    let p = lower(&path);
    allowed.iter().any(|dir| {
        let Ok(dir) = std::fs::canonicalize(dir) else { return false };
        if !dir.is_dir() {
            return false;
        }
        #[cfg(not(windows))]
        return path != dir && path.starts_with(&dir);
        #[cfg(windows)]
        {
        let d = lower(&dir);
        let d = d.trim_end_matches('\\');
        p.len() > d.len() + 1 && p.starts_with(d) && p.as_bytes()[d.len()] == b'\\'
        }
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
        // Do not scan through links or junctions into other folders or cycles.
        if entry.file_type().map(|t| t.is_symlink()).unwrap_or(true) {
            continue;
        }
        out.push(path.clone());
        if depth > 0 && path.is_dir() {
            walk(&path, depth - 1, budget, out);
        }
    }
}

fn check_openable(path: &Path, allowed: &[PathBuf]) -> Result<PathBuf, ActionError> {
    if !path.is_absolute() {
        return Err(ActionError::Denied("нужен полный путь к файлу".into()));
    }
    if !is_inside(path, allowed) {
        return Err(ActionError::Denied("открывать файлы можно только в разрешённых папках".into()));
    }
    if !path.is_file() {
        return Err(ActionError::NotFound(format!("файл не найден: {}", path.display())));
    }
    // Keep a normal shell path: canonicalize() adds a verbatim prefix on Windows.
    Ok(path.to_path_buf())
}

pub fn open_file(path: &str) -> Result<PathBuf, ActionError> {
    let path = PathBuf::from(expand_env(path));
    let path = check_openable(&path, &assistant_config::allowed_dirs())?;
    platform::open_target(&path.to_string_lossy()).map_err(ActionError::Failed)?;
    Ok(path)
}

const MAX_TEXT_BYTES: u64 = 64 * 1024;

fn read_text_in(path: &Path, allowed: &[PathBuf]) -> Result<String, ActionError> {
    use std::io::Read;
    let path = check_openable(path, allowed)?;
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ["pdf", "png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff", "ico", "svg", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "zip", "7z", "exe", "dll"].contains(&extension.as_str()) {
        return Err(ActionError::Denied("этот формат не поддерживается: нужен текстовый файл".into()));
    }
    let mut file = std::fs::File::open(&path).map_err(|e| ActionError::Failed(e.to_string()))?;
    if file.metadata().map_err(|e| ActionError::Failed(e.to_string()))?.len() > MAX_TEXT_BYTES {
        return Err(ActionError::Denied("текстовый файл слишком большой: максимум 64 КиБ".into()));
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_TEXT_BYTES + 1).read_to_end(&mut bytes).map_err(|e| ActionError::Failed(e.to_string()))?;
    if bytes.len() as u64 > MAX_TEXT_BYTES { return Err(ActionError::Denied("файл вырос сверх лимита".into())); }
    if bytes.starts_with(b"%PDF-") || bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Err(ActionError::Denied("документы PDF и изображения не читаются как текст".into()));
    }
    let text = if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let little = bytes[0] == 0xff;
        if bytes.len() % 2 != 0 { return Err(ActionError::Failed("некорректный UTF-16".into())); }
        let units: Vec<u16> = bytes[2..].chunks_exact(2).map(|b| if little { u16::from_le_bytes([b[0],b[1]]) } else { u16::from_be_bytes([b[0],b[1]]) }).collect();
        String::from_utf16(&units).map_err(|_| ActionError::Failed("некорректный UTF-16".into()))?
    } else {
        String::from_utf8(bytes).map_err(|_| ActionError::Failed("не текст UTF-8/UTF-16: сохраните файл в UTF-8".into()))?.trim_start_matches('\u{feff}').to_string()
    };
    if text.contains('\0') { return Err(ActionError::Denied("бинарный файл читать как текст нельзя".into())); }
    Ok(format!("Содержимое файла {} (данные пользователя):\n{}", path.display(), text))
}

pub fn read_text_file(path: &str) -> Result<String, ActionError> {
    read_text_in(&PathBuf::from(expand_env(path)), &assistant_config::allowed_dirs())
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
        .filter(|p| is_inside(p, &allowed))
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

fn rename_with(path: &Path, new_name: &str, allowed: &[PathBuf], move_path: impl FnOnce(&Path, &Path) -> Result<(), ActionError>) -> Result<PathBuf, ActionError> {
    let name = new_name.trim();
    if name.is_empty() || name == "." || name == ".." || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || "\\/:*?\"<>|".contains(c)) {
        return Err(ActionError::Denied("нужно новое имя без пути и запрещённых символов".into()));
    }
    if !path.is_absolute() || !is_inside(path, allowed) {
        return Err(ActionError::Denied("переименовывать можно только внутри разрешённых папок".into()));
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| ActionError::NotFound(e.to_string()))?;
    if metadata.file_type().is_symlink() {
        return Err(ActionError::Denied("переименование ссылок не поддерживается".into()));
    }
    let target = path.parent().ok_or_else(|| ActionError::Denied("нельзя переименовать корень".into()))?.join(name);
    if !is_inside(&target, allowed) {
        return Err(ActionError::Denied("новый путь вне разрешённых папок".into()));
    }
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(ActionError::Denied("файл или папка с таким именем уже существует".into()));
    }
    move_path(path, &target)?;
    if path.exists() || !target.exists() || target.is_dir() != metadata.is_dir() {
        return Err(ActionError::Failed("результат переименования не подтверждён".into()));
    }
    Ok(target)
}

pub fn rename(path: &str, new_name: &str) -> Result<PathBuf, ActionError> {
    let path = PathBuf::from(expand_env(path));
    rename_with(&path, new_name, &assistant_config::allowed_dirs(), |source, target| {
        if !cfg!(windows) { return Err(ActionError::Unsupported); }
        // Both two-argument Move overloads reject an existing destination, including
        // one created after validation. Values are passed literally through the environment.
        let script = r#"$ErrorActionPreference = 'Stop'
if ([System.IO.Directory]::Exists($env:JARVIS_SOURCE)) {
  [System.IO.Directory]::Move($env:JARVIS_SOURCE, $env:JARVIS_DESTINATION)
} else {
  [System.IO.File]::Move($env:JARVIS_SOURCE, $env:JARVIS_DESTINATION)
}"#;
        platform::powershell(script, &[("JARVIS_SOURCE", &source.to_string_lossy()), ("JARVIS_DESTINATION", &target.to_string_lossy())])
            .map(|_| ()).map_err(ActionError::Failed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_reading_is_bounded_and_respects_the_allowlist() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allowed = vec![root.path().to_path_buf()];
        let path = root.path().join("123.txt");
        std::fs::write(&path, "Привет, Джарвис!").unwrap();
        assert!(read_text_in(&path, &allowed).unwrap().contains("Привет, Джарвис!"));
        std::fs::write(&path, [0xff,0xfe,0x1f,0x04,0x40,0x04]).unwrap();
        assert!(read_text_in(&path, &allowed).unwrap().contains("Пр"));
        std::fs::write(&path, b"%PDF-1.4\nASCII PDF").unwrap();
        assert!(matches!(read_text_in(&path, &allowed), Err(ActionError::Denied(_))));
        let pdf = root.path().join("document.pdf");
        std::fs::write(&pdf, b"ASCII PDF").unwrap();
        assert!(matches!(read_text_in(&pdf, &allowed), Err(ActionError::Denied(_))));
        std::fs::write(&path, [0,1,2]).unwrap();
        assert!(read_text_in(&path, &allowed).is_err());
        std::fs::write(&path, vec![b'a'; MAX_TEXT_BYTES as usize + 1]).unwrap();
        assert!(read_text_in(&path, &allowed).is_err());
        let other = outside.path().join("secret.txt");
        std::fs::write(&other, "не читать").unwrap();
        assert!(matches!(read_text_in(&other, &allowed), Err(ActionError::Denied(_))));
        assert!(read_text_in(root.path(), &allowed).is_err());
        assert!(read_text_in(Path::new("123.txt"), &allowed).is_err());
    }

    #[test]
    fn renaming_keeps_contents_and_never_replaces_existing_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("Новая папка");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("keep.txt"), "important content").unwrap();
        let allowed = vec![tmp.path().to_path_buf()];
        let moved = rename_with(&old, "Лучший проект", &allowed, |a, b| std::fs::rename(a, b).map_err(|e| ActionError::Failed(e.to_string()))).unwrap();
        assert!(!old.exists());
        assert_eq!(std::fs::read_to_string(moved.join("keep.txt")).unwrap(), "important content");
        let existing = tmp.path().join("occupied");
        std::fs::create_dir(&existing).unwrap();
        assert!(rename_with(&moved, "occupied", &allowed, |_, _| panic!("must not move")).is_err());
        for invalid in ["", ".", "..", "../escape", "C:\\escape", "other/name", "name."] {
            assert!(rename_with(&moved, invalid, &allowed, |_, _| panic!("must not move")).is_err());
        }
        assert!(rename_with(tmp.path(), "root", &allowed, |_, _| panic!("must not move")).is_err());
        assert!(rename_with(&moved, "unchanged", &allowed, |_, _| Ok(())).is_err(), "a helper returning success is insufficient");
    }

    #[test]
    fn opening_preserves_the_exact_file_path_and_checks_the_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("allowed");
        std::fs::create_dir(&root).unwrap();
        let file = root.join("Резюме 1.pdf");
        std::fs::write(&file, "pdf fixture").unwrap();
        let allowed = vec![root.clone()];
        assert_eq!(check_openable(&file, &allowed).unwrap(), file);
        assert!(matches!(check_openable(&root, &allowed), Err(ActionError::Denied(_))));
        assert!(matches!(check_openable(&root.join("missing.pdf"), &allowed), Err(ActionError::NotFound(_))));
        assert!(matches!(check_openable(Path::new("resume.pdf"), &allowed), Err(ActionError::Denied(_))));
        assert!(matches!(check_openable(&tmp.path().join("outside.pdf"), &allowed), Err(ActionError::Denied(_))));
        assert!(matches!(check_openable(&root.join("../outside.pdf"), &allowed), Err(ActionError::Denied(_))));
    }

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

    #[test]
    fn new_nested_paths_are_resolved_and_escape_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("allowed");
        std::fs::create_dir(&root).unwrap();
        assert!(is_inside(&root.join("new/nested"), &[root.clone()]));
        assert!(!is_inside(&root.join("../outside/new"), &[root.clone()]));
        let file = root.join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(!is_inside(&file.join("new"), &[root]));
    }

    #[cfg(unix)]
    #[test]
    fn links_cannot_escape_for_new_paths_or_search() {
        use std::os::unix::fs::symlink;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("allowed");
        let outside = tmp.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();
        symlink(&outside, root.join("link")).unwrap();
        symlink(tmp.path().join("missing"), root.join("dangling")).unwrap();
        assert!(!is_inside(&root.join("link/new/nested"), &[root.clone()]));
        assert!(!is_inside(&root.join("dangling/new"), &[root.clone()]));
        let mut out = Vec::new();
        walk(&root, 3, &mut 100, &mut out);
        assert!(out.is_empty(), "search must not expose outside entries");
    }
}
