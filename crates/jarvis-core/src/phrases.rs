// Jarvis's short replies ("Слушаю, сэр", "Выполнено, сэр") are recorded in the voice packs
// with "сэр". When the user picked another address ("мисс"), the same replies are spoken
// in the cloned voice instead: synthesized once by the voice server (cloning the selected
// pack), cached as WAV files in the config directory, then played instantly like the recorded ones.

use std::path::PathBuf;

use rand::seq::SliceRandom;

use crate::{assistant_config, APP_CONFIG_DIR};

const CACHE_DIR: &str = "phrases";

// reply kinds of voices::play; "{a}" is the address
fn templates(kind: &str) -> &'static [&'static str] {
    match kind {
        "greet" => &["Джарвис к вашим услугам, {a}.", "Системы запущены, {a}."],
        "greet_morning" => &["Доброе утро, {a}."],
        "greet_day" => &["Добрый день, {a}."],
        "greet_evening" => &["Добрый вечер, {a}."],
        "greet_night" => &["Доброй ночи, {a}."],
        "reply" => &["Слушаю, {a}.", "Да, {a}?", "К вашим услугам, {a}.", "Я здесь, {a}."],
        "ok" => &["Выполнено, {a}.", "Готово, {a}.", "Сделано, {a}.", "Как скажете, {a}."],
        "not_found" => &["Простите, {a}, я не понял команду.", "Не расслышал, {a}. Повторите, пожалуйста."],
        "thanks" => &["Всегда рад помочь, {a}.", "Не за что, {a}."],
        "error" => &["Что-то пошло не так, {a}."],
        "goodbye" => &["До встречи, {a}.", "Отключаюсь, {a}."],
        _ => &[],
    }
}

const KINDS: &[&str] = &[
    "reply", "ok", "not_found", "greet", "greet_morning", "greet_day", "greet_evening", "greet_night", "thanks", "error",
    "goodbye",
];

pub fn texts(kind: &str, address: &str) -> Vec<String> {
    templates(kind).iter().map(|t| t.replace("{a}", address)).collect()
}

// the recorded packs already say "сэр"; other addresses need the cloned voice
pub fn enabled(address: &str, language: &str, tts_backend: &str) -> bool {
    language == "ru" && address != assistant_config::DEFAULT_ADDRESS && tts_backend == "http"
}

fn is_enabled(language: &str) -> bool {
    let cfg = assistant_config::get();
    enabled(&assistant_config::address(), language, &cfg.tts.backend)
}

// FNV-1a: stable across builds, unlike DefaultHasher; each voice pack has its own files
fn file_name(voice: &str, text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    let key = if voice.is_empty() { text.to_string() } else { format!("{}\n{}", voice, text) };
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}.wav", h)
}

fn cache_path(text: &str) -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join(CACHE_DIR).join(file_name(&crate::voices::current_id(), text)))
}

// a ready phrase of this kind, None when the recorded pack should play
pub fn cached(kind: &str, language: &str) -> Option<PathBuf> {
    if !is_enabled(language) {
        return None;
    }
    let ready: Vec<PathBuf> = texts(kind, &assistant_config::address())
        .iter()
        .filter_map(|t| cache_path(t))
        .filter(|p| p.exists())
        .collect();
    if ready.is_empty() {
        debug!("No synthesized phrase for '{}' yet, the recorded one plays", kind);
    }
    ready.choose(&mut rand::thread_rng()).cloned()
}

// synthesizes the missing phrases once the voice server answers (it loads for minutes)
#[cfg(feature = "reqwest")]
pub fn prewarm() {
    if !is_enabled("ru") {
        return;
    }
    std::thread::spawn(|| {
        let server = &assistant_config::get().voice_server;
        let address = assistant_config::address();
        let missing: Vec<(String, PathBuf)> = KINDS
            .iter()
            .flat_map(|k| texts(k, &address))
            .filter_map(|t| cache_path(&t).map(|p| (t, p)))
            .filter(|(_, p)| !p.exists())
            .collect();
        if missing.is_empty() {
            return;
        }
        // up to 15 minutes: the first start downloads nothing, but XTTS on a CPU is slow to load
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15 * 60);
        while !crate::voice_server::is_running(server) {
            if std::time::Instant::now() > deadline {
                warn!("Phrases: voice server did not start, recorded replies stay");
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        info!("Phrases: synthesizing {} replies with «{}»", missing.len(), address);
        let mut failed = 0;
        for (text, path) in missing {
            // generous: on a CPU the voice takes tens of seconds per phrase
            match crate::tts::synthesize_within(&text, std::time::Duration::from_secs(180)) {
                Ok(wav) => {
                    let _ = std::fs::create_dir_all(path.parent().unwrap());
                    // write then rename: a half-written file must never be played
                    let tmp = path.with_extension("tmp");
                    if std::fs::write(&tmp, &wav).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
                        warn!("Phrases: cannot save {}", path.display());
                    }
                }
                Err(e) => {
                    warn!("Phrases: '{}' failed ({}), retried on the next start", text, e);
                    failed += 1;
                    if failed >= 3 {
                        return;
                    }
                }
            }
        }
        info!("Phrases: done ({} failed)", failed);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_goes_into_every_phrase() {
        for kind in KINDS {
            let t = texts(kind, "мисс");
            assert!(!t.is_empty(), "{}", kind);
            assert!(t.iter().all(|p| p.contains("мисс") && !p.contains("{a}")), "{:?}", t);
        }
        assert!(texts("unknown", "мисс").is_empty());
    }

    #[test]
    fn only_a_non_default_address_with_the_voice_server_uses_phrases() {
        assert!(enabled("мисс", "ru", "http"));
        assert!(!enabled("сэр", "ru", "http")); // the recorded pack says it already
        assert!(!enabled("мисс", "ru", "sapi"));
        assert!(!enabled("мисс", "en", "http"));
    }

    #[test]
    fn cache_names_are_stable() {
        assert_eq!(file_name("jarvis-og", "Слушаю, мисс."), file_name("jarvis-og", "Слушаю, мисс."));
        assert_ne!(file_name("jarvis-og", "Слушаю, мисс."), file_name("jarvis-og", "Слушаю, сэр."));
        // another pack is cloned from other samples
        assert_ne!(file_name("jarvis-og", "Слушаю, мисс."), file_name("jarvis-remaster", "Слушаю, мисс."));
        assert_eq!(file_name("", ""), "cbf29ce484222325.wav");
    }
}
