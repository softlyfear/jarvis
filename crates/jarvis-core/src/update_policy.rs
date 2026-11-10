pub fn release_base_url() -> Result<String, String> {
    release_base_for(option_env!("JARVIS_UPDATE_REPOSITORY"))
        .ok_or_else(|| "в этой сборке не настроен репозиторий обновлений".into())
}

fn release_base_for(repository: Option<&str>) -> Option<String> {
    let repository = repository?;
    let (owner, name) = repository.split_once('/')?;
    if [owner, name].iter().any(|part| {
        part.is_empty() || *part == "." || *part == ".."
            || !part.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    }) {
        return None;
    }
    Some(format!("https://github.com/{repository}/releases/download/"))
}

// Trust the build configuration, never the repository supplied by remote metadata.
pub fn valid_setup_url(url: &str) -> bool {
    let Ok(base) = release_base_url() else { return false; };
    valid_setup_url_for(url, &base)
}

fn valid_setup_url_for(url: &str, base: &str) -> bool {
    let Some(path) = url.strip_prefix(base) else { return false; };
    let Some(tag) = path.strip_suffix("/JarvisSetup.exe") else { return false; };
    !tag.is_empty() && tag != "." && tag != ".." && tag.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

pub fn verify_sha256(path: &std::path::Path, expected: &str) -> Result<(), String> {
    use std::io::Read;
    use sha2::{Digest, Sha256};
    if expected.len() != 64 || !expected.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("некорректная контрольная сумма установщика".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buffer[..n]);
    }
    if format!("{:x}", hash.finalize()).eq_ignore_ascii_case(expected) { Ok(()) }
    else { Err("контрольная сумма установщика не совпала; попробуйте скачать обновление ещё раз".into()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_repository_release_installers_are_allowed() {
        let base = release_base_for(Some("example/voice-assistant")).unwrap();
        assert!(valid_setup_url_for(&format!("{base}latest/JarvisSetup.exe"), &base));
        assert!(valid_setup_url_for(&format!("{base}v0.2.14/JarvisSetup.exe"), &base));
        for url in [
            "https://github.com/another/voice-assistant/releases/download/latest/JarvisSetup.exe",
            "https://github.com/example/other/releases/download/latest/JarvisSetup.exe",
            "https://github.com/example/voice-assistant/other.exe",
            "https://github.com/example/voice-assistant/releases/download/../JarvisSetup.exe",
            "https://github.com/example/voice-assistant/releases/download/%2e%2e/JarvisSetup.exe",
            "https://github.com.evil/example/voice-assistant/releases/download/latest/JarvisSetup.exe",
            "https://github.com/example/voice-assistant/releases/download/latest/JarvisSetup.exe?next=evil",
            "https://github.com/example/voice-assistant/releases/download/latest/JarvisSetup.exe#fragment",
        ] {
            assert!(!valid_setup_url_for(url, &base), "{url}");
        }
    }

    #[test]
    fn missing_or_invalid_repository_disables_updates() {
        assert!(release_base_for(None).is_none());
        for repository in ["", "example", "example/", "/repo", "example/repo/extra", "../repo", "example/..", "example/repo?x", "example/repo\\extra", "example/repo\n"] {
            assert!(release_base_for(Some(repository)).is_none(), "{repository:?}");
        }
    }

    #[test]
    fn build_repository_drives_both_metadata_and_installer_urls() {
        match release_base_url() {
            Ok(base) => {
                assert!(valid_setup_url(&format!("{base}latest/JarvisSetup.exe")));
                assert!(!valid_setup_url(&format!("{base}latest/version.json")));
                assert!(base.ends_with("/releases/download/"));
            }
            Err(_) => assert!(!valid_setup_url("https://github.com/example/voice-assistant/releases/download/latest/JarvisSetup.exe")),
        }
    }
    #[test]
    fn installer_digest_detects_corruption_and_rejects_invalid_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.exe");
        std::fs::write(&file, "abc").unwrap();
        let hash = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(verify_sha256(&file, hash).is_ok());
        assert!(verify_sha256(&file, &hash.to_uppercase()).is_ok());
        std::fs::write(&file, "abcd").unwrap();
        assert!(verify_sha256(&file, hash).is_err());
        assert!(verify_sha256(&file, "bad").is_err());
    }
}
