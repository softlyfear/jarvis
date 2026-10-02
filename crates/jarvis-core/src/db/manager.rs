use std::sync::Arc;
use parking_lot::RwLock;

use super::structs::Settings;
use super::save_settings;

// centralized settings manager.
// wraps Arc<RwLock<Settings>> and handles locking + auto-save
// can be used anywhere, ex. from GUI, tray, IPC, CLI, etc.
#[derive(Clone)]
pub struct SettingsManager {
    inner: Arc<RwLock<Settings>>,
}

impl SettingsManager {
    pub fn new(settings: Settings) -> Self {
        Self {
            inner: Arc::new(RwLock::new(settings)),
        }
    }

    // wrap an existing Arc<RwLock<Settings>>
    pub fn from_arc(arc: Arc<RwLock<Settings>>) -> Self {
        Self { inner: arc }
    }

    // read a setting by key
    pub fn read(&self, key: &str) -> Option<String> {
        self.inner.read().get(key)
    }

    // write a setting by key, auto-saves to disk
    pub fn write(&self, key: &str, val: &str) -> Result<(), String> {
        self.write_many(&[(key, val)])
    }

    // write multiple settings at once, single save
    pub fn write_many(&self, pairs: &[(&str, &str)]) -> Result<(), String> {
        self.update_with(pairs, |s| save_settings(s).map_err(|e| format!("failed to save settings: {}", e)))
    }

    fn update_with(&self, pairs: &[(&str, &str)], save: impl FnOnce(&Settings) -> Result<(), String>) -> Result<(), String> {
        // Serialize validation, disk replacement and publication. Failed validation or
        // IO must not change live settings, or race a newer save with an older snapshot.
        let mut settings = self.inner.write();
        let mut next = settings.clone();
        for (key, val) in pairs {
            next.set(key, val)?;
        }
        save(&next)?;
        *settings = next;
        Ok(())
    }

    // direct read access to the full Settings struct (for init code that
    // needs to read multiple fields at once without key-based access)
    pub fn lock(&self) -> parking_lot::RwLockReadGuard<'_, Settings> {
        self.inner.read()
    }

    // direct write access (for bulk operations not covered by set())
    pub fn lock_mut(&self) -> parking_lot::RwLockWriteGuard<'_, Settings> {
        self.inner.write()
    }

    // get the underlying Arc
    pub fn arc(&self) -> &Arc<RwLock<Settings>> {
        &self.inner
    }

    // dump all settings as key-value pairs (for debugging)
    pub fn dump(&self) -> Vec<(String, String)> {
        let settings = self.inner.read();
        Settings::keys().iter()
            .filter_map(|&key| {
                settings.get(key).map(|val| (key.to_string(), if key.starts_with("api_key") && !val.is_empty() { "[скрыто]".into() } else { val }))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_batches_and_failed_saves_leave_live_settings_unchanged() {
        let manager = SettingsManager::new(Settings::default());
        let old = manager.read("selected_vosk_model");
        assert!(manager.update_with(&[("selected_vosk_model", "new"), ("unknown", "x")], |_| panic!("must not save")).is_err());
        assert_eq!(manager.read("selected_vosk_model"), old);
        assert!(manager.update_with(&[("selected_vosk_model", "new")], |_| Err("disk full".into())).is_err());
        assert_eq!(manager.read("selected_vosk_model"), old);
        manager.update_with(&[("selected_vosk_model", "new")], |_| Ok(())).unwrap();
        assert_eq!(manager.read("selected_vosk_model").as_deref(), Some("new"));
    }
    #[test]
    fn diagnostics_hide_keys_and_all_advertised_settings_are_writable() {
        let mut settings = Settings::default();
        settings.set("api_key__picovoice", "arbitrary-secret").unwrap();
        for key in Settings::keys() { settings.set(key, &settings.get(key).unwrap()).unwrap(); }
        let manager = SettingsManager::new(settings);
        assert_eq!(manager.read("api_key__picovoice").as_deref(), Some("arbitrary-secret"));
        assert!(!format!("{:?}", manager.dump()).contains("arbitrary-secret"));
    }
}
