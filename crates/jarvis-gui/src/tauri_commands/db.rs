use crate::AppState;

#[tauri::command]
pub fn db_read(state: tauri::State<'_, AppState>, key: &str) -> String {
    state.settings.read(key).unwrap_or_default()
}

#[tauri::command]
pub fn db_write(state: tauri::State<'_, AppState>, key: &str, val: &str) -> Result<(), String> {
    state.settings.write(key, val)
}

#[tauri::command]
pub fn db_write_many(state: tauri::State<'_, AppState>, values: Vec<(String, String)>) -> Result<(), String> {
    let pairs: Vec<(&str, &str)> = values.iter().map(|(key, val)| (key.as_str(), val.as_str())).collect();
    state.settings.write_many(&pairs)
}
