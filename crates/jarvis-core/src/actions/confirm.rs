// One pending dangerous action waiting for a spoken "yes" / "no".

use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use super::text::normalize;
use super::Action;
use crate::assistant_config;

static PENDING: Lazy<Mutex<Option<(Action, Instant)>>> = Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

const YES: &[&str] = &[
    "да", "давай", "подтверждаю", "подтверди", "конечно", "ага", "угу", "выполняй", "делай",
    "удаляй", "выключай", "перезагружай", "согласен", "согласна", "верно", "yes", "ok", "окей",
];
const NO: &[&str] = &["нет", "не", "отмена", "отмени", "стоп", "не надо", "отставить", "no", "cancel"];

#[derive(Debug, PartialEq)]
pub enum Answer {
    NoPending,
    Confirmed(Action),
    Cancelled,
    // pending action dropped, the phrase is a new command
    Unrelated,
}

pub fn request(action: Action) {
    info!("Waiting for confirmation: {:?}", action);
    *PENDING.lock() = Some((action, Instant::now()));
}

pub fn clear() {
    *PENDING.lock() = None;
}

pub fn has_pending() -> bool {
    let timeout = Duration::from_secs(assistant_config::get().safety.confirm_timeout_secs);
    matches!(PENDING.lock().as_ref(), Some((_, at)) if at.elapsed() < timeout)
}

fn classify(text: &str) -> Option<bool> {
    let t = normalize(text);
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 {
        return None;
    }
    // "no" wins: "да нет", "не надо"
    if words.iter().any(|w| NO.contains(w)) || NO.iter().any(|n| n.contains(' ') && t.contains(n)) {
        return Some(false);
    }
    if words.iter().any(|w| YES.contains(w)) {
        return Some(true);
    }
    None
}

pub fn answer(text: &str) -> Answer {
    let timeout = Duration::from_secs(assistant_config::get().safety.confirm_timeout_secs);
    let mut pending = PENDING.lock();
    let Some((action, at)) = pending.take() else {
        return Answer::NoPending;
    };
    if at.elapsed() >= timeout {
        info!("Confirmation expired for {:?}", action);
        return Answer::NoPending;
    }
    match classify(text) {
        Some(true) => Answer::Confirmed(action),
        Some(false) => Answer::Cancelled,
        None => Answer::Unrelated,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_no_classification() {
        assert_eq!(classify("да"), Some(true));
        assert_eq!(classify("Да, давай"), Some(true));
        assert_eq!(classify("нет"), Some(false));
        assert_eq!(classify("не надо"), Some(false));
        assert_eq!(classify("да нет наверное"), Some(false));
        assert_eq!(classify("открой браузер"), None);
    }

    #[test]
    fn answer_consumes_pending_action() {
        let _guard = TEST_LOCK.lock();
        clear();
        assert_eq!(answer("да"), Answer::NoPending);

        request(Action::Restart);
        assert!(has_pending());
        assert_eq!(answer("да"), Answer::Confirmed(Action::Restart));
        assert!(!has_pending());

        request(Action::Restart);
        assert_eq!(answer("открой браузер"), Answer::Unrelated);
        assert_eq!(answer("да"), Answer::NoPending);
    }
}
