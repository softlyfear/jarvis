// Backend-independent requests, cancellation and events for the voice shell.
pub mod bridge;
pub mod mcp;
pub mod openclaw;
pub mod vision;

use crate::{
    agent_config::{AgentConfig, BackendKind},
    assistant_config, llm,
};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

pub type AgentReply = llm::LlmReply;

#[derive(Clone, Debug)]
pub struct AgentRequest {
    pub text: String,
    pub continuation: bool,
}
impl AgentRequest {
    pub fn text(text: &str) -> Self {
        Self {
            text: text.into(),
            continuation: false,
        }
    }
    pub fn continuation(report: &str) -> Self {
        Self {
            text: report.into(),
            continuation: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum AgentEvent {
    TextDelta(String),
    ToolCall { name: String },
    ToolResult { name: String, success: bool },
    BackendChanged,
    Done,
    Error(AgentError),
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
                "OpenClaw или провайдер отклонил авторизацию. Проверьте настройки подключения."
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
    parent: Option<Arc<RequestControl>>,
    action_attempts: Arc<AtomicUsize>,
}
impl RequestControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<(), AgentError> {
        if self.cancelled.load(Ordering::SeqCst) {
            Err(AgentError::Cancelled)
        } else {
            self.parent.as_ref().map_or(Ok(()), |p| p.check())
        }
    }
    fn child(&self) -> Self {
        Self {
            cancelled: Arc::default(),
            parent: Some(Arc::new(self.clone())),
            action_attempts: self.action_attempts.clone(),
        }
    }
    pub(crate) fn action_attempts(&self) -> usize {
        self.action_attempts.load(Ordering::SeqCst)
    }
    pub(crate) fn mark_action_attempt(&self) {
        self.action_attempts.fetch_add(1, Ordering::SeqCst);
    }
}

pub trait AgentBackend {
    fn handle(
        &self,
        request: &AgentRequest,
        control: &RequestControl,
        emit: &dyn Fn(AgentEvent),
    ) -> Result<AgentReply, AgentError>;
}

pub struct DirectBackend;
impl AgentBackend for DirectBackend {
    fn handle(
        &self,
        request: &AgentRequest,
        control: &RequestControl,
        emit: &dyn Fn(AgentEvent),
    ) -> Result<AgentReply, AgentError> {
        control.check()?;
        let text = if request.continuation {
            format!("Результат подтверждённого действия: {}", request.text)
        } else {
            request.text.clone()
        };
        let reply = llm::handle_controlled(&text, control).map_err(|_| {
            control
                .check()
                .err()
                .unwrap_or(AgentError::AgentUnavailable)
        })?;
        control.check()?;
        emit(AgentEvent::Done);
        Ok(reply)
    }
}

pub fn is_configured() -> bool {
    assistant_config::get().agent.backend == BackendKind::Openclaw || llm::is_configured()
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
// Internal MCP calls do not carry a voice request ID. Fail closed until a cancelled run exits.
pub fn mcp_blocked() -> bool {
    RUNNING.lock().iter().any(|c| c.check().is_err())
}
// Keep the parent token alive after its HTTP worker exits, including queued MCP jobs.
fn mcp_request_control() -> RequestControl {
    RUNNING
        .lock()
        .last()
        .map(RequestControl::child)
        .unwrap_or_default()
}

static ACTION_EXECUTION: once_cell::sync::Lazy<parking_lot::Mutex<()>> =
    once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(()));
pub fn with_action_lock<T>(run: impl FnOnce() -> T) -> T {
    let _guard = ACTION_EXECUTION.lock();
    run()
}

pub fn handle(
    request: &AgentRequest,
    control: &RequestControl,
    emit: &dyn Fn(AgentEvent),
) -> Result<AgentReply, AgentError> {
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
    let cfg = &assistant_config::get().agent;
    let result = match cfg.backend {
        BackendKind::Direct => DirectBackend.handle(request, control, emit),
        BackendKind::Openclaw => {
            openclaw::OpenClawBackend::new(cfg.clone()).handle(request, control, emit)
        }
    };
    control.check()?;
    result
}

pub fn remember_command(phrase: &str, report: &str) {
    llm::remember_command(phrase, report);
    if assistant_config::get().agent.backend == BackendKind::Openclaw {
        openclaw::remember_command(phrase, report);
    }
}

pub fn has_pending() -> bool {
    openclaw::has_pending()
}
pub fn abandon_pending() {
    openclaw::abandon_pending();
}

// Both backends and MCP use the same validated Action entry point.
pub fn execute_tool(
    name: &str,
    args: &Value,
    control: &RequestControl,
) -> Result<crate::actions::ActionOutcome, crate::actions::ActionError> {
    execute_action(llm::tools::to_action(name, args)?, control)
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
    control.mark_action_attempt();
    let outcome = action.run()?;
    if control.check().is_err() && outcome.chain {
        crate::actions::confirm::clear();
        crate::actions::dialog::clear();
        return Err(crate::actions::ActionError::Failed("Запрос отменён".into()));
    }
    Ok(outcome)
}

pub fn check_connection(cfg: &AgentConfig) -> Result<openclaw::ConnectionStatus, AgentError> {
    openclaw::check_connection(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn child_cancellation_is_independent_but_parent_cancellation_propagates() {
        let parent = RequestControl::default();
        let first = parent.child();
        let second = parent.child();
        first.cancel();
        assert!(first.check().is_err());
        assert!(parent.check().is_ok() && second.check().is_ok());
        second.mark_action_attempt();
        assert_eq!(parent.action_attempts(), 1);
        parent.cancel();
        assert_eq!(second.check(), Err(AgentError::Cancelled));
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
