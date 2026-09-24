mod kira;
mod rodio;

use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::config::structs::AudioType;
use crate::{config, DB, SOUND_DIR};

static AUDIO_TYPE: OnceCell<AudioType> = OnceCell::new();

// While Jarvis talks through the speakers the microphone hears him: his own "what are you
// trying to achieve, sir" came back as a command and looped. The listener skips the
// microphone until this moment.
static SPEAKING_UNTIL: Mutex<Option<Instant>> = Mutex::new(None);
// room echo and the audio device's own latency
const SPEECH_TAIL: Duration = Duration::from_millis(350);

// the assistant is going to speak for `d` (a sound of known length)
pub fn hold_microphone(d: Duration) {
    let until = Instant::now() + d + SPEECH_TAIL;
    let mut s = SPEAKING_UNTIL.lock().unwrap_or_else(|e| e.into_inner());
    if s.is_none_or(|t| t < until) {
        *s = Some(until);
    }
}

// blocking speech (SAPI) has ended: listen again after the tail
pub fn release_microphone() {
    *SPEAKING_UNTIL.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + SPEECH_TAIL);
}

pub fn is_speaking() -> bool {
    SPEAKING_UNTIL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some_and(|t| Instant::now() < t)
}

pub fn init() -> Result<(), ()> {
    if AUDIO_TYPE.get().is_some() {
        return Ok(());
    } // already initialized

    // set default audio type
    // @TODO. Make it configurable?
    AUDIO_TYPE.set(config::DEFAULT_AUDIO_TYPE).unwrap();

    // load given audio backend
    match AUDIO_TYPE.get().unwrap() {
        AudioType::Rodio => {
            // Init Rodio
            info!("Initializing Rodio audio backend.");

            match rodio::init() {
                Ok(_) => {
                    info!("Successfully initialized Rodio audio backend.");
                }
                Err(()) => {
                    error!("Failed to initialize Rodio audio backend.");

                    return Err(());
                }
            }
        }
        AudioType::Kira => {
            // Init Kira
            info!("Initializing Kira audio backend.");

            match kira::init() {
                Ok(_) => {
                    info!("Successfully initialized Kira audio backend.");
                }
                Err(_msg) => {
                    error!("Failed to initialize Kira audio backend.");

                    return Err(());
                }
            }
        }
    }

    Ok(())
}

pub fn play_sound(filename: &PathBuf) {
    let audio_type = match AUDIO_TYPE.get() {
        Some(t) => t,
        None => {
            warn!("Audio not initialized, cannot play: {}", filename.display());
            return;
        }
    };
    
    info!("Playing {}", filename.display());

    match audio_type {
        AudioType::Rodio => {
            hold_microphone(Duration::from_secs(60));
            rodio::play_sound(filename, true);
            release_microphone();
        }
        AudioType::Kira => kira::play_sound(filename),
    }
}

pub fn get_sound_directory() -> Option<PathBuf> {
    let db = DB.get()?;

    let voice_path = {
        let s = db.read();
        SOUND_DIR.join(&s.voice)
    };

    match voice_path.exists() {
        true => Some(voice_path),
        _ => {
            error!("No sounds folder found. Search path - {:?}", voice_path);
            None
        }
    }
}

#[cfg(test)]
mod speaking_tests {
    use super::*;

    #[test]
    fn microphone_is_held_while_speaking_and_released_after_the_tail() {
        hold_microphone(Duration::from_millis(50));
        assert!(is_speaking());
        // a shorter sound does not cut a longer one
        hold_microphone(Duration::ZERO);
        std::thread::sleep(SPEECH_TAIL + Duration::from_millis(10));
        assert!(is_speaking());
        std::thread::sleep(Duration::from_millis(60));
        assert!(!is_speaking());

        hold_microphone(Duration::from_secs(60));
        release_microphone();
        assert!(is_speaking());
        std::thread::sleep(SPEECH_TAIL + Duration::from_millis(20));
        assert!(!is_speaking());
    }
}
