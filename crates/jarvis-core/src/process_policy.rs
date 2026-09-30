use std::path::Path;

/// Identify the assistant from this installation, never a similarly named process.
pub fn is_jarvis_app(name: &str, executable: Option<&Path>, app_dir: &Path) -> bool {
    if !name.eq_ignore_ascii_case("jarvis-app") && !name.eq_ignore_ascii_case("jarvis-app.exe") {
        return false;
    }
    let Some(executable) = executable else { return false; };
    let expected = app_dir.join(if cfg!(windows) { "jarvis-app.exe" } else { "jarvis-app" });
    let actual = executable.canonicalize().unwrap_or_else(|_| executable.to_path_buf());
    let expected = expected.canonicalize().unwrap_or(expected);
    #[cfg(windows)]
    return actual.to_string_lossy().eq_ignore_ascii_case(&expected.to_string_lossy());
    #[cfg(not(windows))]
    return actual == expected;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_current_installation_matches() {
        let root = std::env::temp_dir().join("jarvis-process-policy");
        let name = if cfg!(windows) { "jarvis-app.exe" } else { "jarvis-app" };
        let exe = root.join(name);
        assert!(is_jarvis_app(name, Some(&exe), &root));
        assert!(!is_jarvis_app(name, Some(&root.join("other").join(name)), &root));
        assert!(!is_jarvis_app("my-jarvis-app.exe", Some(&exe), &root));
        assert!(!is_jarvis_app(name, None, &root));
    }
}
