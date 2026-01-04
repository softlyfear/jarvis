#[cfg(feature = "vosk")]
mod vosk;

use crate::config;
use once_cell::sync::OnceCell;

use crate::config::structs::SpeechToTextEngine;

use crate::vosk_models;
// use vosk_models::{scan_vosk_models, get_model_path, VoskModelInfo};
pub use self::vosk::init_vosk;
pub use self::vosk::recognize_wake_word;
pub use self::vosk::recognize_speech;
pub use self::vosk::reset_speech_recognizer;
pub use self::vosk::reset_wake_recognizer;

static STT_TYPE: OnceCell<SpeechToTextEngine> = OnceCell::new();

pub fn init() -> Result<(), ()> {
    if STT_TYPE.get().is_some() {
        return Ok(());
    } // already initialized

    // set default stt type
    // @TODO. Make it configurable?
    STT_TYPE.set(config::DEFAULT_SPEECH_TO_TEXT_ENGINE).unwrap();

    // load given recorder
    match STT_TYPE.get().unwrap() {
        SpeechToTextEngine::Vosk => {
            // Init Vosk
            info!("Initializing Vosk STT backend.");
            vosk::init_vosk();
            info!("STT backend initialized.");
        }
    }

    Ok(())
}

pub fn recognize(data: &[i16], include_partial: bool) -> Option<String> {
    if include_partial {
        vosk::recognize_wake_word(data).map(|(text, _)| text)
    } else {
        vosk::recognize_speech(data)
    }
}

// pub fn recognize(data: &[i16], partial: bool) -> Option<String> {
//     match STT_TYPE.get().unwrap() {
//         SpeechToTextEngine::Vosk => vosk::recognize(data, partial),
//     }
// }
