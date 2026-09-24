// LLM fallback for phrases the built-in commands did not understand.
// Talks to any OpenAI-compatible /chat/completions endpoint (Gemini, OpenRouter,
// Groq, Ollama...), rotates keys, and lets the model call native PC actions as tools.

pub mod models;
pub mod tools;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::actions::ActionError;
use crate::assistant_config::{self, LlmConfig, LlmProvider};

const MAX_TOOL_ROUNDS: usize = 5;
const MAX_HISTORY_MESSAGES: usize = 16;
const MAX_SPEECH_CHARS: usize = 600;
// marker in error text: the provider refuses requests from this country (VPN is off)
pub const REGION_BLOCKED: &str = "region blocked";

#[derive(Debug, Clone, PartialEq)]
pub struct LlmReply {
    pub speech: String,
    // keep listening: a dangerous action waits for "yes/no" or the dialog continues
    pub chain: bool,
    // at least one PC action was executed
    pub acted: bool,
}

#[derive(Debug)]
enum CallError {
    // this key is exhausted or invalid for a while
    Key { cooldown: Duration, reason: String },
    // this key hit the limit of this model only: the next model may still answer
    RateLimit { cooldown: Duration, reason: String },
    // this model does not work here (unknown, no tool support, bad request)
    Model(String),
    // the model is overloaded right now (Gemini 503 "high demand"): the next one may answer
    Busy(String),
    // provider unreachable or failing
    Provider(String),
}

struct State {
    // "provider#key_index" -> blocked until
    cooldowns: HashMap<String, Instant>,
    // provider -> next key index (round-robin)
    next_key: HashMap<String, usize>,
    history: Vec<Value>,
    last_used: Option<Instant>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| {
    Mutex::new(State { cooldowns: HashMap::new(), next_key: HashMap::new(), history: Vec::new(), last_used: None })
});

pub fn is_configured() -> bool {
    let cfg = &assistant_config::get().llm;
    cfg.enabled && cfg.active_providers().iter().any(|p| p.keyless || p.keys.iter().any(|k| !k.trim().is_empty()))
}

pub fn reset_history() {
    let mut st = STATE.lock();
    st.history.clear();
    st.last_used = None;
}

fn system_prompt() -> String {
    let cfg = assistant_config::get();
    let now = chrono::Local::now().format("%d.%m.%Y %H:%M, %A");
    let dirs: Vec<String> = assistant_config::allowed_dirs().iter().map(|d| d.display().to_string()).collect();

    let mut p = format!(
        "Ты — Джарвис, голосовой ассистент на компьютере с Windows 11. Сейчас {now}.\n\
         Твой ответ будет произнесён вслух синтезатором речи, поэтому:\n\
         - отвечай по-русски, коротко: одно-два предложения;\n\
         - без markdown, списков, эмодзи и ссылок;\n\
         - числа и сокращения пиши так, как их удобно произнести.\n\
         Чтобы что-то сделать на компьютере, вызывай инструменты; не выдумывай, что действие выполнено, \
         если инструмент вернул ошибку. Если просят то, чего инструменты не умеют, честно скажи об этом.\n\
         Для удаления файла сначала найди его через find_files, затем вызови delete_file с полным путём — \
         пользователь подтвердит голосом. Файлы доступны только в папках: {dirs}.\n\
         Речь распознаётся с ошибками: названия программ и игр могут быть искажены, угадывай по смыслу.\n\
         Обращайся к пользователю «{address}».",
        now = now,
        address = assistant_config::address(),
        dirs = dirs.join("; ")
    );
    if !cfg.llm.extra_prompt.trim().is_empty() {
        p.push_str("\n");
        p.push_str(cfg.llm.extra_prompt.trim());
    }
    p
}

// strip formatting the TTS would read aloud, cap the length
pub fn clean_for_speech(text: &str) -> String {
    // drop reasoning blocks some open models emit
    let mut raw = text.to_string();
    while let (Some(s), Some(e)) = (raw.find("<think>"), raw.find("</think>")) {
        if e < s {
            break;
        }
        raw.replace_range(s..e + "</think>".len(), "");
    }
    let out: String = raw
        .chars()
        .filter(|c| !matches!(c, '*' | '#' | '`' | '_' | '~' | '>' | '|'))
        .collect();
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().count() > MAX_SPEECH_CHARS {
        let cut: String = out.chars().take(MAX_SPEECH_CHARS).collect();
        // end at the last full sentence if there is one
        match cut.rfind(|c| matches!(c, '.' | '!' | '?')) {
            Some(i) if i > MAX_SPEECH_CHARS / 2 => cut[..=i].to_string(),
            _ => format!("{}…", cut),
        }
    } else {
        out
    }
}

