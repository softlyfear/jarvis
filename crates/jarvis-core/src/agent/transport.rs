// Dropping an asynchronous request closes its response on cancellation, including stalled bodies.
use super::{AgentError, AgentEvent, RequestControl};
use crate::agent_config::AgentConfig;
use serde_json::Value;
use std::time::Duration;

pub(super) fn post(
    cfg: &AgentConfig,
    url: &str,
    body: &Value,
    model: &str,
    timeout: Duration,
    control: &RequestControl,
    emit: &dyn Fn(AgentEvent),
) -> Result<Vec<u8>, AgentError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| AgentError::ConnectionError)?;
    rt.block_on(async {
        let mut builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(cfg.openclaw.connect_timeout_secs))
            .redirect(reqwest::redirect::Policy::none());
        if reqwest::Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .is_some_and(|h| ["localhost", "127.0.0.1", "[::1]", "::1"].contains(&h.as_str()))
        {
            builder = builder.no_proxy();
        }
        let client = builder.build().map_err(|_| AgentError::ConnectionError)?;
        let mut request = client.post(url).json(body).timeout(timeout);
        if !cfg.openclaw.api_key.is_empty() {
            request = request.bearer_auth(&cfg.openclaw.api_key);
        }
        if !model.is_empty() {
            request = request.header("x-openclaw-model", model);
        }
        let cancelled = async {
            loop {
                if control.check().is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        };
        let receive = async {
            let mut response = request.send().await.map_err(http_error)?;
            if !response.status().is_success() {
                return Err(super::openclaw::status_error(response.status().as_u16()));
            }
            let mut bytes = Vec::new();
            let mut scanned = 0;
            while let Some(chunk) = response.chunk().await.map_err(http_error)? {
                if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                    return Err(AgentError::InvalidResponse);
                }
                bytes.extend_from_slice(&chunk);
                if cfg.openclaw.streaming {
                    // Only preview text is exposed before the complete response is validated.
                    while let Some(offset) = bytes[scanned..].iter().position(|b| *b == b'\n') {
                        let end = scanned + offset;
                        if let Ok(line) = std::str::from_utf8(&bytes[scanned..end]) {
                            if let Some(data) = line.trim().strip_prefix("data:") {
                                if data.trim() == "[DONE]" {
                                    return Ok(bytes);
                                }
                                if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                                    if let Some(text) = v
                                        .pointer("/choices/0/delta/content")
                                        .and_then(Value::as_str)
                                    {
                                        emit(AgentEvent::TextDelta(text.into()));
                                    }
                                }
                            }
                        }
                        scanned = end + 1;
                    }
                }
            }
            Ok(bytes)
        };
        tokio::select! { biased;
            _ = cancelled => Err(AgentError::Cancelled),
            result = receive => result,
        }
    })
}
fn http_error(e: reqwest::Error) -> AgentError {
    if e.is_connect() {
        AgentError::ConnectionError
    } else if e.is_timeout() {
        AgentError::Timeout
    } else {
        AgentError::ProviderError
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Write};
    #[test]
    fn cancellation_disconnects_waiting_headers_and_stalled_stream_body() {
        for body_started in [false, true] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!(
                "http://{}/v1/chat/completions",
                listener.local_addr().unwrap()
            );
            let (tx, rx) = std::sync::mpsc::channel();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(socket.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(n) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = n.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if body_started {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"hello.\"}}]}\n\n").unwrap();
                }
                tx.send(()).unwrap();
                // A dropped async response must release the connection before its long timeout.
                let mut byte = [0];
                assert_eq!(reader.read(&mut byte).unwrap(), 0);
            });
            let control = RequestControl::default();
            let worker = control.clone();
            let request = std::thread::spawn(move || {
                let mut cfg = AgentConfig::default();
                cfg.openclaw.streaming = body_started;
                post(
                    &cfg,
                    &url,
                    &serde_json::json!({}),
                    "",
                    Duration::from_secs(30),
                    &worker,
                    &|_| {},
                )
            });
            rx.recv_timeout(Duration::from_secs(3)).unwrap();
            let at = std::time::Instant::now();
            control.cancel();
            assert_eq!(request.join().unwrap(), Err(AgentError::Cancelled));
            assert!(at.elapsed() < Duration::from_secs(1));
            server.join().unwrap();
        }
    }
}
