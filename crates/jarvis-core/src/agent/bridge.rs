// Authenticated local adapter. All OS actions are dispatched to the running voice loop.
use super::RequestControl;
use crate::actions::{self, ActionOutcome};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    mpsc::{self, Receiver, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};

pub const DISCOVERY_FILE: &str = "jarvis-pc.json";
#[derive(Serialize, Deserialize)]
pub struct Discovery {
    pub address: String,
    pub token: String,
}
struct Job {
    name: String,
    args: Value,
    control: RequestControl,
    deadline: Instant,
    reply: SyncSender<Value>,
}
struct Deferred {
    reply: SyncSender<Value>,
    deadline: Instant,
    name: String,
    control: RequestControl,
}
static CHANNEL: Lazy<(SyncSender<Job>, Mutex<Receiver<Job>>)> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel(8);
    (tx, Mutex::new(rx))
});
static DEFERRED: Lazy<Mutex<Option<Deferred>>> = Lazy::new(|| Mutex::new(None));

pub fn start() -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "Не удалось запустить локальные инструменты".to_string())?;
    let info = Discovery {
        address: listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string(),
        token: format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        ),
    };
    let path = crate::APP_CONFIG_DIR
        .get()
        .ok_or("Нет каталога конфигурации")?
        .join(DISCOVERY_FILE);
    crate::storage::atomic_write(
        &path,
        &serde_json::to_vec(&info).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
    }
    let token = Arc::new(info.token);
    std::thread::spawn(move || {
        // Bound concurrent sockets before spawning threads, including unauthorized requests.
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for stream in listener.incoming().flatten() {
            use std::sync::atomic::Ordering;
            if active.fetch_add(1, Ordering::SeqCst) >= 8 {
                active.fetch_sub(1, Ordering::SeqCst);
                drop(stream);
                continue;
            }
            let (token, active) = (token.clone(), active.clone());
            std::thread::spawn(move || {
                let _ = serve(stream, &token);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    info!("jarvis-pc tools enabled on loopback");
    Ok(())
}

fn response(stream: &mut TcpStream, code: u16, value: Value) -> Result<(), String> {
    let body = value.to_string();
    write!(stream, "HTTP/1.1 {} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", code, body.len(), body).map_err(|e| e.to_string())
}
fn authorized(candidate: &str, token: &str) -> bool {
    candidate.len() == token.len()
        && candidate
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}
fn serve(mut stream: TcpStream, token: &str) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let mut reader =
        std::io::BufReader::new(stream.try_clone().map_err(|e| e.to_string())?).take(32 * 1024);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if line.trim() != "POST /tool HTTP/1.1" {
        return response(&mut stream, 404, json!({"error":"unknown endpoint"}));
    }
    let mut length = None;
    let mut auth = None;
    let mut origin = false;
    let mut chunked = false;
    let mut ended = false;
    loop {
        line.clear();
        if reader.read_line(&mut line).map_err(|e| e.to_string())? == 0 {
            break;
        }
        if line == "\r\n" {
            ended = true;
            break;
        }
        if let Some((key, val)) = line.split_once(':') {
            match key.to_ascii_lowercase().as_str() {
                "content-length" => {
                    if length.is_some() {
                        return response(&mut stream, 400, json!({"error":"duplicate length"}));
                    }
                    length = val.trim().parse::<usize>().ok();
                }
                "authorization" => auth = val.trim().strip_prefix("Bearer ").map(str::to_string),
                "origin" => origin = true,
                "transfer-encoding" => chunked = true,
                _ => {}
            }
        }
    }
    if !ended || chunked {
        return response(&mut stream, 400, json!({"error":"bad framing"}));
    }
    if origin || !auth.as_deref().is_some_and(|a| authorized(a, token)) {
        return response(&mut stream, 403, json!({"error":"forbidden"}));
    }
    let Some(length) = length.filter(|n| *n <= 64 * 1024) else {
        return response(&mut stream, 413, json!({"error":"request too large"}));
    };
    let mut bytes = vec![0; length];
    // The header budget must not truncate a valid body buffered by BufReader.
    let mut reader = reader.into_inner();
    reader.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid JSON".to_string())?;
    let name = v["name"]
        .as_str()
        .ok_or("missing name")?
        .trim_start_matches("jarvis.")
        .to_string();
    let args = v.get("arguments").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object()
        || !super::openclaw::definitions()
            .iter()
            .any(|t| t["function"]["name"].as_str() == Some(&name))
    {
        return response(
            &mut stream,
            400,
            json!({"error":"unknown tool or invalid arguments"}),
        );
    }
    if super::mcp_blocked() {
        return response(
            &mut stream,
            409,
            json!({"error":"cancelled agent is still stopping"}),
        );
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let control = super::mcp_request_control();
    let job = Job {
        name,
        args,
        control: control.clone(),
        deadline: Instant::now() + Duration::from_secs(120),
        reply: tx,
    };
    if CHANNEL.0.try_send(job).is_err() {
        return response(&mut stream, 429, json!({"error":"voice loop busy"}));
    }
    let result = rx
        .recv_timeout(Duration::from_secs(120))
        .unwrap_or_else(|_| {
            control.cancel();
            json!({"error":"action timed out; check its result before retrying"})
        });
    response(&mut stream, 200, result)
}

pub fn has_pending() -> bool {
    DEFERRED.lock().is_some()
}

pub fn process_next() -> Option<ActionOutcome> {
    expire();
    if DEFERRED.lock().is_some()
        || actions::confirm::has_pending()
        || actions::dialog::has_pending()
    {
        return None;
    }
    let job = CHANNEL.1.lock().try_recv().ok()?;
    if super::mcp_blocked() || job.deadline <= Instant::now() || job.control.check().is_err() {
        let _ = job
            .reply
            .try_send(json!({"error":"cancelled before execution"}));
        return None;
    }
    if job.name == "capture_screen_for_agent" {
        job.control.mark_action_attempt();
        let result = if job.args.as_object().is_some_and(|a| a.is_empty()) {
            super::vision::capture()
        } else {
            Err(super::AgentError::ToolError)
        };
        let result = result.and_then(|url| job.control.check().map(|_| url));
        let payload = match result {
            Ok(url) => json!({"image_url":url}),
            Err(e) => json!({"error":e.to_string()}),
        };
        let _ = job.reply.try_send(payload);
        return None;
    }
    match super::execute_tool(&job.name, &job.args, &job.control) {
        Ok(out) if out.chain => {
            *DEFERRED.lock() = Some(Deferred {
                reply: job.reply,
                deadline: job.deadline.min(
                    Instant::now()
                        + Duration::from_secs(
                            crate::assistant_config::get().safety.confirm_timeout_secs,
                        ),
                ),
                name: job.name,
                control: job.control,
            });
            Some(out)
        }
        Ok(out) => {
            super::remember_command(&format!("MCP {}", job.name), &out.report);
            let _ = job.reply.try_send(json!({"result":out.report}));
            None
        }
        Err(e) => {
            let _ = job.reply.try_send(json!({"error":e.to_string()}));
            None
        }
    }
}
pub fn finish(report: &str, success: bool) -> bool {
    let Some(d) = DEFERRED.lock().take() else {
        return false;
    };
    super::remember_command(&format!("MCP {}", d.name), report);
    let key = if success { "result" } else { "error" };
    let _ = d.reply.try_send(json!({key:report}));
    true
}
pub fn expire() {
    if DEFERRED
        .lock()
        .as_ref()
        .is_some_and(|d| Instant::now() >= d.deadline || d.control.check().is_err())
    {
        finish("не выполнено: истекло время ожидания подтверждения", false);
        actions::confirm::clear();
        actions::dialog::clear();
    }
}

pub fn invoke(name: &str, arguments: &Value) -> Result<Value, String> {
    let path = crate::APP_CONFIG_DIR
        .get()
        .ok_or("Нет каталога конфигурации")?
        .join(DISCOVERY_FILE);
    let d: Discovery = serde_json::from_slice(
        &std::fs::read(path).map_err(|_| "Запустите Джарвиса и включите локальные инструменты")?,
    )
    .map_err(|_| "Некорректные параметры локального подключения")?;
    let address: std::net::SocketAddr = d
        .address
        .parse()
        .map_err(|_| "Некорректный локальный адрес")?;
    if !address.ip().is_loopback() {
        return Err("Инструменты доступны только локально".into());
    }
    let c = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(125))
        .build()
        .map_err(|_| "Не удалось создать подключение")?;
    let r = c
        .post(format!("http://{}/tool", address))
        .bearer_auth(d.token)
        .json(&json!({"name":name,"arguments":arguments}))
        .send()
        .map_err(|_| "Локальные инструменты не отвечают. Проверьте результат перед повтором.")?;
    if !r.status().is_success() {
        return Err(format!(
            "Локальные инструменты: HTTP {}",
            r.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    r.take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Не удалось прочитать результат")?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Слишком большой результат".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "Некорректный результат".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_checks_and_http_origin_guards() {
        assert!(authorized("abcd", "abcd"));
        assert!(!authorized("abc", "abcd"));
        assert!(!authorized("abce", "abcd"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker =
            std::thread::spawn(move || serve(listener.accept().unwrap().0, "secret-token"));
        let mut c = TcpStream::connect(addr).unwrap();
        c.write_all(b"POST /tool HTTP/1.1\r\nAuthorization: Bearer secret-token\r\nOrigin: https://evil.example\r\nContent-Length: 2\r\n\r\n{}").unwrap();
        let mut result = String::new();
        c.read_to_string(&mut result).unwrap();
        assert!(result.contains("403"));
        worker.join().unwrap().unwrap();
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    #[test]
    fn queued_mcp_call_cannot_outlive_its_cancelled_voice_request() {
        let _confirm = actions::confirm::TEST_LOCK.lock();
        let parent = RequestControl::default();
        super::super::RUNNING.lock().push(parent.clone());
        let lease = super::super::RequestLease(parent.clone());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || serve(listener.accept().unwrap().0, "test-token"));
        let client = std::thread::spawn(move || {
            reqwest::blocking::Client::new()
                .post(format!("http://{}/tool", address))
                .bearer_auth("test-token")
                .timeout(Duration::from_secs(5))
                .json(&json!({"name":"jarvis.system_info","arguments":{"what":"uptime"}}))
                .send()
                .unwrap()
                .json::<Value>()
                .unwrap()
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let job = loop {
            if let Ok(job) = CHANNEL.1.lock().try_recv() {
                break job;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        parent.cancel();
        drop(lease);
        assert!(!super::super::mcp_blocked());
        assert!(CHANNEL.0.try_send(job).is_ok());
        process_next();
        let result = client.join().unwrap();
        server.join().unwrap().unwrap();
        assert!(
            result.get("error").is_some(),
            "cancelled queued call executed: {}",
            result
        );
    }
    #[test]
    fn disconnected_or_expired_mcp_request_cannot_be_confirmed_later() {
        let _confirm = actions::confirm::TEST_LOCK.lock();
        for cancelled in [false, true] {
            let (tx, rx) = mpsc::sync_channel(1);
            let control = RequestControl::default();
            if cancelled {
                control.cancel();
            }
            actions::confirm::request(actions::Action::Restart);
            *DEFERRED.lock() = Some(Deferred {
                reply: tx,
                deadline: if cancelled {
                    Instant::now() + Duration::from_secs(10)
                } else {
                    Instant::now()
                },
                name: "power".into(),
                control,
            });
            expire();
            assert!(!has_pending());
            assert_eq!(
                actions::confirm::answer("да"),
                actions::confirm::Answer::NoPending
            );
            assert!(rx.try_recv().unwrap().get("error").is_some());
        }
    }
    #[test]
    fn authenticated_http_dispatches_to_voice_loop_and_waits_for_confirmation() {
        let _confirm = actions::confirm::TEST_LOCK.lock();
        actions::confirm::clear();
        for dangerous in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let server =
                std::thread::spawn(move || serve(listener.accept().unwrap().0, "test-token"));
            let client = std::thread::spawn(move || {
                reqwest::blocking::Client::new()
                    .post(format!("http://{}/tool", addr))
                    .bearer_auth("test-token")
                    .timeout(Duration::from_secs(5))
                    .json(&if dangerous {
                        json!({"name":"jarvis.power","arguments":{"action":"shutdown"}})
                    } else {
                        json!({"name":"jarvis.system_info","arguments":{"what":"uptime"},"padding":"x".repeat(40 * 1024)})
                    })
                    .send()
                    .unwrap()
                    .json::<Value>()
                    .unwrap()
            });
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(out) = process_next() {
                    assert!(dangerous && out.chain && actions::confirm::has_pending());
                    assert_eq!(
                        actions::confirm::answer("нет"),
                        actions::confirm::Answer::Cancelled
                    );
                    assert!(finish("не выполнено: пользователь отменил действие", false));
                }
                if client.is_finished() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "voice loop did not dispatch the request"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            let v = client.join().unwrap();
            if dangerous {
                assert!(v.get("error").is_some());
                assert!(v.get("result").is_none());
            } else {
                assert!(v.get("result").is_some(), "{}", v);
            }
            server.join().unwrap().unwrap();
        }
    }
}