fn key_id(provider: &str, idx: usize) -> String {
    format!("{}#{}", provider, idx)
}

fn classify_status(status: u16, body: &str, retry_after: Option<u64>) -> CallError {
    let short: String = body.chars().take(300).collect();
    let lower = body.to_lowercase();
    // Gemini reports a bad key as 400 and a blocked country as 400 FAILED_PRECONDITION
    if lower.contains("api_key_invalid") || lower.contains("api key not valid") || lower.contains("api key expired") {
        return CallError::Key { cooldown: Duration::from_secs(3600), reason: format!("invalid API key: {}", short) };
    }
    if lower.contains("location is not supported") {
        return CallError::Provider(format!("{} {}: {}", status, REGION_BLOCKED, short));
    }
    match status {
        429 => CallError::RateLimit {
            cooldown: Duration::from_secs(retry_after.unwrap_or(60).clamp(5, 3600)),
            reason: format!("429 rate limit: {}", short),
        },
        401 => CallError::Key { cooldown: Duration::from_secs(3600), reason: format!("401 unauthorized: {}", short) },
        // Kilo answers 403 now and then under a burst of requests, or for one model: that model
        // rests a few minutes, the key keeps working with the others
        403 => CallError::RateLimit { cooldown: Duration::from_secs(300), reason: format!("403 forbidden: {}", short) },
        400 | 404 | 405 | 409 | 413 | 422 => CallError::Model(format!("{}: {}", status, short)),
        500 | 502 | 503 | 504 => CallError::Busy(format!("{}: {}", status, short)),
        402 => CallError::Key { cooldown: Duration::from_secs(3600), reason: format!("402 no credits left, the next providers answer: {}", short) },
        _ => CallError::Provider(format!("{}: {}", status, short)),
    }
}

fn post(cfg: &LlmConfig, provider: &LlmProvider, key: &str, model: &str, messages: &[Value], timeout: Duration) -> Result<Value, CallError> {
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": messages,
        "tools": tools::definitions(),
        "tool_choice": "auto",
        "temperature": cfg.temperature,
        "max_tokens": cfg.max_tokens,
    });

    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| CallError::Provider(e.to_string()))?;

    let mut req = client.post(&url).json(&body);
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    if provider.base_url.contains("openrouter.ai") {
        req = req.header("X-Title", "Jarvis Voice Assistant");
    }

    let resp = req.send().map_err(|e| CallError::Provider(format!("network: {}", e)))?;
    let status = resp.status().as_u16();
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    let text = resp.text().unwrap_or_default();

    if !(200..300).contains(&status) {
        return Err(classify_status(status, &text, retry_after));
    }

    let v: Value = serde_json::from_str(&text).map_err(|e| CallError::Provider(format!("bad JSON: {}", e)))?;
    // some gateways report errors with HTTP 200
    if let Some(err) = v.get("error") {
        let code = err.get("code").and_then(|c| c.as_u64()).unwrap_or(500) as u16;
        return Err(classify_status(code, &err.to_string(), None));
    }
    v.pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| CallError::Model(format!("no choices in response: {}", text.chars().take(200).collect::<String>())))
}

// after a network failure (Gemini hangs until the timeout without a VPN) the next providers go first
const PROVIDER_REST: Duration = Duration::from_secs(120);

fn provider_id(name: &str) -> String {
    format!("provider:{}", name)
}

