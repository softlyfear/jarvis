// Speaking arbitrary text (LLM answers, confirmation questions).
// Backends: "http" (Jarvis New on a GPU through the local server), "none". Calls block until speech ends so the microphone
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

fn speak_http(text: &str, wait: &dyn Fn(Duration)) -> Result<(), String> {
    let bytes = synthesize(text)?;

    let file = tempfile::Builder::new()
        .prefix("jarvis-tts-")
        .suffix(".wav")
        .tempfile()
        .map_err(|e| e.to_string())?;
    std::fs::write(file.path(), &bytes).map_err(|e| e.to_string())?;

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
