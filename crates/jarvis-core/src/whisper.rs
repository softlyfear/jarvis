// Speech recognition through the local voice server (tools/voice-server).
// Vosk only finds the wake word; the endpointer cuts utterances and the server recognizes
// them (ServerListener). If the server fails, Vosk recognizes that utterance from its audio
// and then listens on its own while the server is skipped for a while.

use std::io::Cursor;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::assistant_config::{self, SttConfig};

pub const SAMPLE_RATE: u32 = 16_000;
// longest utterance kept for Whisper (the server caps at 30 s too)
const MAX_SAMPLES: usize = SAMPLE_RATE as usize * 30;

static FAILED_UNTIL: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

// A phrase Vosk finalized while waiting for the wake word belongs to the command only if it
// ended together with the wake word: the wake recognizer finalizes on the same pause, and the
// listener then sniffs 0.3 s. Anything older was said before "Джарвис" (to someone else).
pub const PREFED_MAX_AGE_SAMPLES: usize = SAMPLE_RATE as usize;

// audio of the utterance currently being recognized
#[derive(Default)]
pub struct UtteranceBuffer {
    samples: Vec<i16>,
    prefed_text: Option<String>,
    // samples[..prefed_end] is the audio of the last finalized phrase and the ones before it
    prefed_end: usize,
    // that phrase was cut by the endpointer and still waits for the voice server
    segment_closed: bool,
    // its last samples that are speech (the silence before it is not sent)
    segment_len: usize,
}

impl UtteranceBuffer {
    pub fn push(&mut self, frame: &[i16]) {
        self.samples.extend_from_slice(frame);
        if self.samples.len() > MAX_SAMPLES {
            let excess = self.samples.len() - MAX_SAMPLES;
            self.samples.drain(..excess);
            self.prefed_end = self.prefed_end.saturating_sub(excess);
        }
    }

    // Vosk finalized a phrase in the audio pushed so far; earlier phrases are not needed any more
    pub fn remember_prefed(&mut self, text: String) {
        if text.is_empty() { return; }
        self.samples.drain(..self.prefed_end);
        self.prefed_end = self.samples.len();
        self.prefed_text = Some(text);
    }

    // the finalized phrase if it ended at most `max_age` samples ago; a stale one is dropped with its audio
    pub fn take_prefed(&mut self, max_age: usize) -> Option<String> {
        let text = self.prefed_text.take()?;
        if self.samples.len() - self.prefed_end > max_age {
            self.samples.drain(..self.prefed_end);
            self.prefed_end = 0;
            return None;
        }
        Some(text)
    }

    // the endpointer cut a phrase here: like a Vosk final result, without its text
    pub fn close_segment(&mut self, len: usize) {
        self.samples.drain(..self.prefed_end);
        self.prefed_end = self.samples.len();
        self.prefed_text = None;
        self.segment_closed = true;
        self.segment_len = len;
    }

    // the audio of the phrase cut at most `max_age` samples ago; a stale one is dropped
    pub fn take_closed(&mut self, max_age: usize) -> Option<Vec<i16>> {
        if !std::mem::take(&mut self.segment_closed) {
            return None;
        }
        let fresh = self.samples.len() - self.prefed_end <= max_age;
        let audio: Vec<i16> = self.samples.drain(..self.prefed_end).collect();
        self.prefed_end = 0;
        fresh.then(|| tail(audio, self.segment_len))
    }

    pub fn take(&mut self) -> Vec<i16> {
        self.prefed_end = 0;
        self.segment_closed = false;
        std::mem::take(&mut self.samples)
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.prefed_text = None;
        self.prefed_end = 0;
        self.segment_closed = false;
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

// Listening while the voice server works: the endpointer cuts utterances by the pauses and
// the server recognizes them; the Vosk speech recognizer stays idle.
#[derive(Default)]
pub struct ServerListener {
    buffer: UtteranceBuffer,
    endpoint: crate::endpoint::Endpointer,
}

impl ServerListener {
    // waiting for the wake word: remember where utterances end
    pub fn feed(&mut self, frame: &[i16]) {
        self.buffer.push(frame);
        if self.endpoint.push(frame) {
            self.buffer.close_segment(self.endpoint.utterance_samples());
        }
    }

    // a command: the utterance that woke Jarvis up if it has just ended, else the next one
    pub fn next_utterance(&mut self, frame: &[i16]) -> Option<Vec<i16>> {
        self.buffer.push(frame);
        if let Some(audio) = self.buffer.take_closed(PREFED_MAX_AGE_SAMPLES) {
            return Some(audio);
        }
        self.endpoint.push(frame).then(|| tail(self.buffer.take(), self.endpoint.utterance_samples()))
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.endpoint.reset();
    }
}

fn tail(mut audio: Vec<i16>, len: usize) -> Vec<i16> {
    audio.drain(..audio.len().saturating_sub(len));
    audio
}

pub fn enabled() -> bool {
    enabled_with()
}

fn enabled_with() -> bool {
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
            warn!("Speech server unavailable ({}), using Vosk for {} s", e, cfg.retry_after_secs);
            *FAILED_UNTIL.lock() = Some(Instant::now() + Duration::from_secs(cfg.retry_after_secs));
        }
    }
    result.map(|t| Some(normalize(&t)))
}

