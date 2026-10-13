// Spoken replies: keep error reasons direct and avoid the assistant's own wake word.

pub fn failure(message: &str) -> String {
    let mut text = message.trim().trim_end_matches('.').trim();
    loop {
        let lower = text.to_lowercase();
        let prefix = ["не получилось выполнить действие:", "не получилось:", "ошибка:", "не найдено:"]
            .into_iter().find(|prefix| lower.starts_with(prefix));
        match prefix {
            Some(prefix) => text = text[prefix.len()..].trim(),
            None => break,
        }
    }
    if text.is_empty() { return "Команда не сработала.".into(); }
    let mut chars = text.chars();
    let first = chars.next().unwrap().to_uppercase().collect::<String>();
    let text = first + chars.as_str();
    if text.ends_with(['.', '?', '!']) { text } else { text + "." }
}

pub fn without_wake_name(text: &str) -> String {
    fn word(text: &str) -> String {
        match text.to_lowercase().as_str() {
            "jarvis" | "джарвис" => "ассистент".into(),
            "джарвиса" => "ассистента".into(),
            "джарвису" => "ассистенту".into(),
            "джарвисом" => "ассистентом".into(),
            "джарвисе" => "ассистенте".into(),
            _ => text.into(),
        }
    }
    let mut out = String::new();
    let mut token = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            token.push(c);
        } else {
            out.push_str(&word(&token));
            token.clear();
            out.push(c);
        }
    }
    out.push_str(&word(&token));
    out
}

// A single unstable partial must not cut off speech; a finalized wake remains immediate.
#[derive(Default)]
pub struct PartialWakeGuard { samples: usize }

impl PartialWakeGuard {
    pub fn update(&mut self, matched: bool, finalized: bool, frame_samples: usize) -> bool {
        if !matched { self.samples = 0; return false; }
        if finalized { self.samples = 0; return true; }
        self.samples = self.samples.saturating_add(frame_samples);
        self.samples >= 16000 / 5
    }
    pub fn reset(&mut self) { self.samples = 0; }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn errors_say_the_reason_without_nested_preambles() {
        assert_eq!(failure("Не получилось выполнить действие: ошибка: не нашёл программу «Steam»."), "Не нашёл программу «Steam».");
        assert_eq!(failure("Открытого окна нет."), "Открытого окна нет.");
        assert_eq!(failure("Доступ запрещён: папка вне разрешённых каталогов"), "Доступ запрещён: папка вне разрешённых каталогов.");
        assert_eq!(failure("Какую программу открыть?"), "Какую программу открыть?");
    }
    #[test]
    fn the_spoken_wake_name_is_replaced_without_touching_other_names() {
        assert_eq!(without_wake_name("Панель JARVIS Voice Assistant. Настройки Джарвиса."), "Панель ассистент Voice Assistant. Настройки ассистента.");
        assert_eq!(without_wake_name("JarvisTools, Steam, API, джарвис_тест"), "JarvisTools, Steam, API, джарвис_тест");
    }
    #[test]
    fn partial_wakes_need_stability_but_real_final_wakes_are_not_delayed() {
        let mut gate = PartialWakeGuard::default();
        assert!(!gate.update(true, false, 512));
        assert!(!gate.update(false, false, 512));
        for _ in 0..6 { assert!(!gate.update(true, false, 512)); }
        assert!(gate.update(true, false, 512));
        gate.reset();
        assert!(gate.update(true, true, 512));
        assert!(!gate.update(false, true, 512));
    }
}
