mod rustpotter;
mod vosk;

use once_cell::sync::OnceCell;

use crate::config::structs::WakeWordEngine;

use crate::DB;

static WAKE_WORD_ENGINE: OnceCell<WakeWordEngine> = OnceCell::new();

pub fn barge_in_callback(frame: &[i16]) -> Option<i32> {
    vosk::barge_in_callback(frame)
}

pub fn init() -> Result<(), String> {
    if WAKE_WORD_ENGINE.get().is_some() {
        return Ok(());
    }

    // The command pipeline relies on Vosk: it hears the name and cuts the phrase that woke
    // Jarvis up. Rustpotter, chosen in an old settings window, is not used any more.
    let saved = DB.get().unwrap().read().wake_word_engine;
    if saved != WakeWordEngine::Vosk {
        info!("Wake-word engine {:?} from the settings is replaced with Vosk.", saved);
    }
    let engine = WakeWordEngine::Vosk;

    WAKE_WORD_ENGINE.set(engine)
        .map_err(|_| "Wake word engine already set".to_string())?;

    match engine {
        WakeWordEngine::Porcupine => {
            Err("Porcupine wake-word engine is not supported".to_string())
        }
        WakeWordEngine::Rustpotter => {
            info!("Initializing Rustpotter wake-word engine.");
            rustpotter::init()
                .map_err(|_| "Failed to init Rustpotter".to_string())
        }
        WakeWordEngine::Vosk => {
            info!("Initializing Vosk as wake-word engine.");
            vosk::init()
                .map_err(|_| "Failed to init Vosk wake-word".to_string())
        }
    }
}

pub fn data_callback(frame_buffer: &[i16]) -> Option<i32> {
    match WAKE_WORD_ENGINE.get()? {
        WakeWordEngine::Porcupine => None,
        WakeWordEngine::Rustpotter => rustpotter::data_callback(frame_buffer),
        WakeWordEngine::Vosk => vosk::data_callback(frame_buffer),
    }
}
