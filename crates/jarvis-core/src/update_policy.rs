// Updates may only name release installers from this repository.
pub fn valid_setup_url(url: &str) -> bool {
    let Some(path) = url.strip_prefix("https://github.com/softlyfear/jarvis/releases/download/") else { return false; };
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
        assert!(valid_setup_url("https://github.com/softlyfear/jarvis/releases/download/latest/JarvisSetup.exe"));
        assert!(valid_setup_url("https://github.com/softlyfear/jarvis/releases/download/v0.2.14/JarvisSetup.exe"));
        for url in ["https://github.com/softlyfear/jarvis/other.exe", "https://github.com/softlyfear/jarvis/releases/download/../JarvisSetup.exe", "https://github.com/softlyfear/jarvis/releases/download/%2e%2e/JarvisSetup.exe", "https://github.com.evil/softlyfear/jarvis/releases/download/latest/JarvisSetup.exe", "https://github.com/softlyfear/jarvis/releases/download/latest/JarvisSetup.exe?next=evil"] {
            assert!(!valid_setup_url(url), "{url}");
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
