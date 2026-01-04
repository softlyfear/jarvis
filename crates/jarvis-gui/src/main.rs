// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use jarvis_core::{config, db, APP_CONFIG_DIR, APP_LOG_DIR, DB};

#[macro_use]
extern crate simple_log;

mod events;

// mod tauri_commands;

fn main() {
    config::init_dirs().expect("Failed to init dirs");
    
    // basic logging setup (simpler for GUI)
    simple_log::quick!("info");

    let _ = DB.set(db::init_settings());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .invoke_handler(tauri::generate_handler![
            // commands will be added here after tauri_commands module is fixed
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}