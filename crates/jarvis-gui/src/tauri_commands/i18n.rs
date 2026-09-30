use jarvis_core::i18n;
use std::collections::HashMap;
use crate::AppState;

// Get all translations for frontend
#[tauri::command]
pub fn get_translations() -> HashMap<String, String> {
    i18n::get_all_translations()
}

// Get single translation
#[tauri::command]
pub fn translate(key: &str) -> String {
    i18n::t(key)
}

// Get current language
#[tauri::command]
pub fn get_current_language() -> String {
    i18n::get_language()
}

// Set language and get new translations
#[tauri::command]
pub fn set_language(state: tauri::State<'_, AppState>, lang: &str) -> Result<HashMap<String, String>, String> {
    // update i18n
    state.settings.write("language", lang)?;
    i18n::set_language(lang);

        // return new translations
    Ok(i18n::get_all_translations())
}

// Get supported languages
#[tauri::command]
pub fn get_supported_languages() -> Vec<&'static str> {
    i18n::SUPPORTED_LANGUAGES.to_vec()
}
