use jarvis_core::{agent, agent_config::AgentConfig, assistant_config::EditableSettings};

#[tauri::command]
pub async fn agent_setup(settings: EditableSettings) -> Result<AgentConfig, String> {
    tauri::async_runtime::spawn_blocking(move || agent::managed::install(&settings))
        .await
        .map_err(|_| "Не удалось настроить OpenClaw.".to_string())?
}

#[tauri::command]
pub async fn agent_models(settings: AgentConfig) -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(move || agent::managed::models(&settings))
        .await
        .map_err(|_| "Не удалось прочитать модели.".to_string())?
}

#[tauri::command]
pub fn agent_open_profile(settings: AgentConfig) -> Result<(), String> {
    let path = agent::managed::config_path(&settings)?;
    std::process::Command::new(if cfg!(windows) {
        "notepad.exe"
    } else {
        "xdg-open"
    })
    .arg(path)
    .spawn()
    .map(|_| ())
    .map_err(|_| "Не удалось открыть профиль OpenClaw.".into())
}

#[tauri::command]
pub async fn agent_open_dashboard(settings: AgentConfig) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || agent::managed::open_dashboard(&settings))
        .await
        .map_err(|_| "Не удалось открыть настройки OpenClaw.".to_string())?
}
