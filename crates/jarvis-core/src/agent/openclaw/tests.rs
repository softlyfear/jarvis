use super::*;
use crate::actions::{Action, ActionError, ActionOutcome};
use std::{
    cell::RefCell,
    io::{BufRead, Read, Write},
    net::TcpListener,
    sync::Arc,
};
static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[test]
#[ignore = "Requires the isolated real Gateway harness; no user configuration is used."]
fn real_gateway_client_tool_contract() {
    let _guard = TEST_LOCK.lock();
    reset();
    let path = std::env::var("JARVIS_GATEWAY_TEST_PROFILE").expect("isolated test profile");
    let profile: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let mut config = AgentConfig::default();
    config.backend = crate::agent_config::BackendKind::Openclaw;
    config.fallback_backend = "none".into();
    config.openclaw.base_url = format!("http://127.0.0.1:{}", profile["gateway"]["port"].as_u64().unwrap());
    config.openclaw.api_key = profile["gateway"]["auth"]["token"].as_str().unwrap().into();
    config.openclaw.streaming = true;
    let reply = OpenClawBackend::new(config).handle(&AgentRequest::text("CASE:client"), &RequestControl::default(), &|_| {}).unwrap();
    assert!(reply.success && reply.acted && reply.chain);
    assert!(reply.speech.contains("Таймеров нет."), "{}", reply.speech);
}
fn reset() {
    *SESSION.lock() = None;
    *PENDING.lock() = None;
}
struct Mock {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    thread: std::thread::JoinHandle<()>,
}
fn mock(responses: Vec<(u16, String)>) -> Mock {
    mock_delayed(responses, Duration::ZERO)
}
fn mock_delayed(responses: Vec<(u16, String)>, delay: Duration) -> Mock {
    mock_gated(responses, delay, None)
}
fn mock_gated(
    responses: Vec<(u16, String)>,
    delay: Duration,
    gate: Option<std::sync::mpsc::Receiver<()>>,
) -> Mock {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    l.set_nonblocking(true).unwrap();
    let thread = std::thread::spawn(move || {
        for (code, body) in responses {
            let end = Instant::now() + Duration::from_secs(10);
            let mut s = loop {
                match l.accept() {
                    Ok((s, _)) => break s,
                    Err(_) if Instant::now() < end => std::thread::sleep(Duration::from_millis(2)),
                    Err(e) => panic!("missing HTTP request: {}", e),
                }
            };
            // Windows accepted sockets inherit the listener's nonblocking mode.
            s.set_nonblocking(false).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut r = std::io::BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            let mut length = 0;
            r.read_line(&mut line).unwrap();
            assert!(
                line.starts_with("POST /v1/chat/completions") || line.starts_with("GET /v1/models")
            );
            loop {
                line.clear();
                r.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            r.read_exact(&mut bytes).unwrap();
            captured.lock().push(if bytes.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(&bytes).unwrap()
            });
            if let Some(gate) = &gate {
                gate.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            std::thread::sleep(delay);
            let _ = write!(
                s,
                "HTTP/1.1 {} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                code,
                body.len(),
                body
            );
        }
    });
    Mock {
        url,
        requests,
        thread,
    }
}
fn cfg(url: &str) -> AgentConfig {
    let mut c = AgentConfig::default();
    c.backend = crate::agent_config::BackendKind::Openclaw;
    c.fallback_backend = "none".into();
    c.openclaw.base_url = url.into();
    c
}
fn text(s: &str) -> (u16, String) {
    (
        200,
        json!({"choices":[{"message":{"role":"assistant","content":s}}]}).to_string(),
    )
}
fn call(id: &str, name: &str, args: Value) -> (u16, String) {
    (200,json!({"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}).to_string())
}
fn plain(b: &OpenClawBackend, input: &str) -> AgentReply {
    b.run(
        &AgentRequest::text(input),
        &RequestControl::default(),
        &|_| {},
        &|_, _, _| panic!("unexpected tool"),
        &|| panic!("unexpected capture"),
    )
    .unwrap()
}
#[test]
fn plain_text_keeps_session_and_delivers_local_events_once() {
    let _lock = TEST_LOCK.lock();
    reset();
    remember_command("Открой блокнот", "open_app успешно выполнено");
    let m = mock(vec![text("Закрываю, сэр."), text("Привет, сэр.")]);
    let b = OpenClawBackend::new(cfg(&m.url));
    assert_eq!(plain(&b, "Теперь закрой его").speech, "Закрываю, сэр.");
    plain(&b, "Привет");
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert_eq!(req[0]["model"], "openclaw/jarvis");
    assert_eq!(req[0]["user"], req[1]["user"]);
    assert!(req[0]["messages"]
        .to_string()
        .contains("open_app успешно выполнено"));
    assert!(!req[1]["messages"]
        .to_string()
        .contains("open_app успешно выполнено"));
    assert_eq!(
        req[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["role"] == "user")
            .count(),
        1
    );
}
#[test]
fn sequential_tools_use_existing_actions_and_actual_results() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![
        call("a", "open_app", json!({"name":"блокнот"})),
        call("b", "type_text", json!({"text":"привет"})),
        text("Готово, сэр."),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    let executed = RefCell::new(Vec::new());
    let out = b
        .run(
            &AgentRequest::text("Открой блокнот и напечатай привет"),
            &RequestControl::default(),
            &|_| {},
            &|name, args, _| {
                executed
                    .borrow_mut()
                    .push(llm::tools::to_action(name, args)?);
                Ok(ActionOutcome {
                    chain: false,
                    speech: None,
                    report: format!("{} выполнено", name),
                })
            },
            &|| panic!(),
        )
        .unwrap();
    assert!(out.success && out.acted);
    assert!(matches!(&executed.borrow()[0],Action::OpenApp{name} if name=="блокнот"));
    assert!(matches!(&executed.borrow()[1],Action::TypeText{text} if text=="привет"));
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert!(req[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["tool_call_id"] == "a" && v["content"] == "open_app выполнено"));
    assert!(req[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["tool_call_id"] == "b"));
}
#[test]
fn unknown_tools_bad_arguments_and_tool_errors_are_not_success() {
    let _lock = TEST_LOCK.lock();
    reset();
    for response in [
        call("x", "shell", json!({"command":"evil"})),
        call("x", "set_volume", json!({"level":[]})),
        call("x", "open_app", json!({})),
        call("x", "open_url", json!({"url":"file:///C:/Windows"})),
    ] {
        let m = mock(vec![response, text("Готово")]);
        let b = OpenClawBackend::new(cfg(&m.url));
        let out = b
            .run(
                &AgentRequest::text("выполни"),
                &RequestControl::default(),
                &|_| {},
                &super::super::execute_tool,
                &|| panic!(),
            )
            .unwrap();
        assert!(!out.success && !out.acted);
        m.thread.join().unwrap();
    }
}
#[test]
fn malformed_focus_arguments_block_later_input() {
    let _lock = TEST_LOCK.lock();
    let _confirm = crate::actions::confirm::TEST_LOCK.lock();
    for args in ["{bad", "[]"] {
        reset();
        let mut first: Value = serde_json::from_str(&call("focus", "jarvis_client__focus_app", json!({})).1).unwrap();
        first["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!(args);
        let m = mock(vec![
            (200, first.to_string()),
            call("input", "jarvis_client__type_text", json!({"text":"не отправлять"})),
            text("Готово"),
        ]);
        let control = RequestControl::default();
        let out = OpenClawBackend::new(cfg(&m.url)).run(
            &AgentRequest::text("введи в нужном окне"), &control, &|_| {},
            &super::super::execute_tool, &|| panic!(),
        ).unwrap();
        assert!(!out.success && !out.acted);
        assert_eq!(control.action_attempts(), 0);
        assert!(out.speech.contains("ввод отменён"));
        m.thread.join().unwrap();
    }
}

#[test]
fn dangerous_tool_waits_for_user_and_returns_real_result() {
    let _lock = TEST_LOCK.lock();
    let _confirm = crate::actions::confirm::TEST_LOCK.lock();
    reset();
    crate::actions::confirm::clear();
    let m = mock(vec![
        call("danger", "power", json!({"action":"shutdown"})),
        text("Отменено, сэр."),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    let first = b
        .handle(
            &AgentRequest::text("выключи компьютер"),
            &RequestControl::default(),
            &|_| {},
        )
        .unwrap();
    assert!(first.chain && !first.acted);
    assert!(has_pending() && crate::actions::confirm::has_pending());
    assert_eq!(
        crate::actions::confirm::answer("нет"),
        crate::actions::confirm::Answer::Cancelled
    );
    let second = b
        .handle(
            &AgentRequest::continuation("не выполнено: пользователь отменил действие"),
            &RequestControl::default(),
            &|_| {},
        )
        .unwrap();
    assert!(!second.success);
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert!(req[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["tool_call_id"] == "danger"
            && v["content"] == "не выполнено: пользователь отменил действие"));
}
#[test]
fn actions_are_not_replayed_after_upstream_failure() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![
        call("a", "type_text", json!({"text":"привет"})),
        (503, "failed".into()),
    ]);
    let mut c = cfg(&m.url);
    c.fallback_backend = "direct".into();
    let b = OpenClawBackend::new(c);
    let count = RefCell::new(0);
    let out = b
        .run(
            &AgentRequest::text("напечатай"),
            &RequestControl::default(),
            &|_| {},
            &|_, _, _| {
                *count.borrow_mut() += 1;
                Ok(ActionOutcome {
                    chain: false,
                    speech: None,
                    report: "Текст напечатан".into(),
                })
            },
            &|| panic!(),
        )
        .unwrap();
    assert_eq!(*count.borrow(), 1);
    assert!(out.speech.contains("Текст напечатан"));
    m.thread.join().unwrap();
}

#[test]
fn keyboard_delivery_does_not_prove_a_project_was_created() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![
        call("input", "press_keys", json!({"name":"enter"})),
        (200, json!({"choices":[{"message":{"role":"assistant","content":"Проект успешно инициализирован."}}]}).to_string()),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    let out = b.run(
        &AgentRequest::text("инициализируй проект"),
        &RequestControl::default(),
        &|_| {},
        &|_, _, _| Ok(ActionOutcome { chain: false, speech: None, report: "нажато: enter".into() }),
        &|| panic!(),
    ).unwrap();
    assert!(out.speech.contains("нажато: enter") && out.speech.contains("не проверен"));
    assert!(!out.speech.contains("успешно инициализирован"));
    m.thread.join().unwrap();
}

#[test]
fn only_a_capture_after_keyboard_input_allows_a_visual_answer() {
    let _lock = TEST_LOCK.lock();
    for fresh in [true, false] {
        reset();
        let input = call("input", "press_keys", json!({"name":"enter"}));
        let capture = call("screen", "capture_screen_for_agent", json!({}));
        let mut responses = if fresh { vec![input, capture] } else { vec![capture, input] };
        responses.push(text("На экране ошибка: имя проекта недопустимо."));
        let m = mock(responses);
        let b = OpenClawBackend::new(cfg(&m.url));
        let out = b.run(
            &AgentRequest::text("нажми ввод и прочитай ошибку"),
            &RequestControl::default(),
            &|_| {},
            &|_, _, _| Ok(ActionOutcome { chain: false, speech: None, report: "нажато: enter".into() }),
            &|| Ok("data:image/png;base64,AA==".into()),
        ).unwrap();
        assert_eq!(out.speech.contains("имя проекта недопустимо"), fresh);
        assert_eq!(out.speech.contains("не проверен"), !fresh);
        m.thread.join().unwrap();
    }
}
#[test]
fn cancelled_requests_never_execute_late_tools() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![call("a", "type_text", json!({"text":"поздно"}))]);
    let b = OpenClawBackend::new(cfg(&m.url));
    let token = RequestControl::default();
    let out = b.run(
        &AgentRequest::text("задание"),
        &token,
        &|_| {},
        &|_, _, c| {
            c.cancel();
            c.check().map_err(|e| ActionError::Failed(e.to_string()))?;
            panic!("late action")
        },
        &|| panic!(),
    );
    assert_eq!(out.unwrap_err(), AgentError::Cancelled);
    m.thread.join().unwrap();
    let token = RequestControl::default();
    token.cancel();
    assert!(super::super::execute_tool("power", &json!({"action":"shutdown"}), &token).is_err());
}
#[test]
fn streaming_assembles_tools_and_rejects_truncation() {
    let values = [
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"open_app","arguments":"{\"name\":"}}]}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"test\"}"}}]}}]}),
    ];
    let bytes = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        values[0], values[1]
    );
    let msg = read_stream(bytes.as_bytes(), &RequestControl::default(), &|_| {}).unwrap();
    assert_eq!(
        msg["tool_calls"][0]["function"]["arguments"],
        "{\"name\":\"test\"}"
    );
    assert!(read_stream(&b"data: {}\n"[..], &RequestControl::default(), &|_| {}).is_err());
    let wrong_role = "data: {\"choices\":[{\"delta\":{\"role\":\"user\",\"content\":\"привет\"}}]}\n\ndata: [DONE]\n\n";
    assert_eq!(
        read_stream(wrong_role.as_bytes(), &RequestControl::default(), &|_| {}).unwrap_err(),
        AgentError::InvalidResponse
    );
}
#[test]
fn vision_is_attached_after_tool_results() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![
        call("v", "capture_screen_for_agent", json!({})),
        text("Вижу окно, сэр."),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    let out = b
        .run(
            &AgentRequest::text("Что на экране?"),
            &RequestControl::default(),
            &|_| {},
            &|_, _, _| panic!(),
            &|| super::super::vision::image_url(b"\x89PNG\r\n\x1a\n"),
        )
        .unwrap();
    assert!(out.success);
    m.thread.join().unwrap();
    let req = m.requests.lock();
    let messages = req[1]["messages"].as_array().unwrap();
    assert_eq!(messages[messages.len() - 2]["role"], "tool");
    assert_eq!(
        messages.last().unwrap()["content"][1]["image_url"]["url"],
        "data:image/png;base64,iVBORw0KGgo="
    );
}
#[test]
fn connection_checks_inventory_and_hides_arbitrary_tokens() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![(
        200,
        json!({"data":[{"id":"openclaw/jarvis"}]}).to_string(),
    )]);
    let mut c = cfg(&m.url);
    c.openclaw.api_key = "arbitrary-secret-token".into();
    assert!(check_connection(&c).unwrap().connected);
    assert!(!format!("{:?}", c).contains("arbitrary-secret-token"));
    m.thread.join().unwrap();
    let m = mock(vec![(401, "arbitrary-secret-token".into())]);
    c.openclaw.base_url = m.url.clone();
    let e = check_connection(&c).unwrap_err();
    assert_eq!(e, AgentError::AuthenticationError);
    assert!(!e.to_string().contains("arbitrary-secret-token"));
    m.thread.join().unwrap();
}
#[test]
fn unavailable_gateway_uses_direct_and_both_failures_return_an_error() {
    let _lock = TEST_LOCK.lock();
    reset();
    for works in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let mut c = cfg(&url);
        c.fallback_backend = "direct".into();
        let b = OpenClawBackend::new(c);
        let used = RefCell::new(false);
        let out = b.run_with_fallback(
            &AgentRequest::text("Привет"),
            &RequestControl::default(),
            &|_| {},
            &|_, _, _| panic!(),
            &|| panic!(),
            &|_, _, _| {
                *used.borrow_mut() = true;
                if works {
                    Ok(reply("Привет, сэр.".into(), true, false, true))
                } else {
                    Err(AgentError::AgentUnavailable)
                }
            },
        );
        assert!(*used.borrow());
        if works {
            assert!(out.unwrap().speech.contains("резервную"));
        } else {
            assert_eq!(out.unwrap_err(), AgentError::AgentUnavailable);
        }
    }
}
#[test]
fn ambiguous_timeouts_and_provider_errors_never_restart_the_task() {
    let _lock = TEST_LOCK.lock();
    reset();
    for code in [401, 403, 404, 405, 408, 500, 503, 504] {
        let m = mock(vec![(code, "possibly acted".into())]);
        let mut c = cfg(&m.url);
        c.fallback_backend = "direct".into();
        let b = OpenClawBackend::new(c);
        let out = b.run_with_fallback(
            &AgentRequest::text("создай заметку"),
            &RequestControl::default(),
            &|_| {},
            &|_, _, _| panic!(),
            &|| panic!(),
            &|_, _, _| panic!("ambiguous request must not be replayed"),
        );
        assert!(out.is_err());
        m.thread.join().unwrap();
    }
}
#[test]
fn response_size_and_malformed_json_are_bounded() {
    assert!(limited(&vec![b'x'; MAX_BYTES + 1][..]).is_err());
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![(200, "not JSON".into())]);
    let b = OpenClawBackend::new(cfg(&m.url));
    assert_eq!(
        b.handle(
            &AgentRequest::text("привет"),
            &RequestControl::default(),
            &|_| {}
        )
        .unwrap_err(),
        AgentError::InvalidResponse
    );
    m.thread.join().unwrap();
}
#[test]
fn vision_model_returns_analysis_to_the_main_agent_without_tools() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock(vec![
        call("v", "capture_screen_for_agent", json!({})),
        text("Открыто окно редактора"),
        text("На экране редактор, сэр."),
    ]);
    let mut c = cfg(&m.url);
    c.openclaw.vision_model = "vision/model".into();
    let b = OpenClawBackend::new(c);
    b.run(
        &AgentRequest::text("Что на экране?"),
        &RequestControl::default(),
        &|_| {},
        &|_, _, _| panic!(),
        &|| super::super::vision::image_url(b"\x89PNG\r\n\x1a\n"),
    )
    .unwrap();
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert_eq!(req[1]["tool_choice"], "none");
    assert!(req[1]["tools"].as_array().unwrap().is_empty());
    assert!(req[2]["messages"]
        .to_string()
        .contains("Анализ экрана: Открыто окно редактора"));
    assert!(!req[2]["messages"].to_string().contains("data:image"));
}
#[test]
fn late_http_response_after_cancel_does_not_execute_its_tool() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock_delayed(
        vec![call("a", "type_text", json!({"text":"поздно"}))],
        Duration::from_millis(150),
    );
    let b = OpenClawBackend::new(cfg(&m.url));
    let token = RequestControl::default();
    let worker_token = token.clone();
    let worker = std::thread::spawn(move || {
        b.run(
            &AgentRequest::text("задание"),
            &worker_token,
            &|_| {},
            &|_, _, _| panic!("late tool must not run"),
            &|| panic!(),
        )
    });
    let end = Instant::now() + Duration::from_secs(2);
    while m.requests.lock().is_empty() {
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(2));
    }
    token.cancel();
    assert_eq!(worker.join().unwrap().unwrap_err(), AgentError::Cancelled);
    m.thread.join().unwrap();
}
#[test]
fn hanging_gateway_obeys_the_request_budget() {
    let _lock = TEST_LOCK.lock();
    reset();
    let m = mock_delayed(vec![text("Поздно")], Duration::from_secs(4));
    let mut c = cfg(&m.url);
    c.openclaw.request_timeout_secs = 3;
    c.openclaw.task_timeout_secs = 3;
    let b = OpenClawBackend::new(c);
    let start = Instant::now();
    assert_eq!(
        b.handle(
            &AgentRequest::text("привет"),
            &RequestControl::default(),
            &|_| {}
        )
        .unwrap_err(),
        AgentError::Timeout
    );
    assert!(start.elapsed() < Duration::from_secs(4));
    m.thread.join().unwrap();
}
#[test]
fn abandoned_confirmation_is_reported_before_the_next_user_request() {
    let _lock = TEST_LOCK.lock();
    let _confirm = crate::actions::confirm::TEST_LOCK.lock();
    reset();
    crate::actions::confirm::clear();
    let m = mock(vec![
        call("danger", "power", json!({"action":"shutdown"})),
        text("Привет, сэр."),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    b.handle(
        &AgentRequest::text("выключи компьютер"),
        &RequestControl::default(),
        &|_| {},
    )
    .unwrap();
    crate::actions::confirm::clear();
    plain(&b, "Привет");
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert!(req[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["tool_call_id"] == "danger"
            && v["content"]
                .as_str()
                .unwrap_or("")
                .starts_with("не выполнено")));
    assert_eq!(
        req[1]["messages"].as_array().unwrap().last().unwrap()["content"],
        "Привет"
    );
}

#[test]
fn restart_recovers_a_tool_report_without_replaying_the_action() {
    let _lock = TEST_LOCK.lock();
    let _confirm = crate::actions::confirm::TEST_LOCK.lock();
    reset();
    crate::actions::confirm::clear();
    let m = mock(vec![
        call("danger", "power", json!({"action":"shutdown"})),
        text("Привет, сэр."),
    ]);
    let b = OpenClawBackend::new(cfg(&m.url));
    b.handle(
        &AgentRequest::text("выключи компьютер"),
        &RequestControl::default(),
        &|_| {},
    )
    .unwrap();
    assert!(SESSION
        .lock()
        .as_ref()
        .unwrap()
        .abandoned
        .iter()
        .any(|v| v["tool_call_id"] == "danger"));
    *PENDING.lock() = None;
    crate::actions::confirm::clear();
    plain(&b, "Привет");
    m.thread.join().unwrap();
    let req = m.requests.lock();
    assert!(req[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["tool_call_id"] == "danger"
            && v["content"].as_str().unwrap_or("").contains("перезапущен")));
    assert!(SESSION.lock().as_ref().unwrap().abandoned.is_empty());
}

#[test]
fn internal_mcp_action_followed_by_http_failure_never_uses_direct() {
    let _lock = TEST_LOCK.lock();
    let _confirm = crate::actions::confirm::TEST_LOCK.lock();
    reset();
    crate::actions::confirm::clear();
    for code in [401, 404, 503] {
        let (tx, rx) = std::sync::mpsc::channel();
        let m = mock_gated(
            vec![(code, "provider failed".into())],
            Duration::ZERO,
            Some(rx),
        );
        let mut c = cfg(&m.url);
        c.fallback_backend = "direct".into();
        let b = OpenClawBackend::new(c);
        let control = RequestControl::default();
        let child = control.child();
        let worker = std::thread::spawn(move || {
            b.run_with_fallback(
                &AgentRequest::text("задача"),
                &control,
                &|_| {},
                &|_, _, _| panic!("unexpected client tool"),
                &|| panic!(),
                &|_, _, _| panic!("task repeated after internal MCP action"),
            )
        });
        let end = Instant::now() + Duration::from_secs(3);
        while m.requests.lock().is_empty() {
            assert!(Instant::now() < end);
            std::thread::sleep(Duration::from_millis(2));
        }
        super::super::execute_tool("system_info", &json!({"what":"uptime"}), &child).unwrap();
        tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err(),
            AgentError::ResultUnknown
        );
        m.thread.join().unwrap();
    }
}

#[test]
fn unusable_final_answers_preserve_actual_tool_results() {
    let _lock = TEST_LOCK.lock();
    reset();
    for answer in ["", "```rust\nfn main() {}\n```", "The user wants me to open a program and then I should decide which tools to call to complete this task."] {
        let m = mock(vec![call("a", "type_text", json!({"text":"привет"})), text(answer)]);
        let b = OpenClawBackend::new(cfg(&m.url));
        let reply = b.run(
            &AgentRequest::text("напечатай привет"), &RequestControl::default(), &|_| {},
            &|_, _, _| Ok(ActionOutcome { chain: false, speech: None, report: "Текст напечатан".into() }),
            &|| panic!(),
        ).unwrap();
        assert!(reply.speech.contains("Текст напечатан"));
        assert!(!reply.speech.contains("The user"));
        assert!(reply.acted && reply.success);
        m.thread.join().unwrap();
    }
}

#[test]
fn malformed_assistant_roles_and_tool_types_never_execute_actions() {
    let _lock = TEST_LOCK.lock();
    reset();
    let (_, body) = call("a", "type_text", json!({"text":"привет"}));
    let mut wrong_role: Value = serde_json::from_str(&body).unwrap();
    wrong_role["choices"][0]["message"]["role"] = json!("user");
    let mut wrong_type: Value = serde_json::from_str(&body).unwrap();
    wrong_type["choices"][0]["message"]["tool_calls"][0]["type"] = json!("custom");
    for body in [wrong_role, wrong_type] {
        let m = mock(vec![(200, body.to_string())]);
        assert_eq!(
            OpenClawBackend::new(cfg(&m.url))
                .run(
                    &AgentRequest::text("напечатай"),
                    &RequestControl::default(),
                    &|_| {},
                    &|_, _, _| panic!("malformed tool executed"),
                    &|| panic!(),
                )
                .unwrap_err(),
            AgentError::InvalidResponse
        );
        m.thread.join().unwrap();
    }
}
