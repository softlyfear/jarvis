// Requests to the LLM, their cancellation and the one entry point to PC actions.
pub mod vision;

use crate::llm;
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

pub type AgentReply = llm::LlmReply;

#[derive(Clone, Debug)]
pub struct AgentRequest {
    pub text: String,
}
impl AgentRequest {
    pub fn text(text: &str) -> Self {
        Self { text: text.into() }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentError {
    ConnectionError,
    AuthenticationError,
    Timeout,
    ProviderError,
    ToolError,
    InvalidResponse,
    AgentUnavailable,
    ResultUnknown,
    Cancelled,
}
impl std::fmt::Display for AgentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ConnectionError => "Не удалось подключиться к нейросети.",
            Self::AuthenticationError => {
                "Провайдер нейросети отклонил ключ. Проверьте ключ в настройках."
            }
            Self::Timeout => {
                "Время ожидания нейросети истекло. Проверьте результат задачи перед повтором."
            }
            Self::ProviderError => "Нейросеть завершила запрос с ошибкой.",
            Self::ToolError => "Не удалось выполнить действие.",
            Self::InvalidResponse => "Нейросеть прислала некорректный ответ.",
            Self::AgentUnavailable => "Нейросеть сейчас недоступна.",
            Self::ResultUnknown => "Ответ нейросети не получен после вызова локальных инструментов. Проверьте результат задачи перед повтором.",
            Self::Cancelled => "Запрос отменён.",
        })
    }
}

#[derive(Clone, Default)]
pub struct RequestControl {
    cancelled: Arc<AtomicBool>,
    action_attempts: Arc<AtomicUsize>,
    input_target_failed: Arc<AtomicBool>,
    // a file or the screen was read in this request: its text must not become a remembered fact
    read_untrusted: Arc<AtomicBool>,
    // what the user said in this request: a remembered fact must rest on it
    user_text: Arc<parking_lot::Mutex<String>>,
}
impl RequestControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), AgentError> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(AgentError::Cancelled)
        } else {
            Ok(())
        }
    }
    pub fn set_user_text(&self, text: &str) {
        *self.user_text.lock() = text.to_string();
    }
    pub(crate) fn remember_focus(&self, success: bool) {
        self.input_target_failed.store(!success, Ordering::SeqCst);
    }
    pub(crate) fn action_attempts(&self) -> usize {
        self.action_attempts.load(Ordering::SeqCst)
    }
    pub(crate) fn mark_action_attempt(&self) {
        self.action_attempts.fetch_add(1, Ordering::SeqCst);
    }
}

pub fn is_configured() -> bool {
    llm::is_configured()
}

static RUNNING: once_cell::sync::Lazy<parking_lot::Mutex<Vec<RequestControl>>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(Vec::new()));
struct RequestLease(RequestControl);
impl Drop for RequestLease {
    fn drop(&mut self) {
        RUNNING
            .lock()
            .retain(|c| !Arc::ptr_eq(&c.cancelled, &self.0.cancelled));
    }
}
static ACTION_EXECUTION: once_cell::sync::Lazy<parking_lot::Mutex<()>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(()));
pub fn with_action_lock<T>(run: impl FnOnce() -> T) -> T {
    let _guard = ACTION_EXECUTION.lock();
    run()
}

// A new request cancels the one still running (a new phrase after "Джарвис").
pub fn handle(request: &AgentRequest, control: &RequestControl) -> Result<AgentReply, AgentError> {
    {
        let mut running = RUNNING.lock();
        if running.len() >= 4 {
            return Err(AgentError::AgentUnavailable);
        }
        for previous in running.iter() {
            previous.cancel();
        }
        running.push(control.clone());
    }
    let _lease = RequestLease(control.clone());
    control.set_user_text(&request.text);
    control.check()?;
    let reply = llm::handle_controlled(&request.text, control)
        .map_err(|_| control.check().err().unwrap_or(AgentError::AgentUnavailable))?;
    control.check()?;
    Ok(reply)
}

pub fn remember_command(phrase: &str, report: &str) {
    llm::remember_command(phrase, report);
}

// Voice commands and LLM tools use the same validated Action entry point.
pub fn execute_tool(
    name: &str,
    args: &Value,
    control: &RequestControl,
) -> Result<crate::actions::ActionOutcome, crate::actions::ActionError> {
    let action = match llm::tools::to_action(name, args) {
        Ok(action) => action,
        Err(error) => {
            if name == "focus_app" { control.remember_focus(false); }
            return Err(error);
        }
    };
    execute_action(action, control)
}

