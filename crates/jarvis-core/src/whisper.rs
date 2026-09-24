// Whisper speech recognition through the local voice server (tools/voice-server).
// Vosk still finds the wake word and the end of an utterance; the utterance audio is
// then re-recognized by Whisper, which is far more accurate. Any failure falls back to
// the Vosk text, and a failing server is skipped for a while so commands are not delayed.

use std::io::Cursor;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::assistant_config::{self, SttConfig};

pub const SAMPLE_RATE: u32 = 16_000;
// longest utterance kept for Whisper (the server caps at 30 s too)
const MAX_SAMPLES: usize = SAMPLE_RATE as usize * 30;

static FAILED_UNTIL: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

// audio of the utterance currently being recognized
#[derive(Default)]
pub struct UtteranceBuffer {
    samples: Vec<i16>,
}

impl UtteranceBuffer {
    pub fn push(&mut self, frame: &[i16]) {
        self.samples.extend_from_slice(frame);
        if self.samples.len() > MAX_SAMPLES {
            let excess = self.samples.len() - MAX_SAMPLES;
            self.samples.drain(..excess);
        }
    }

    pub fn take(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.samples)
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

pub fn enabled() -> bool {
    enabled_with(&assistant_config::get().stt)
}

fn enabled_with(cfg: &SttConfig) -> bool {
    if cfg.engine != "whisper" {
        return false;
    }
    match *FAILED_UNTIL.lock() {
        Some(until) => Instant::now() >= until,
        None => true,
    }
}

// make Whisper output look like Vosk output: lowercase words, no punctuation
pub fn normalize(text: &str) -> String {
    let lower = text.to_lowercase();
    let cleaned: String = lower
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned
        .split_whitespace()
        .map(|w| if w == "jarvis" { "джарвис" } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn encode_wav(samples: &[i16]) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).map_err(|e| e.to_string())?;
        for s in samples {
            writer.write_sample(*s).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(cursor.into_inner())
}

// Ok(None): too short to bother; Ok(Some(text)): recognized (may be empty); Err: server problem
pub fn transcribe(samples: &[i16]) -> Result<Option<String>, String> {
    transcribe_with(&assistant_config::get().stt, samples)
}

fn transcribe_with(cfg: &SttConfig, samples: &[i16]) -> Result<Option<String>, String> {
    let min_samples = (cfg.min_audio_ms * SAMPLE_RATE as u64 / 1000) as usize;
    if samples.len() < min_samples {
        return Ok(None);
    }

    let result = request(cfg, samples);
    match &result {
        Ok(_) => *FAILED_UNTIL.lock() = None,
        Err(e) => {
            warn!("Whisper unavailable ({}), using Vosk for {} s", e, cfg.retry_after_secs);
            *FAILED_UNTIL.lock() = Some(Instant::now() + Duration::from_secs(cfg.retry_after_secs));
        }
    }
    result.map(|t| Some(normalize(&t)))
}

fn request(cfg: &SttConfig, samples: &[i16]) -> Result<String, String> {
    let wav = encode_wav(samples)?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(cfg.whisper_timeout_secs.max(1)))
        .build()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let resp = client
        .post(format!("{}?language={}", cfg.whisper_url, cfg.language))
        .header("Content-Type", "audio/wav")
        .body(wav)
        .send()
        .map_err(|e| format!("request failed: {}", e))?;
    let status = resp.status();
    let body = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{}: {}", status, body.chars().take(200).collect::<String>()));
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad JSON: {}", e))?;
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "no \"text\" in response".to_string())?
        .to_string();
    info!("Whisper ({} ms): {}", started.elapsed().as_millis(), text);
    Ok(text)
}

// pick the final text: Whisper when it produced something, otherwise Vosk
pub fn refine(vosk_text: String, audio: &[i16]) -> String {
    if vosk_text.trim().is_empty() || !enabled() {
        return vosk_text;
    }
    match transcribe(audio) {
        Ok(Some(text)) if !text.is_empty() => {
            info!("Vosk: '{}' -> Whisper: '{}'", vosk_text, text);
            text
        }
        _ => vosk_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_keeps_last_30_seconds() {
        let mut b = UtteranceBuffer::default();
        b.push(&vec![1i16; MAX_SAMPLES]);
        b.push(&[2i16; 10]);
        assert_eq!(b.len(), MAX_SAMPLES);
        let taken = b.take();
        assert_eq!(&taken[taken.len() - 10..], &[2i16; 10]);
        assert!(b.is_empty());
    }

    #[test]
    fn whisper_text_is_normalized_like_vosk() {
        assert_eq!(normalize("Джарвис, открой Телеграм!"), "джарвис открой телеграм");
        assert_eq!(normalize("Jarvis, громкость 50%."), "джарвис громкость 50");
        assert_eq!(normalize("Запусти Counter-Strike 2"), "запусти counter strike 2");
    }

    #[test]
    fn wav_is_16k_mono() {
        let wav = encode_wav(&[0, 1, -1, 32767]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 16000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len(), 4);
    }

    fn server(status: u16, body: &'static str) -> (String, std::sync::Arc<Mutex<Vec<(String, usize)>>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut req: Vec<u8> = Vec::new();
                let mut buf = vec![0u8; 65536];
                loop {
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&req).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text
                            .lines()
                            .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        if req.len() >= h + 4 + len {
                            let first = text.lines().next().unwrap_or("").to_string();
                            seen2.lock().push((first, len));
                            break;
                        }
                    }
                }
                let resp = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        (format!("http://{}/stt", addr), seen)
    }

    fn cfg(url: &str) -> SttConfig {
        SttConfig { whisper_url: url.to_string(), whisper_timeout_secs: 5, ..SttConfig::default() }
    }

    // FAILED_UNTIL is global: serialize the tests that touch it
    static NET_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    #[test]
    fn transcribes_through_server_and_resets_backoff() {
        let _g = NET_LOCK.lock();
        let (url, seen) = server(200, r#"{"text": "Открой Телеграм."}"#);
        let c = cfg(&url);
        *FAILED_UNTIL.lock() = None;

        let audio = vec![0i16; 16000];
        assert_eq!(transcribe_with(&c, &audio).unwrap(), Some("открой телеграм".to_string()));
        let (line, len) = seen.lock()[0].clone();
        assert!(line.starts_with("POST /stt?language=ru "), "{}", line);
        assert_eq!(len, 16000 * 2 + 44);

        // too short: no request at all
        assert_eq!(transcribe_with(&c, &[0i16; 100]).unwrap(), None);
        assert_eq!(seen.lock().len(), 1);
        assert!(enabled_with(&c));
    }

    #[test]
    fn failure_turns_whisper_off_for_a_while() {
        let _g = NET_LOCK.lock();
        let (url, _) = server(500, r#"{"error": "cuda"}"#);
        let c = cfg(&url);
        *FAILED_UNTIL.lock() = None;

        assert!(transcribe_with(&c, &vec![0i16; 16000]).is_err());
        assert!(!enabled_with(&c));

        *FAILED_UNTIL.lock() = None;
        assert!(enabled_with(&c));
        assert!(!enabled_with(&SttConfig { engine: "vosk".into(), ..c }));
    }
}
