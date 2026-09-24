// no console window in release builds: Jarvis lives in the tray
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::slots;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

// include core
use jarvis_core::{
    audio, audio_processing, commands, config, db, listener, recorder, stt, intent, assistant_config, voice_server,
    ipc::{self, IpcAction},
    i18n, voices, models,
    APP_CONFIG_DIR, APP_LOG_DIR, COMMANDS_LIST, DB,
};

// include log
#[macro_use]
extern crate simple_log;
mod log;

// include app
mod app;

// include tray
// @TODO. macOS currently not supported for tray functionality.
#[cfg(not(target_os = "macos"))]
mod tray;

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

fn main() -> Result<(), String> {
    // initialize directories
    config::init_dirs()?;

    // initialize logging
    log::init_logging()?;

    // a crash must not look like "the window flashed and nothing happened"
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("Джарвис аварийно завершился: {}", info);
        error!("{}", msg);
        app::show_error(&msg);
    }));

    // one Jarvis at a time: several copies answer each phrase, fight over the microphone
    // and keep an old voice after the settings change
    if !single_instance() {
        info!("Jarvis is already running, this copy exits.");
        return Ok(());
    }

    // log some base info
    info!("Starting Jarvis v{} (build {}) ...", config::APP_VERSION.unwrap(), option_env!("JARVIS_BUILD").unwrap_or("local"));
    info!("Config directory is: {}", APP_CONFIG_DIR.get().unwrap().display());
    info!("Log directory is: {}", APP_LOG_DIR.get().unwrap().display());

    // user-editable assistant settings (LLM keys, TTS, app aliases)
    assistant_config::init();

    // Whisper + voice clone server (tools/voice-server), if installed
    info!("Voice server: {}", voice_server::start());

    // initialize settings
    let settings = db::init();

    // set global DB (for core modules that read settings at init time)
    DB.set(settings.arc().clone())
            .expect("DB already initialized");

    // init voices
    let voice_id = settings.lock().voice.clone();
    let language = settings.lock().language.clone();
    if let Err(e) = voices::init(&voice_id, &language) {
        warn!("Failed to init voices: {}", e);
    }
    // replies with the user's address in the cloned voice of this pack, once the server is up
    jarvis_core::phrases::prewarm();

    // init i18n
    i18n::init(&settings.lock().language);

    // init recorder
    if recorder::init().is_err() {
        app::fatal("Не удалось открыть микрофон.\n\nПроверьте, что микрофон подключён и Windows разрешает к нему доступ: Параметры → Конфиденциальность → Микрофон.");
    }

    // init models registry (scans available AI models)
    if let Err(e) = models::init() {
        warn!("Models registry init failed: {}", e);
    }

    // init stt engine
    if let Err(e) = stt::init() {
        app::fatal(&format!("Не удалось загрузить распознавание речи (Vosk): {}\n\nПроверьте, что папка resources\\vosk на месте и путь установки без русских букв.", e));
    }

    // init commands
    info!("Initializing commands.");
    let cmds = match commands::parse_commands() {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse commands: {}. Starting with empty command list.", e);
            Vec::new()
        }
    };
    info!("Commands initialized. Count: {}, List: {:?}", cmds.len(), commands::list_paths(&cmds));
    COMMANDS_LIST.set(cmds).unwrap();

    // init audio
    if audio::init().is_err() {
        app::fatal("Не удалось открыть устройство вывода звука (колонки или наушники).");
    }

    // init wake-word engine
    if let Err(e) = listener::init() {
        app::fatal(&format!("Не удалось запустить распознавание слова «Джарвис»: {}", e));
    }

    // shared async runtime for intent classification, IPC, etc.
    let rt = Arc::new(
        tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
    );

    // init intent-recognition engine
    rt.block_on(async {
        if let Err(e) = intent::init(COMMANDS_LIST.get().unwrap()).await {
            app::fatal(&format!("Не удалось подготовить распознавание команд: {}", e));
        }
    });

    // init slots parsing engine
    slots::init().map_err(|e| error!("Slot extraction init failed: {}", e)).ok();

    // init audio processing
    info!("Initializing audio processing...");
    if let Err(e) = audio_processing::init() {
        warn!("Audio processing init failed: {}", e);
    }

    // init IPC
    info!("Initializing IPC...");
    ipc::init();

    // channel for text commands (manually written in the GUI)
    let (text_cmd_tx, text_cmd_rx) = mpsc::channel::<String>();

    ipc::set_action_handler(move |action| {
        if !matches!(action, IpcAction::Ping) {
            info!("GUI action: {:?}", action);
        }
        match action {
            IpcAction::Stop => {
                info!("Received stop command from GUI");
                SHOULD_STOP.store(true, Ordering::SeqCst);
            }
            IpcAction::ReloadCommands => {
                info!("Received reload commands request");
                // TODO: implement reload
            }
            IpcAction::SetMuted { muted } => {
                info!("Received mute request: {}", muted);
                // TODO: implement mute
            }
            IpcAction::TextCommand { text } => {
                info!("Received text command: {}", text);
                if let Err(e) = text_cmd_tx.send(text) {
                    error!("Failed to send text command to app: {}", e);
                }
            }
            IpcAction::Ping => {
                // handled internally by server
            }
            _ => {}
        }
    });

    // start WebSocket server on the shared runtime
    let ipc_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        ipc_rt.block_on(ipc::start_server());
    });
    
    // start the app (in the background thread)
    let app_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = app::start(text_cmd_rx, &app_rt);
        // stopped from the GUI: leave the tray too, or the process lives on without listening
        info!("Main loop finished, exiting.");
        std::process::exit(0);
    });

    tray::init_blocking(settings);

    Ok(())
}

// a named mutex held until the process exits; a restarting copy waits for the old one to quit
#[cfg(windows)]
fn single_instance() -> bool {
    use winapi::um::synchapi::{CreateMutexW, WaitForSingleObject};
    use winapi::um::winbase::{WAIT_ABANDONED, WAIT_OBJECT_0};
    let name: Vec<u16> = "Local\\JarvisVoiceAssistantApp".encode_utf16().chain(Some(0)).collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
        if handle.is_null() {
            return true; // cannot tell, better run than not
        }
        let r = WaitForSingleObject(handle, 8000);
        r == WAIT_OBJECT_0 || r == WAIT_ABANDONED
    }
}

#[cfg(not(windows))]
fn single_instance() -> bool {
    true
}

pub fn should_stop() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}
