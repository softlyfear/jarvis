// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::{config, db, i18n, voices, DB, SettingsManager};

#[macro_use]
extern crate simple_log;

mod events;

mod tauri_commands;

#[derive(Clone)]
pub struct AppState {
    pub settings: SettingsManager,
}

fn main() {
    // one window: a second launch (tray click, shortcut) brings the open one to the front
    if jarvis_core::actions::platform::focus_window(jarvis_core::actions::platform::GUI_WINDOW_TITLES) {
        return;
    }

    config::init_dirs().expect("Failed to init dirs");
    
    // GUI log next to Jarvis's own log: clicks, navigation and UI errors end up here
    let gui_log = jarvis_core::APP_LOG_DIR
        .get()
        .map(|d| d.join("gui-log.txt"))
        .unwrap_or_else(|| std::path::PathBuf::from("gui-log.txt"));
    let log_config = simple_log::LogConfigBuilder::builder()
        .path(gui_log.to_string_lossy().to_string())
        .size(10)
        .roll_count(3)
        .time_format("%Y-%m-%d %H:%M:%S.%f")
        .level("debug")
        .and_then(|b| Ok(b.output_file().output_console().build()));
    match log_config {
        Ok(c) => { let _ = simple_log::new(c); }
        Err(_) => { let _ = simple_log::quick!("info"); }
    }
    info!("Jarvis GUI v{} (build {})", config::APP_VERSION.unwrap_or("?"), option_env!("JARVIS_BUILD").unwrap_or("local"));

    // init settings
    let manager = db::init();

    // init i18n
    i18n::init(&manager.lock().language);

    // init voices
    if let Err(e) = voices::init(&manager.lock().voice, &manager.lock().language) {
        eprintln!("Failed to init voices: {}", e);
    }

    // init audio backend
    if let Err(e) = jarvis_core::audio::init() {
        eprintln!("Failed to init audio: {:?}", e);
    }

    // set global DB (for core modules that read settings at init time)
    DB.set(manager.arc().clone())
            .expect("DB already initialized");

    tauri::Builder::default()
        .manage(AppState { settings: manager })
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // audio
            tauri_commands::pv_get_audio_devices,
            tauri_commands::pv_get_audio_device_name,
            tauri_commands::play_sound,

            // db
            tauri_commands::db_read,
            tauri_commands::db_write,

            // etc
            tauri_commands::get_app_version,
            tauri_commands::get_author_name,
            tauri_commands::get_repository_link,
            tauri_commands::get_tg_official_link,
            tauri_commands::get_boosty_link,
            tauri_commands::get_patreon_link,
            tauri_commands::get_feedback_link,

            // fs
            tauri_commands::get_log_file_path,
            tauri_commands::show_in_folder,

            // sys
            tauri_commands::get_current_ram_usage,
            tauri_commands::get_peak_ram_usage,
            tauri_commands::get_cpu_temp,
            tauri_commands::get_cpu_usage,
            tauri_commands::get_jarvis_app_stats,
            tauri_commands::is_jarvis_app_running,
            tauri_commands::run_jarvis_app,
            tauri_commands::stop_jarvis_app,
            tauri_commands::restart_jarvis_app,

            // vosk
            tauri_commands::list_vosk_models,

            // gliner
            tauri_commands::list_gliner_models,

            // i18n
            tauri_commands::get_translations,
            tauri_commands::translate,
            tauri_commands::get_current_language,
            tauri_commands::set_language,
            tauri_commands::get_supported_languages,

            // commands
            tauri_commands::get_commands_count,
            tauri_commands::get_commands_list,

            // fork: assistant.toml, voice server, command packs
            tauri_commands::assistant_settings_read,
            tauri_commands::assistant_settings_write,
            tauri_commands::open_assistant_config,
            tauri_commands::voice_server_status,
            tauri_commands::get_command_packs,
            tauri_commands::ui_log,
            tauri_commands::collect_logs,
            tauri_commands::check_update,
            tauri_commands::install_update,
            tauri_commands::update_status,

            // voices
            tauri_commands::list_voices,
            tauri_commands::get_voice,
            tauri_commands::preview_voice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
