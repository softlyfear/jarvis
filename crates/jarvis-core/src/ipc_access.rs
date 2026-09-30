// Browser clients must belong to the GUI; native clients do not send Origin.
pub(crate) fn allowed_origin(origin: Option<&str>) -> bool {
    matches!(origin, None | Some("tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost" | "http://localhost:1420" | "http://127.0.0.1:1420"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_native_clients_and_exact_gui_origins_are_allowed() {
        for origin in [None, Some("tauri://localhost"), Some("http://tauri.localhost"), Some("https://tauri.localhost"), Some("http://localhost:1420"), Some("http://127.0.0.1:1420")] {
            assert!(allowed_origin(origin));
        }
        for origin in ["null", "", "https://example.com", "http://localhost:14200", "http://tauri.localhost.example.com", "http://localhost:1420/path"] {
            assert!(!allowed_origin(Some(origin)));
        }
    }
}
