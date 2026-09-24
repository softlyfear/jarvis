#[cfg(feature = "vosk")]
mod vosk;

use crate::config;
use once_cell::sync::OnceCell;

use crate::config::structs::SpeechToTextEngine;
pub use self::vosk::init_vosk;
pub use self::vosk::recognize_wake_word;
pub use self::vosk::recognize_speech;
pub use self::vosk::reset_wake_recognizer;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::whisper::{self, UtteranceBuffer};

static STT_TYPE: OnceCell<SpeechToTextEngine> = OnceCell::new();

// audio of the current utterance, re-recognized by Whisper when Vosk finalizes it
static UTTERANCE: Lazy<Mutex<UtteranceBuffer>> = Lazy::new(|| Mutex::new(UtteranceBuffer::default()));

pub fn init() -> Result<(), String> {
    if STT_TYPE.get().is_some() {
        return Ok(());
    }

    STT_TYPE.set(config::DEFAULT_SPEECH_TO_TEXT_ENGINE)
        .map_err(|_| "STT type already set".to_string())?;

    match STT_TYPE.get().unwrap() {
        SpeechToTextEngine::Vosk => {
            info!("Initializing Vosk STT backend.");
            vosk::init_vosk()?;
            info!("STT backend initialized.");
        }
    }

    Ok(())
}

pub fn recognize(data: &[i16], include_partial: bool) -> Option<String> {
    if include_partial {
        return vosk::recognize_wake_word(data).map(|(text, _)| text);
    }

    UTTERANCE.lock().push(data);
    let vosk_text = vosk::recognize_speech_finalized(data)?;
    let audio = UTTERANCE.lock().take();

    // keep upstream semantics: an empty final result means "nothing recognized"
    if vosk_text.is_empty() {
        return None;
    }
    Some(whisper::refine(vosk_text, &audio))
}

// feed audio whose result is not needed (dual-feed while waiting for the wake word)
pub fn feed(data: &[i16]) {
    UTTERANCE.lock().push(data);
    if vosk::recognize_speech_finalized(data).is_some() {
        UTTERANCE.lock().clear();
    }
}

pub fn reset_speech_recognizer() {
    UTTERANCE.lock().clear();
    vosk::reset_speech_recognizer();
}