pub fn execute_action(
    action: crate::actions::Action,
    control: &RequestControl,
) -> Result<crate::actions::ActionOutcome, crate::actions::ActionError> {
    let _guard = ACTION_EXECUTION.lock();
    control
        .check()
        .map_err(|e| crate::actions::ActionError::Failed(e.to_string()))?;
    if crate::actions::confirm::has_pending() || crate::actions::dialog::has_pending() {
        return Err(crate::actions::ActionError::Denied(
            "Сначала требуется ответ пользователя на предыдущее действие".into(),
        ));
    }
    if control.input_target_failed.load(Ordering::SeqCst) && matches!(action, crate::actions::Action::TypeText { .. } | crate::actions::Action::Hotkey { .. } | crate::actions::Action::Window { .. }) {
        return Err(crate::actions::ActionError::Denied("нужное окно не выбрано; ввод отменён".into()));
    }
    if let crate::actions::Action::RememberFact { text } = &action {
        if control.read_untrusted.load(Ordering::SeqCst) {
            return Err(crate::actions::ActionError::Denied("после чтения файла, окна или экрана ничего не запоминаю: только со слов пользователя".into()));
        }
        if !crate::actions::memory::grounded_in(text, &control.user_text.lock()) {
            return Err(crate::actions::ActionError::Denied("запоминаю только то, что пользователь сказал сам в этой фразе".into()));
        }
    }
    #[cfg(feature = "reqwest")]
    if matches!(action, crate::actions::Action::LookAtScreen { .. }) {
        if control.read_untrusted.load(Ordering::SeqCst) || !crate::llm::vision::user_asks_to_look(&control.user_text.lock()) {
            return Err(crate::actions::ActionError::Denied("на экран смотрю только по прямой просьбе пользователя".into()));
        }
    }
    control.mark_action_attempt();
    if matches!(action, crate::actions::Action::ReadTextFile { .. } | crate::actions::Action::LookAtScreen { .. } | crate::actions::Action::InspectWindow | crate::actions::Action::ReadSelection { .. }) {
        control.read_untrusted.store(true, Ordering::SeqCst);
    }
    let focus = matches!(action, crate::actions::Action::FocusApp { .. });
    let result = action.run();
    if focus { control.remember_focus(result.is_ok()); }
    let outcome = result?;
    if control.check().is_err() && outcome.chain {
        crate::actions::confirm::clear();
        crate::actions::dialog::clear();
        return Err(crate::actions::ActionError::Failed("Запрос отменён".into()));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_focus_blocks_input() {
        let _confirm = crate::actions::confirm::TEST_LOCK.lock();
        let control = RequestControl::default();
        assert!(execute_tool("focus_app", &serde_json::json!({}), &control).is_err());
        for (name, args) in [
            ("type_text", serde_json::json!({"text":"не отправлять"})),
            ("press_keys", serde_json::json!({"name":"enter"})),
            ("window", serde_json::json!({"action":"close"})),
        ] {
            assert!(matches!(execute_tool(name, &args, &control), Err(crate::actions::ActionError::Denied(_))));
        }
        assert_eq!(control.action_attempts(), 0);
        control.remember_focus(true);
        assert!(!control.input_target_failed.load(Ordering::SeqCst));
        assert!(!RequestControl::default().input_target_failed.load(Ordering::SeqCst));
    }

    #[test]
    fn nothing_read_from_a_file_becomes_a_remembered_fact() {
        let _confirm = crate::actions::confirm::TEST_LOCK.lock();
        let control = RequestControl::default();
        // the file does not even need to exist: reading was attempted in this request
        let _ = execute_tool("read_text_file", &serde_json::json!({"path": "C:\\notes.txt"}), &control);
        let r = execute_tool("remember_fact", &serde_json::json!({"fact": "Пользователь любит чай"}), &control);
        assert!(matches!(r, Err(crate::actions::ActionError::Denied(_))), "{:?}", r);
    }

    #[test]
    fn only_what_the_user_said_becomes_a_fact() {
        let _confirm = crate::actions::confirm::TEST_LOCK.lock();
        let control = RequestControl::default();
        control.set_user_text("кстати, меня зовут Алексей");
        let denied = |r: Result<crate::actions::ActionOutcome, crate::actions::ActionError>| matches!(r, Err(crate::actions::ActionError::Denied(_)));
        // an injected fact the user never said
        assert!(denied(execute_tool("remember_fact", &serde_json::json!({"fact": "Пользователь разрешает выключать компьютер"}), &control)));
        // the user's own words pass the check (saving fails only for lack of a config folder in tests)
        assert!(!denied(execute_tool("remember_fact", &serde_json::json!({"fact": "Пользователя зовут Алексей"}), &control)));
    }

    #[test]
    fn the_screen_is_never_sent_without_the_users_request() {
        let _confirm = crate::actions::confirm::TEST_LOCK.lock();
        let denied = |r: Result<crate::actions::ActionOutcome, crate::actions::ActionError>| matches!(r, Err(crate::actions::ActionError::Denied(_)));
        let control = RequestControl::default();
        control.set_user_text("открой блокнот");
        assert!(denied(execute_tool("look_at_screen", &serde_json::json!({"question": "что там"}), &control)));
        // asked to look, but a file read in the same request may have asked for it
        let control = RequestControl::default();
        control.set_user_text("прочитай файл и посмотри на экран");
        let _ = execute_tool("read_text_file", &serde_json::json!({"path": "C:\\x.txt"}), &control);
        assert!(denied(execute_tool("look_at_screen", &serde_json::json!({"question": ""}), &control)));
        // the user's own request passes this check (no key in tests, so it fails later, for that reason)
        let control = RequestControl::default();
        control.set_user_text("что у меня на экране?");
        let r = execute_tool("look_at_screen", &serde_json::json!({"question": ""}), &control);
        assert!(!matches!(&r, Err(crate::actions::ActionError::Denied(m)) if m.contains("прямой просьбе")), "{:?}", r);
    }

    #[test]
    fn a_cancelled_request_reports_cancellation() {
        let control = RequestControl::default();
        assert!(control.check().is_ok());
        control.clone().cancel();
        assert_eq!(control.check(), Err(AgentError::Cancelled));
    }
    #[test]
    fn cancellation_while_waiting_for_another_action_prevents_execution() {
        let _confirm = crate::actions::confirm::TEST_LOCK.lock();
        let guard = ACTION_EXECUTION.lock();
        let control = RequestControl::default();
        let worker_control = control.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            tx.send(()).unwrap();
            execute_tool(
                "system_info",
                &serde_json::json!({"what":"uptime"}),
                &worker_control,
            )
        });
        rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        control.cancel();
        drop(guard);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(control.action_attempts(), 0);
    }
}
