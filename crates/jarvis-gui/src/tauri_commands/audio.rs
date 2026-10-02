use jarvis_core::recorder;

#[tauri::command]
pub fn pv_get_audio_devices() -> Vec<String> {
    recorder::get_audio_devices()
}

#[tauri::command]
pub fn pv_get_audio_device_name(idx: i32) -> String {
     recorder::get_audio_device_name(idx)
}
