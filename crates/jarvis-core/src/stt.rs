#[cfg(feature = "vosk")]
mod vosk;

use crate::config;
use once_cell::sync::OnceCell;

use crate::config::structs::SpeechToTextEngine;
pub use self::vosk::init_vosk;
pub use self::vosk::recognize_wake_word;
pub use self::vosk::recognize_wake_word_partial;
pub use self::vosk::recognize_speech;
pub use self::vosk::reset_wake_recognizer;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::whisper::{self, UtteranceBuffer};

static STT_TYPE: OnceCell<SpeechToTextEngine> = OnceCell::new();

// Vosk listening alone (the voice server is down): which of its final results is fresh
static UTTERANCE: Lazy<Mutex<UtteranceBuffer>> = Lazy::new(|| Mutex::new(UtteranceBuffer::default()));
// while the voice server answers it alone recognizes speech; Vosk only finds the wake word
static SERVER: Lazy<Mutex<whisper::ServerListener>> = Lazy::new(|| Mutex::new(whisper::ServerListener::default()));

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

    if whisper::enabled() {
        return recognize_with_server(data);
    }

    UTTERANCE.lock().push(data);
    let prefed = UTTERANCE.lock().take_prefed(whisper::PREFED_MAX_AGE_SAMPLES);
    let vosk_text = match prefed {
        Some(text) => text,
        None => vosk::recognize_speech_finalized(data)?,
    };
    UTTERANCE.lock().take();

    // keep upstream semantics: an empty final result means "nothing recognized"
    if vosk_text.is_empty() {
        return None;
    }
    Some(vosk_text)
}

// The voice server recognizes each utterance the endpointer cuts. If it fails, Vosk
// recognizes that utterance from its audio and listens on its own until the server is retried.
fn recognize_with_server(data: &[i16]) -> Option<String> {
    let audio = SERVER.lock().next_utterance(data)?;
    match whisper::transcribe(&audio) {
        Ok(Some(text)) if !text.is_empty() => Some(text),
        // no speech in it (noise) or too short to bother
        Ok(_) => None,
        Err(_) => {
            let text = vosk::recognize_audio(&audio);
            info!("Vosk instead of the voice server: '{}'", text);
            (!text.is_empty()).then_some(text)
        }
    }
}

// feed audio whose result is not needed (dual-feed while waiting for the wake word)
pub fn feed(data: &[i16]) {
    if whisper::enabled() {
        SERVER.lock().feed(data);
        return;
    }
    UTTERANCE.lock().push(data);
    if let Some(text) = vosk::recognize_speech_finalized(data) {
        UTTERANCE.lock().remember_prefed(text);
    }
}

pub fn reset_speech_recognizer() {
    UTTERANCE.lock().clear();
    SERVER.lock().clear();
    vosk::reset_speech_recognizer();
}
