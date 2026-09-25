use once_cell::sync::OnceCell;
use std::path::PathBuf;
use std::sync::Mutex;

// use kira::{
//     manager::{backend::DefaultBackend, AudioManager, AudioManagerSettings},
//     sound::static_sound::{StaticSoundData, StaticSoundSettings},
// };

use kira::{
    AudioManager, AudioManagerSettings, DefaultBackend, Tween,
    sound::{static_sound::{StaticSoundData, StaticSoundHandle}, PlaybackState},
};

static MANAGER: OnceCell<Mutex<AudioManager>> = OnceCell::new();
// sounds that may still be playing, so speech can be cut off
static PLAYING: Mutex<Vec<StaticSoundHandle>> = Mutex::new(Vec::new());

pub fn stop_all() {
    let mut playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    for h in playing.iter_mut() {
        h.stop(Tween { duration: std::time::Duration::from_millis(80), ..Default::default() });
    }
    playing.clear();
}

pub fn init() -> Result<(), ()> {
    if MANAGER.get().is_some() {
        return Ok(());
    }  // already initialized

    // Create an audio manager. This plays sounds and manages resources.
    match AudioManager::<DefaultBackend>::new(AudioManagerSettings::default()) {
        Ok(manager) => {
            // store
            MANAGER.set(Mutex::new(manager)).ok();

            // success
            Ok(())
        }
        Err(msg) => {
            error!("Failed to initialize audio stream.\nError details: {}", msg);

            // failed
            Err(())
        }
    }
}

// @TODO. Cache sounds in memory? With a pool of a certain size, for instance.
pub fn play_sound(filename: &PathBuf) {
    // load the file
    match StaticSoundData::from_file(filename) {
        Ok(sound_data) => {
            // sound_data.duration() can be used in order to sleep, if (for some reason) blocking behaviour is required

            // play it (non-blocking); the listener ignores the microphone meanwhile
            super::hold_microphone(sound_data.duration());
            if let Some(manager) = MANAGER.get() {
                if let Ok(mut audio_manager) = manager.lock() {
                    match audio_manager.play(sound_data) {
                        Ok(handle) => {
                            let mut playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
                            playing.retain(|h| h.state() != PlaybackState::Stopped);
                            playing.push(handle);
                        }
                        Err(e) => warn!("Failed to play sound: {}", e),
                    }
                }
            } else {
                warn!("Audio manager not initialized");
            }
        }
        Err(err) => {
            warn!("Cannot find sound file: {} (err: {})", filename.display(), err);
        }
    }
}