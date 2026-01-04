use once_cell::sync::{Lazy, OnceCell};
use platform_dirs::AppDirs;
use std::path::PathBuf;

#[macro_use]
extern crate log;

pub mod audio;
pub mod commands;
pub mod config;
pub mod db;
pub mod listener;
pub mod recorder;
pub mod stt;

// shared statics
pub static APP_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::current_dir().unwrap());
pub static SOUND_DIR: Lazy<PathBuf> = Lazy::new(|| APP_DIR.clone().join("sound"));
pub static APP_DIRS: OnceCell<AppDirs> = OnceCell::new();
pub static APP_CONFIG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static APP_LOG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static DB: OnceCell<db::structs::Settings> = OnceCell::new();
pub static COMMANDS_LIST: OnceCell<Vec<commands::AssistantCommand>> = OnceCell::new();

// re-exports
pub use commands::AssistantCommand;
pub use config::structs::*;
pub use db::structs::Settings;