// one completion from the first provider/model/key that works
fn complete_with(cfg: &LlmConfig, messages: &[Value]) -> Result<Value, String> {
    let timeout = Duration::from_secs(cfg.timeout_secs.max(3));
    let mut errors: Vec<String> = Vec::new();

    // a provider that just timed out is skipped for a while, unless nothing else is left
    let enabled: Vec<&LlmProvider> = cfg.active_providers();
    let resting = |p: &LlmProvider| STATE.lock().cooldowns.get(&provider_id(&p.name)).is_some_and(|until| Instant::now() < *until);
    let awake: Vec<&LlmProvider> = enabled.iter().copied().filter(|p| !resting(p)).collect();
    let providers = if awake.is_empty() { enabled } else { awake };

    for provider in providers {
        let keys: Vec<(usize, String)> = if provider.keyless {
            vec![(0, String::new())]
        } else {
            provider
                .keys
                .iter()
                .enumerate()
                .filter(|(_, k)| !k.trim().is_empty())
                .map(|(i, k)| (i, k.trim().to_string()))
                .collect()
        };
        if keys.is_empty() || provider.models.is_empty() {
            continue;
        }

        // "auto" asks the provider which models exist (needs any key)
        let model_list = models::resolve(provider, &keys[0].1, timeout);

        'models: for model in &model_list {
            // round-robin start, skip keys in cooldown
            let start = *STATE.lock().next_key.get(&provider.name).unwrap_or(&0);
            for offset in 0..keys.len() {
                let (idx, key) = &keys[(start + offset) % keys.len()];
                let id = key_id(&provider.name, *idx);
                let model_id = format!("{}:{}", id, model);
                let blocked = {
                    let st = STATE.lock();
                    [&id, &model_id].iter().any(|k| st.cooldowns.get(*k).is_some_and(|until| Instant::now() < *until))
                };
                if blocked {
                    continue;
                }

                info!("LLM request: provider={} model={} key#{}", provider.name, model, idx);
                match post(cfg, provider, key, model, messages, timeout) {
                    Ok(msg) => {
                        let mut st = STATE.lock();
                        st.next_key.insert(provider.name.clone(), (start + offset + 1) % keys.len());
                        return Ok(msg);
                    }
                    Err(CallError::Key { cooldown, reason }) => {
                        warn!("LLM {} key#{}: {} (cooldown {:?})", provider.name, idx, reason, cooldown);
                        STATE.lock().cooldowns.insert(id, Instant::now() + cooldown);
                        errors.push(format!("{} key#{}: {}", provider.name, idx, reason));
                    }
                    Err(CallError::RateLimit { cooldown, reason }) => {
                        warn!("LLM {} key#{} model {}: {} (cooldown {:?})", provider.name, idx, model, reason, cooldown);
                        STATE.lock().cooldowns.insert(model_id, Instant::now() + cooldown);
                        errors.push(format!("{} key#{} {}: {}", provider.name, idx, model, reason));
                    }
                    Err(CallError::Busy(reason)) => {
                        warn!("LLM {} model {} is busy: {}", provider.name, model, reason);
                        errors.push(format!("{} {}: {}", provider.name, model, reason));
                        continue 'models;
                    }
                    Err(CallError::Model(reason)) => {
                        warn!("LLM {} model {}: {}", provider.name, model, reason);
                        models::forget(&provider.name, model);
                        errors.push(format!("{} {}: {}", provider.name, model, reason));
                        continue 'models;
                    }
                    Err(CallError::Provider(reason)) => {
                        warn!("LLM provider {} failed: {}", provider.name, reason);
                        if reason.starts_with("network") {
                            STATE.lock().cooldowns.insert(provider_id(&provider.name), Instant::now() + PROVIDER_REST);
                        }
                        errors.push(format!("{}: {}", provider.name, reason));
                        break 'models;
                    }
                }
            }
        }
    }

    if errors.is_empty() {
        Err("нет доступных ключей (все на паузе или не заданы)".into())
    } else {
        Err(errors.join(" || "))
    }
}

fn trim_history(history: &mut Vec<Value>) {
    if history.len() <= MAX_HISTORY_MESSAGES {
        return;
    }
    let mut cut = history.len() - MAX_HISTORY_MESSAGES;
    // never start with a tool result or an assistant tool call without its pair
    while cut < history.len() && history[cut].get("role").and_then(|r| r.as_str()) != Some("user") {
        cut += 1;
    }
    history.drain(..cut);
}

// handle a phrase the command matcher did not understand
pub fn handle(text: &str) -> Result<LlmReply, String> {
    if !is_configured() {
        return Err("нейросеть не настроена: добавьте ключи в assistant.toml".into());
    }
    handle_with(&assistant_config::get().llm, text)
}

