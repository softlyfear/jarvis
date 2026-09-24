// Speaking arbitrary text (LLM answers, confirmation questions).
// Backends: "sapi" (built-in Windows voices), "http" (local voice-clone server,
// see tools/voice-server), "none". Calls block until speech ends so the microphone
// does not pick the assistant's own voice up as a command.

use std::time::Duration;

use crate::actions::platform;
use crate::assistant_config;

pub fn speak(text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let cfg = &assistant_config::get().tts;
    info!("TTS ({}): {}", cfg.backend, text);

    let result = match cfg.backend.as_str() {
        "none" => Ok(()),
        "http" => match speak_http(text) {
            Ok(()) => Ok(()),
            Err(e) if cfg.http_fallback_sapi => {
                warn!("HTTP TTS failed ({}), falling back to SAPI", e);
                speak_sapi(text)
            }
            Err(e) => Err(e),
        },
        _ => speak_sapi(text),
    };

    if let Err(e) = result {
        warn!("TTS failed: {}", e);
        // at least show the text
        platform::notify("Джарвис", text);
    }
}

fn speak_sapi(text: &str) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("SAPI is available on Windows only".into());
    }
    let cfg = &assistant_config::get().tts;
    // text and voice come through environment variables, never through the script text
    let script = r#"
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$want = $env:JARVIS_TTS_VOICE
if ($want) {
  $v = $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Name -like "*$want*" } | Select-Object -First 1
  if ($v) { $s.SelectVoice($v.VoiceInfo.Name) }
} else {
  $ru = $s.GetInstalledVoices() | Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -eq 'ru-RU' } | Select-Object -First 1
  if ($ru) { $s.SelectVoice($ru.VoiceInfo.Name) }
}
$s.Rate = [int]$env:JARVIS_TTS_RATE
$s.Speak($env:JARVIS_TTS_TEXT)
"#;
    let rate = cfg.sapi_rate.clamp(-10, 10).to_string();
    crate::audio::hold_microphone(Duration::from_secs(120));
    let result = platform::powershell(
        script,
        &[("JARVIS_TTS_TEXT", text), ("JARVIS_TTS_VOICE", cfg.sapi_voice.trim()), ("JARVIS_TTS_RATE", &rate)],
    )
    .map(|_| ());
    crate::audio::release_microphone();
    result
}

// WAV bytes of `text` in the cloned voice, from the local voice server
pub fn synthesize(text: &str) -> Result<Vec<u8>, String> {
    synthesize_within(text, Duration::from_secs(assistant_config::get().tts.http_timeout_secs.max(3)))
}

pub fn synthesize_within(text: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let cfg = &assistant_config::get().tts;
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(&cfg.http_url)
        .json(&serde_json::json!({"text": text, "language": "ru", "voice": crate::voices::current_id()}))
        .send()
        .map_err(|e| format!("TTS server unreachable: {}", e))?;
    if !resp.status().is_success() {
        return Err(format!("TTS server returned {}", resp.status()));
    }
    Ok(resp.bytes().map_err(|e| e.to_string())?.to_vec())
}

fn speak_http(text: &str) -> Result<(), String> {
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
    std::thread::sleep(duration + Duration::from_millis(250));
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