fn request(cfg: &SttConfig, samples: &[i16]) -> Result<String, String> {
    let wav = encode_wav(samples)?;
    let client = crate::http::client()?;
    let mut url = reqwest::Url::parse(&cfg.whisper_url).map_err(|e| e.to_string())?;
    url.query_pairs_mut().append_pair("language", &cfg.language);
    let started = Instant::now();
    let resp = client
        .post(url)
        .timeout(Duration::from_secs(cfg.whisper_timeout_secs.max(1)))
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
    info!("Speech server ({} ms): {}", started.elapsed().as_millis(), text);
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalized_prefeed_preserves_text_and_audio() {
        let mut b = UtteranceBuffer::default();
        b.push(&[1, 2, 3]);
        b.remember_prefed("джарвис привет".into());
        b.push(&[4, 5]);
        assert_eq!(b.take_prefed(2).as_deref(), Some("джарвис привет"));
        assert_eq!(b.take(), vec![1, 2, 3, 4, 5]);
        b.remember_prefed("старая фраза".into()); b.clear();
        assert!(b.take_prefed(2).is_none());
    }

    #[test]
    fn a_phrase_said_before_the_wake_word_is_not_the_command() {
        let mut b = UtteranceBuffer::default();
        // "слушай" ends, then "джарвис открой браузер" is still being decoded
        b.push(&[1, 1]);
        b.remember_prefed("слушай".into());
        b.push(&[2, 2, 2, 2]);
        assert!(b.take_prefed(3).is_none());
        b.push(&[3]);
        // the stale phrase's audio is not sent to Whisper with the command
        assert_eq!(b.take(), vec![2, 2, 2, 2, 3]);

        // a later phrase replaces the earlier one with its audio; an empty final result does not
        b.push(&[1]);
        b.remember_prefed("слушай".into());
        b.push(&[2, 3]);
        b.remember_prefed("джарвис открой браузер".into());
        b.remember_prefed(String::new());
        b.push(&[4]);
        assert_eq!(b.take_prefed(1).as_deref(), Some("джарвис открой браузер"));
        assert_eq!(b.take(), vec![2, 3, 4]);
    }

    fn speech(amplitude: f32, seconds: f32) -> Vec<Vec<i16>> {
        let frames = (seconds * SAMPLE_RATE as f32) as usize / 512;
        (0..frames).map(|f| (0..512).map(|i| (amplitude * ((f * 512 + i) as f32 * 0.05).sin()) as i16).collect()).collect()
    }

    #[test]
    fn the_server_gets_the_wake_utterance_or_the_next_one() {
        // "Джарвис, открой блокнот" in one breath: cut while waiting, handed over after the wake word
        let mut l = ServerListener::default();
        for f in speech(50.0, 0.5).iter().chain(&speech(3000.0, 1.5)).chain(&speech(50.0, 0.7)) { l.feed(f); }
        let quiet = speech(50.0, 0.3);
        let audio = l.next_utterance(&quiet[0]).expect("the wake utterance");
        assert!(audio.len() >= 2 * SAMPLE_RATE as usize && audio.len() < 3 * SAMPLE_RATE as usize, "{}", audio.len());

        // an old phrase, then "Джарвис" still being said: the old one is dropped, the new one waits for its pause
        let mut l = ServerListener::default();
        for f in speech(50.0, 0.5).iter().chain(&speech(3000.0, 1.0)).chain(&speech(50.0, 2.0)).chain(&speech(3000.0, 0.5)) { l.feed(f); }
        let mut got = None;
        for f in speech(3000.0, 0.5).iter().chain(&speech(50.0, 1.0)) {
            if let Some(a) = l.next_utterance(f) { got = Some(a); break; }
        }
        let audio = got.expect("the new utterance");
        assert!(audio.len() < 2 * SAMPLE_RATE as usize, "{}", audio.len());
    }

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
        assert!(enabled_with());
    }

    #[test]
    fn failure_turns_whisper_off_for_a_while() {
        let _g = NET_LOCK.lock();
        let (url, _) = server(500, r#"{"error": "cuda"}"#);
        let c = cfg(&url);
        *FAILED_UNTIL.lock() = None;

        assert!(transcribe_with(&c, &vec![0i16; 16000]).is_err());
        assert!(!enabled_with());

        *FAILED_UNTIL.lock() = None;
        assert!(enabled_with());
    }
}
