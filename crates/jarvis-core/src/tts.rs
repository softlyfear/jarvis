// Speaking arbitrary text (LLM answers, confirmation questions).
// Backends: "http" (Jarvis New through the local voice server), "none". Calls block until speech ends so the microphone
// does not pick the assistant's own voice up as a command.

use std::time::Duration;

use crate::actions::platform;
use crate::assistant_config;

pub fn speak(text: &str) {
    speak_with(text, &|d| std::thread::sleep(d))
}

// `wait` blocks while the cloned voice plays: the listener passes one that also hears the
// wake word and cuts the speech off
pub fn speak_with(text: &str, wait: &dyn Fn(Duration)) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let cfg = &assistant_config::get().tts;
    info!("TTS ({}): {}", cfg.backend, text);

    if cfg.backend == "none" {
        platform::notify("Джарвис", text);
        return;
    }
    let result = speak_http(text, wait);

    if let Err(e) = result {
        warn!("TTS failed: {}", e);
        // at least show the text
        platform::notify("Джарвис", text);
    }
}

// WAV bytes of `text` in the cloned voice, from the local voice server
pub fn synthesize(text: &str) -> Result<Vec<u8>, String> {
    synthesize_within(text, Duration::from_secs(assistant_config::get().tts.http_timeout_secs.max(3)))
}

pub fn synthesize_within(text: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let cfg = &assistant_config::get().tts;
    let client = crate::http::client()?;
    let resp = client
        .post(&cfg.http_url)
        .timeout(timeout)
        .json(&serde_json::json!({"text": text, "language": "ru", "voice": crate::voices::current_id()}))
        .send()
        .map_err(|e| format!("TTS server unreachable: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("TTS server returned {}", resp.status()));
    }
    Ok(resp.bytes().map_err(|e| e.to_string())?.to_vec())
}

// The first sentence plays while the rest is synthesized: speech starts after one short
// sentence, not after the whole answer.
fn speak_http(text: &str, wait: &dyn Fn(Duration)) -> Result<(), String> {
    let chunks = speech_chunks(text);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for chunk in chunks {
            let result = synthesize(&chunk);
            let failed = result.is_err();
            // nobody listens any more: the reply was cut off
            if tx.send(result).is_err() || failed {
                break;
            }
        }
    });
    let cut_before = crate::audio::is_interrupted();
    for bytes in rx {
        play_prepared(&bytes?, wait)?;
        if !cut_before && crate::audio::is_interrupted() {
            break;
        }
    }
    Ok(())
}

const SHORT_REPLY: usize = 100;
const FIRST_CHUNK: usize = 40;
const MAX_CHUNK: usize = 250;

// a reply split at sentence ends: the first part short (but not a lone "Сэр."), the others
// up to MAX_CHUNK; a short reply stays whole
fn speech_chunks(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.chars().count() <= SHORT_REPLY {
        return vec![text.to_string()];
    }
    let mut sentences = Vec::new();
    let mut start = 0;
    let mut prev_end = false;
    for (i, c) in text.char_indices() {
        if prev_end && c.is_whitespace() {
            sentences.push(&text[start..i]);
            start = i;
        }
        prev_end = matches!(c, '.' | '!' | '?' | '…');
    }
    sentences.push(&text[start..]);

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    for sentence in sentences {
        let sentence = sentence.trim();
        if sentence.is_empty() {
            continue;
        }
        let limit = if chunks.is_empty() { FIRST_CHUNK } else { MAX_CHUNK };
        let fits = chunks.is_empty() || current.chars().count() + sentence.chars().count() < limit;
        if !current.is_empty() && (!fits || (chunks.is_empty() && current.chars().count() >= FIRST_CHUNK)) {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(sentence);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

// Play previously synthesized bytes after the final response has been validated.
pub fn play_prepared(bytes: &[u8], wait: &dyn Fn(Duration)) -> Result<(), String> {

    let file = tempfile::Builder::new()
        .prefix("jarvis-tts-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?;
    std::fs::write(file.path(), bytes).map_err(|e| e.to_string())?;

    let duration = wav_duration(file.path()).unwrap_or(Duration::from_secs(3));
    crate::audio::play_sound(&file.path().to_path_buf());
    // playback is asynchronous; keep the file and wait until it has been played
    wait(duration + Duration::from_millis(250));
    Ok(())
}

fn wav_duration(path: &std::path::Path) -> Option<Duration> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 {
        return None;
    }
    let frames = reader.duration() as f64;
    Some(Duration::from_secs_f64(frames / spec.sample_rate as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_reply_starts_with_a_short_sentence() {
        assert_eq!(speech_chunks(" Готово, сэр. Открыл блокнот. "), vec!["Готово, сэр. Открыл блокнот."]);
        let reply = "Сэр. Я могу искать файлы и папки по имени в ваших папках. Также я могу открыть папку в проводнике. \
                     Пустую папку так не найти, но можно посмотреть на экран, если есть ключ Google.";
        let chunks = speech_chunks(reply);
        // "Сэр." alone is too short to start with: it joins the next sentence
        assert_eq!(chunks[0], "Сэр. Я могу искать файлы и папки по имени в ваших папках.");
        assert_eq!(chunks[1..].join(" "), "Также я могу открыть папку в проводнике. Пустую папку так не найти, но можно посмотреть на экран, если есть ключ Google.");
        assert_eq!(chunks.join(" "), reply.split_whitespace().collect::<Vec<_>>().join(" "));
        // numbers and abbreviations without a space after the dot are not sentence ends
        let long = format!("Версия 0.2.80 уже стоит, сэр, обновлять не нужно. {}", "Ещё слово. ".repeat(20));
        assert_eq!(speech_chunks(&long)[0], "Версия 0.2.80 уже стоит, сэр, обновлять не нужно.");
        // nothing is longer than MAX_CHUNK unless one sentence is
        let many = "Короткое предложение номер один. ".repeat(30);
        assert!(speech_chunks(&many).iter().all(|c| c.chars().count() < MAX_CHUNK));
    }

    #[test]
    fn wav_duration_is_read() {
        let tmp = tempfile::Builder::new().suffix(".wav").tempfile().unwrap();
        let spec = hound::WavSpec { channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(tmp.path(), spec).unwrap();
        for _ in 0..8000 {
            w.write_sample(0i16).unwrap();
        }
        w.finalize().unwrap();
        let d = wav_duration(tmp.path()).unwrap();
        assert!((d.as_secs_f64() - 0.5).abs() < 0.01);
    }
}