fn handle_with(cfg: &LlmConfig, text: &str) -> Result<LlmReply, String> {
    let memory = Duration::from_secs(cfg.memory_minutes * 60);
    let mut history = {
        let mut st = STATE.lock();
        if st.last_used.map(|t| t.elapsed() > memory).unwrap_or(true) {
            st.history.clear();
        }
        st.last_used = Some(Instant::now());
        st.history.clone()
    };

    history.push(json!({"role": "user", "content": text}));

    let mut acted = false;
    let mut result: Option<LlmReply> = None;

    for _round in 0..MAX_TOOL_ROUNDS {
        let mut messages = vec![json!({"role": "system", "content": system_prompt()})];
        messages.extend(history.iter().cloned());

        let msg = complete_with(cfg, &messages)?;
        let tool_calls = msg.get("tool_calls").and_then(|t| t.as_array()).cloned().unwrap_or_default();

        if tool_calls.is_empty() {
            let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
            history.push(json!({"role": "assistant", "content": content}));
            let speech = clean_for_speech(&content);
            result = Some(LlmReply {
                speech: if speech.is_empty() { "Готово.".into() } else { speech },
                chain: true,
                acted,
            });
            break;
        }

        // keep the assistant turn exactly as the API expects it back
        let mut assistant_turn = json!({"role": "assistant", "content": msg.get("content").cloned().unwrap_or(Value::Null), "tool_calls": []});
        let mut normalized_calls = Vec::new();
        for (i, call) in tool_calls.iter().enumerate() {
            let mut call = call.clone();
            if call.get("id").and_then(|v| v.as_str()).map(|s| s.is_empty()).unwrap_or(true) {
                call["id"] = json!(format!("call_{}", i));
            }
            if call.get("type").is_none() {
                call["type"] = json!("function");
            }
            normalized_calls.push(call);
        }
        assistant_turn["tool_calls"] = json!(normalized_calls.clone());
        history.push(assistant_turn);

        let mut waiting_confirmation: Option<String> = None;

        for call in &normalized_calls {
            let id = call["id"].as_str().unwrap_or("call").to_string();
            let name = call.pointer("/function/name").and_then(|v| v.as_str()).unwrap_or("");
            let args_raw = call.pointer("/function/arguments").cloned().unwrap_or(json!("{}"));
            // arguments are a JSON string per spec, some servers send an object
            let args: Value = match &args_raw {
                Value::String(s) => serde_json::from_str(s).unwrap_or(json!({})),
                other => other.clone(),
            };

            let content = if waiting_confirmation.is_some() {
                "не выполнено: сначала нужно подтверждение предыдущего действия".to_string()
            } else {
                info!("LLM tool call: {} {}", name, args);
                match tools::to_action(name, &args).and_then(|a| a.run()) {
                    Ok(outcome) => {
                        acted = true;
                        if outcome.chain {
                            waiting_confirmation = outcome.speech.clone();
                        }
                        outcome.report
                    }
                    Err(ActionError::NotFound(m)) => format!("не найдено: {}", m),
                    Err(e) => format!("ошибка: {}", e),
                }
            };
            history.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
        }

        if let Some(question) = waiting_confirmation {
            result = Some(LlmReply { speech: question, chain: true, acted });
            break;
        }
    }

    let reply = result.unwrap_or(LlmReply { speech: "Готово.".into(), chain: false, acted });

    trim_history(&mut history);
    let mut st = STATE.lock();
    st.history = history;
    st.last_used = Some(Instant::now());

    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speech_is_cleaned_and_capped() {
        assert_eq!(clean_for_speech("**Привет**, `мир`!\n\n# Итог"), "Привет, мир! Итог");
        assert_eq!(clean_for_speech("<think>hmm</think>Ответ."), "Ответ.");
        let long = "Предложение. ".repeat(100);
        let c = clean_for_speech(&long);
        assert!(c.chars().count() <= MAX_SPEECH_CHARS);
        assert!(c.ends_with('.'));
    }

    #[test]
    fn statuses_are_classified() {
        assert!(matches!(classify_status(429, "", Some(10)), CallError::RateLimit { cooldown, .. } if cooldown == Duration::from_secs(10)));
        assert!(matches!(classify_status(401, "", None), CallError::Key { .. }));
        assert!(matches!(classify_status(403, "", None), CallError::RateLimit { cooldown, .. } if cooldown == Duration::from_secs(300)));
        assert!(matches!(classify_status(503, r#"{"error":{"code":503,"status":"UNAVAILABLE"}}"#, None), CallError::Busy(_)));
        assert!(matches!(classify_status(404, "no endpoints support tools", None), CallError::Model(_)));
        assert!(matches!(classify_status(521, "", None), CallError::Provider(_)));
        assert!(matches!(classify_status(400, r#"{"error":{"message":"API key not valid. Please pass a valid API key.","status":"INVALID_ARGUMENT"}}"#, None), CallError::Key { .. }));
        assert!(matches!(classify_status(400, r#"{"error":{"message":"User location is not supported for the API use.","status":"FAILED_PRECONDITION"}}"#, None), CallError::Provider(ref m) if m.contains(REGION_BLOCKED)));
    }

    #[test]
    fn history_trim_starts_with_user_turn() {
        let mut h: Vec<Value> = Vec::new();
        for i in 0..10 {
            h.push(json!({"role": "user", "content": i}));
            h.push(json!({"role": "assistant", "tool_calls": []}));
            h.push(json!({"role": "tool", "content": "x"}));
        }
        trim_history(&mut h);
        assert!(h.len() <= MAX_HISTORY_MESSAGES);
        assert_eq!(h[0]["role"], "user");
    }
    // minimal HTTP server: answers by the bearer key, records what it saw
    fn mock_server(responses: Vec<(&'static str, u16, &'static str)>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = vec![0u8; 65536];
                let mut req = String::new();
                // read until the body is complete
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 { break; }
                    req.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if let Some(h_end) = req.find("\r\n\r\n") {
                        let len = req.lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if req.len() >= h_end + 4 + len { break; }
                    }
                }
                let auth = req.lines()
                    .find_map(|l| l.strip_prefix("authorization: Bearer ").or_else(|| l.strip_prefix("Authorization: Bearer ")))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                seen2.lock().push(auth.clone());
                let (status, body) = responses.iter()
                    .find(|(k, _, _)| *k == auth)
                    .map(|(_, s, b)| (*s, *b))
                    .unwrap_or((500, "{}"));
                let resp = format!("HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", status, body.len(), body);
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{}", addr), seen)
    }

    const OK_BODY: &str = r#"{"choices":[{"message":{"role":"assistant","content":"Привет"}}]}"#;

    fn provider(name: &str, url: &str, keys: &[&str]) -> LlmProvider {
        LlmProvider {
            name: name.into(),
            enabled: true,
            base_url: url.into(),
            models: vec!["m".into()],
            keys: keys.iter().map(|k| k.to_string()).collect(),
            keyless: false,
        }
    }

    #[test]
    fn rotates_keys_and_providers() {
        let (url, seen) = mock_server(vec![
            ("limited", 429, r#"{"error":"rate"}"#),
            ("bad", 401, r#"{"error":"auth"}"#),
            ("good", 200, OK_BODY),
        ]);
        let cfg = LlmConfig {
            timeout_secs: 5,
            providers: vec![
                provider("rot-a", &url, &["limited", "bad"]),
                provider("rot-b", &url, &["", "good"]),
            ],
            ..LlmConfig::default()
        };
        let msg = complete_with(&cfg, &[json!({"role": "user", "content": "hi"})]).unwrap();
        assert_eq!(msg["content"], "Привет");
        assert_eq!(*seen.lock(), vec!["limited".to_string(), "bad".into(), "good".into()]);

        // exhausted keys are skipped on the next request
        seen.lock().clear();
        complete_with(&cfg, &[json!({"role": "user", "content": "hi"})]).unwrap();
        assert_eq!(*seen.lock(), vec!["good".to_string()]);
    }

    #[test]
    fn reports_error_when_nothing_works() {
        let (url, _) = mock_server(vec![("k", 503, "{}")]);
        let cfg = LlmConfig { timeout_secs: 5, providers: vec![provider("fail-a", &url, &["k"])], ..LlmConfig::default() };
        let err = complete_with(&cfg, &[json!({"role": "user", "content": "hi"})]).unwrap_err();
        assert!(err.contains("fail-a"), "{}", err);
    }
    #[test]
    fn free_models_answer_when_the_paid_key_has_no_credits() {
        let ok = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
        let (url, seen) = mock_server(vec![("no-credits", 402, r#"{"error":{"message":"Insufficient balance"}}"#), ("", 200, ok)]);
        let mut free = provider("free-402", &url, &[]);
        free.keyless = true;
        let cfg = LlmConfig { timeout_secs: 5, providers: vec![provider("paid-402", &url, &["no-credits"]), free], ..LlmConfig::default() };
        let msgs = [json!({"role": "user", "content": "hi"})];
        assert!(complete_with(&cfg, &msgs).is_ok());
        assert!(complete_with(&cfg, &msgs).is_ok());
        // the empty balance is asked once, then the key rests
        assert_eq!(*seen.lock(), vec!["no-credits".to_string(), String::new(), String::new()]);

        let only_free = LlmConfig { free_only: true, ..cfg };
        assert_eq!(only_free.active_providers().len(), 1);
    }

    #[test]
    fn hanging_provider_rests_and_the_next_one_answers() {
        // accepts connections and never answers, like Gemini without a VPN
        let hang = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let hang_url = format!("http://{}", hang.local_addr().unwrap());
        std::thread::spawn(move || {
            let _held: Vec<_> = hang.incoming().flatten().collect();
        });
        let (url, seen) = mock_server(vec![("", 200, r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#)]);
        let mut free = provider("free-rest", &url, &[]);
        free.keyless = true;
        let cfg = LlmConfig {
            timeout_secs: 3,
            providers: vec![provider("hang-rest", &hang_url, &["k"]), free],
            ..LlmConfig::default()
        };
        let msgs = [json!({"role": "user", "content": "hi"})];
        assert!(complete_with(&cfg, &msgs).is_ok());
        let started = Instant::now();
        assert!(complete_with(&cfg, &msgs).is_ok());
        assert!(started.elapsed() < Duration::from_secs(2), "the hanging provider was asked again");
        assert_eq!(seen.lock().len(), 2);
    }

    // answers requests in order from a queue, records request bodies
    fn queue_server(bodies: Vec<&'static str>) -> (String, std::sync::Arc<Mutex<Vec<Value>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            let mut queue = bodies.into_iter();
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = vec![0u8; 65536];
                let mut req: Vec<u8> = Vec::new();
                let body_start;
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 { body_start = req.len(); break; }
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(h_end) = text.find("\r\n\r\n") {
                        let len = text.lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if req.len() >= h_end + 4 + len { body_start = h_end + 4; break; }
                    }
                }
                let body: Value = serde_json::from_slice(&req[body_start.min(req.len())..]).unwrap_or(Value::Null);
                seen2.lock().push(body);
                let out = queue.next().unwrap_or("{}");
                let resp = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", out.len(), out);
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{}", addr), seen)
    }

    #[test]
    fn tool_calls_are_executed_and_answer_is_spoken() {
        let _guard = crate::actions::confirm::TEST_LOCK.lock();
        crate::actions::confirm::clear();
        reset_history();
        let (url, seen) = queue_server(vec![
            // arguments as a JSON string (OpenAI style), no id (some gateways omit it)
            r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"type":"function","function":{"name":"power","arguments":"{\"action\":\"shutdown\"}"}}]}}]}"#,
            r#"{"choices":[{"message":{"role":"assistant","content":"не должно дойти"}}]}"#,
        ]);
        let cfg = LlmConfig { timeout_secs: 5, providers: vec![provider("tools", &url, &["k"])], ..LlmConfig::default() };

        let reply = handle_with(&cfg, "выключи компьютер").unwrap();
        // dangerous action: asks for confirmation instead of a second LLM round
        assert!(reply.chain);
        assert!(reply.speech.contains("да или нет"), "{}", reply.speech);
        assert_eq!(seen.lock().len(), 1);
        let sent = &seen.lock()[0];
        assert_eq!(sent["messages"][0]["role"], "system");
        assert!(sent["tools"].as_array().unwrap().len() > 10);
        assert_eq!(crate::actions::confirm::answer("нет"), crate::actions::confirm::Answer::Cancelled);

        // history keeps the tool call paired with its result
        let st = STATE.lock();
        let roles: Vec<&str> = st.history.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool"]);
        assert_eq!(st.history[1]["tool_calls"][0]["id"], "call_0");
        assert_eq!(st.history[2]["tool_call_id"], "call_0");
    }

    #[test]
    fn plain_answer_after_failed_tool() {
        let _guard = crate::actions::confirm::TEST_LOCK.lock();
        reset_history();
        let (url, seen) = queue_server(vec![
            r#"{"choices":[{"message":{"role":"assistant","content":"","tool_calls":[{"id":"a1","type":"function","function":{"name":"open_url","arguments":{"url":"file:///etc"}}}]}}]}"#,
            r#"{"choices":[{"message":{"role":"assistant","content":"**Не могу** открыть эту ссылку."}}]}"#,
        ]);
        let cfg = LlmConfig { timeout_secs: 5, providers: vec![provider("tools2", &url, &["k"])], ..LlmConfig::default() };
        let reply = handle_with(&cfg, "открой файл").unwrap();
        assert_eq!(reply.speech, "Не могу открыть эту ссылку.");
        assert!(!reply.acted);
        let second = &seen.lock()[1];
        let tool_msg = second["messages"].as_array().unwrap().iter().find(|m| m["role"] == "tool").unwrap().clone();
        assert!(tool_msg["content"].as_str().unwrap().contains("http"), "{}", tool_msg);
    }
    // routes by "METHOD path" and records requests with their model and key header
    fn route_server(routes: Vec<(&'static str, u16, &'static str)>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = vec![0u8; 65536];
                let mut req = String::new();
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 { break; }
                    req.push_str(&String::from_utf8_lossy(&buf[..n]));
                    if let Some(h_end) = req.find("\r\n\r\n") {
                        let len = req.lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if req.len() >= h_end + 4 + len { break; }
                    }
                }
                let first = req.lines().next().unwrap_or("").to_string();
                let parts: Vec<&str> = first.split_whitespace().collect();
                let key = format!("{} {}", parts.first().unwrap_or(&""), parts.get(1).unwrap_or(&"").split('?').next().unwrap_or(""));
                let model = req.split("\"model\":\"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("-").to_string();
                let goog = req.lines().any(|l| l.to_lowercase().starts_with("x-goog-api-key: k1"));
                seen2.lock().push(format!("{} model={} goog={}", key, model, goog));
                let route = format!("{} model={}", key, model);
                let (status, body) = routes.iter()
                    .find(|(k, _, _)| *k == route)
                    .map(|(_, s, b)| (*s, *b))
                    .unwrap_or((404, r#"{"error":{"code":404,"message":"not found"}}"#));
                let resp = format!("HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", status, body.len(), body);
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{}", addr), seen)
    }

    #[test]
    fn auto_models_start_with_the_cheapest_and_skip_limited_ones() {
        models::clear_cache();
        let (url, seen) = route_server(vec![
            ("GET /v1beta/models model=-", 200, r#"{"models":[
                {"name":"models/gemini-2.5-flash","supportedGenerationMethods":["generateContent"]},
                {"name":"models/gemini-3.6-flash-lite","supportedGenerationMethods":["generateContent"]},
                {"name":"models/gemini-3.5-flash-lite","supportedGenerationMethods":["generateContent"]},
                {"name":"models/gemini-3.4-flash-lite","supportedGenerationMethods":["generateContent"]},
                {"name":"models/gemini-embedding-001","supportedGenerationMethods":["embedContent"]}]}"#),
            // the cheapest model hits its limit, the next Flash-Lite answers;
            // 3.4 is below the preferred versions and "-" is a retired one (404)
            ("POST /v1beta/openai/chat/completions model=gemini-3.5-flash-lite", 429, r#"{"error":{"code":429,"message":"quota"}}"#),
            ("POST /v1beta/openai/chat/completions model=gemini-3.6-flash-lite", 200, r#"{"choices":[{"message":{"role":"assistant","content":"Да, сэр"}}]}"#),
        ]);
        let cfg = LlmConfig {
            timeout_secs: 5,
            providers: vec![LlmProvider {
                name: "gemini".into(),
                enabled: true,
                base_url: format!("{}/v1beta/openai", url),
                models: vec!["auto".into()],
                keys: vec!["k1".into()],
                keyless: false,
            }],
            ..LlmConfig::default()
        };
        let msg = complete_with(&cfg, &[json!({"role": "user", "content": "hi"})]).unwrap();
        assert_eq!(msg["content"], "Да, сэр");
        let log = seen.lock().clone();
        assert_eq!(log[0], "GET /v1beta/models model=- goog=true", "{:?}", log);
        assert_eq!(log[1], "POST /v1beta/openai/chat/completions model=gemini-3.5-flash-lite goog=false");
        assert_eq!(log[2], "POST /v1beta/openai/chat/completions model=gemini-3.6-flash-lite goog=false");
        assert_eq!(log.len(), 3, "{:?}", log);

        // the list is cached; the limited model waits out its cooldown, the key keeps working
        seen.lock().clear();
        complete_with(&cfg, &[json!({"role": "user", "content": "hi"})]).unwrap();
        assert_eq!(*seen.lock(), vec!["POST /v1beta/openai/chat/completions model=gemini-3.6-flash-lite goog=false".to_string()]);
    }
}
