use jarvis_core::{vosk_models, DB};
use serde::Serialize;

#[derive(Serialize)]
pub struct VoskModel {
    pub name: String,
    pub language: String,
    pub size: String,
}

#[tauri::command]
pub fn list_vosk_models() -> Vec<VoskModel> {
    vosk_models::scan_vosk_models()
        .into_iter()
        .map(|m| VoskModel {
            name: m.name,
            language: m.language,
            size: m.size,
        })
        .collect()
}