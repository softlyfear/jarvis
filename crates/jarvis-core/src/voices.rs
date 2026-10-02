use std::fs;
use std::path::{Path, PathBuf};
use rand::prelude::*;
use once_cell::sync::OnceCell;
// use chrono::Timelike;

use crate::{DB, SOUND_DIR, audio, config, time};

mod structs;
pub use structs::*;

pub const VOICE_ID: &str = "jarvis-remaster";
static VOICE: OnceCell<structs::VoiceConfig> = OnceCell::new();

pub fn init() -> Result<(), String> {
    let voice = load_single_voice(&SOUND_DIR.join(config::VOICES_PATH))?;
    info!("Loaded voice: {} ({})", voice.voice.name, VOICE_ID);
    VOICE.set(voice).map_err(|_| "Voice already initialized".to_string())
}

fn load_single_voice(voices_dir: &Path) -> Result<structs::VoiceConfig, String> {
    // Old installations may still have other packs. Only Jarvis New is ever loaded.
    let voice_path = voices_dir.join(VOICE_ID);
    let voice = load_voice_config(&voice_path.join("voice.toml"), &voice_path)?;
    if voice.voice.id != VOICE_ID {
        return Err("Jarvis New voice metadata has an unexpected id".into());
    }
    Ok(voice)
}

fn load_voice_config(toml_path: &Path, voice_path: &Path) -> Result<structs::VoiceConfig, String> {
    let content = fs::read_to_string(toml_path)
        .map_err(|e| format!("Failed to read voice.toml: {}", e))?;
    
    let mut config: structs::VoiceConfig = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse voice.toml: {}", e))?;
    
    config.path = voice_path.to_path_buf();
    
    Ok(config)
}

pub fn get_current_voice() -> Option<&'static structs::VoiceConfig> {
    VOICE.get()
}

pub fn current_id() -> &'static str {
    VOICE_ID
}

fn get_current_language() -> String {
    DB.get()
        .map(|db| db.read().language.clone())
        .unwrap_or_else(|| "ru".to_string())
}

fn find_sound_file(voice_path: &Path, lang: &str, sound_name: &str) -> Option<PathBuf> {
    const EXTENSIONS: &[&str] = &["mp3", "wav", "ogg"];
    let lang_path = voice_path.join(lang);
    
    // try language subfolder first (/en, /ua, /ru, etc)
    for ext in EXTENSIONS {
        let file_path = lang_path.join(format!("{}.{}", sound_name, ext));
        if file_path.exists() {
            return Some(file_path);
        }
    }
    
    // fallback to root voice folder
    for ext in EXTENSIONS {
        let file_path = voice_path.join(format!("{}.{}", sound_name, ext));
        if file_path.exists() {
            return Some(file_path);
        }
    }
    
    None
}

fn play_random_from_list(voice_path: &Path, lang: &str, sounds: &[String]) {
    if sounds.is_empty() {
        return;
    }
    
    let sound_name = sounds.choose(&mut rand::thread_rng()).unwrap();
    
    match find_sound_file(voice_path, lang, sound_name) {
        Some(path) => {
            debug!("Playing: {:?}", path);
            audio::play_sound(&path);
        }
        None => {
            warn!("Sound not found: {} (lang: {})", sound_name, lang);
        }
    }
}

pub fn play(reaction: structs::Reaction) {
    let voice = match get_current_voice() {
        Some(v) => v,
        None => {
            warn!("No current voice set");
            return;
        }
    };
    
    let lang = get_current_language();

    // another address than the recorded "сэр": the same reply in the cloned voice
    let kind = match reaction {
        structs::Reaction::Greet => match time::TimeOfDay::now() {
            time::TimeOfDay::Morning => "greet_morning",
            time::TimeOfDay::Day => "greet_day",
            time::TimeOfDay::Evening => "greet_evening",
            time::TimeOfDay::Night => "greet_night",
        },
        structs::Reaction::Reply => "reply",
        structs::Reaction::Ok => "ok",
        structs::Reaction::NotFound => "not_found",
        structs::Reaction::Thanks => "thanks",
        structs::Reaction::Error => "error",
        structs::Reaction::Goodbye => "goodbye",
    };
    if let Some(path) = crate::phrases::cached(kind, &lang) {
        audio::play_sound(&path);
        return;
    }
    
    let reactions = match voice.reactions.get(&lang) {
        Some(r) => r,
        None => {
            warn!("No reactions for language: {}", lang);
            return;
        }
    };

    let sounds = match reaction {
        structs::Reaction::Greet => {
            // try time-specific first
            let time_specific = match time::TimeOfDay::now() {
                time::TimeOfDay::Morning => &reactions.greet_morning,
                time::TimeOfDay::Day => &reactions.greet_day,
                time::TimeOfDay::Evening => &reactions.greet_evening,
                time::TimeOfDay::Night => &reactions.greet_night,
            };

            if time_specific.is_empty() {
                &reactions.greet
            } else {
                time_specific
            }
        }
        structs::Reaction::Reply => &reactions.reply,
        structs::Reaction::Ok => &reactions.ok,
        structs::Reaction::NotFound => &reactions.not_found,
        structs::Reaction::Thanks => &reactions.thanks,
        structs::Reaction::Error => &reactions.error,
        structs::Reaction::Goodbye => &reactions.goodbye,
    };
    
    play_random_from_list(&voice.path, &lang, sounds);
}

pub fn play_random_from(sounds: &[String]) {
    let voice = match get_current_voice() {
        Some(v) => v,
        None => {
            warn!("No current voice set");
            return;
        }
    };
    
    let lang = get_current_language();

    // with another address than "сэр" the command's recorded "сэр" is replaced too
    if let Some(path) = crate::phrases::cached("ok", &lang) {
        audio::play_sound(&path);
        return;
    }

    // a command sound this pack does not have: its ordinary "done" instead of silence
    let available: Vec<String> =
        sounds.iter().filter(|s| find_sound_file(&voice.path, &lang, s).is_some()).cloned().collect();
    if available.is_empty() && !sounds.is_empty() {
        play(structs::Reaction::Ok);
        return;
    }
    play_random_from_list(&voice.path, &lang, &available);
}

// shortcuts
pub fn play_greet() { play(structs::Reaction::Greet); } // app startup
pub fn play_reply() { play(structs::Reaction::Reply); } // wake word detected
pub fn play_ok() { play(structs::Reaction::Ok); } // command executed
pub fn play_not_found() { play(structs::Reaction::NotFound); }
pub fn play_thanks() { play(structs::Reaction::Thanks); }
pub fn play_error() { play(structs::Reaction::Error); }
pub fn play_goodbye() { play(structs::Reaction::Goodbye); }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_jarvis_new_is_loaded_even_if_old_packs_remain() {
        let root = tempfile::tempdir().unwrap();
        let pack = root.path().join(VOICE_ID);
        fs::create_dir_all(&pack).unwrap();
        fs::write(pack.join("voice.toml"), include_str!("../../../resources/sound/voices/jarvis-remaster/voice.toml")).unwrap();
        fs::create_dir(root.path().join("jarvis-og")).unwrap();
        fs::write(root.path().join("jarvis-og/voice.toml"), "invalid TOML").unwrap();
        let voice = load_single_voice(root.path()).unwrap();
        assert_eq!(voice.voice.id, VOICE_ID);
        assert_eq!(voice.voice.name, "Jarvis New");
        assert_eq!(voice.path, pack);
    }

    #[test]
    fn missing_jarvis_new_does_not_select_an_old_voice() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("jarvis-og")).unwrap();
        assert!(load_single_voice(root.path()).is_err());
    }
}
