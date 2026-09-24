// User-editable assistant settings (assistant.toml in the config directory):
// LLM providers and keys, TTS backend, app aliases, safety rules.
// Kept separate from app.db so that it can be edited by hand.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use once_cell::sync::OnceCell;
use serde::Deserialize;

use crate::APP_CONFIG_DIR;

pub const FILE_NAME: &str = "assistant.toml";

// written on first launch, commented so it can be filled in by hand
pub const DEFAULT_TEMPLATE: &str = include_str!("../assets/assistant.default.toml");

static CONFIG: OnceCell<AssistantConfig> = OnceCell::new();

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct AssistantConfig {
    pub llm: LlmConfig,
    pub stt: SttConfig,
    pub voice_server: VoiceServerConfig,
    pub tts: TtsConfig,
    pub safety: SafetyConfig,
    // spoken name -> program path, shortcut, URL or URI
    pub apps: HashMap<String, String>,
    // spoken name -> process name (without .exe), used by "close"
    pub processes: HashMap<String, Vec<String>>,
    // spoken name -> folder path
    pub folders: HashMap<String, String>,
    // spoken name -> Steam game title or appid
    pub games: HashMap<String, String>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct LlmConfig {
    pub enabled: bool,
    pub timeout_secs: u64,
    pub max_tokens: u32,
    pub temperature: f32,
    // extra instructions appended to the system prompt
    pub extra_prompt: String,
    // how long the dialog history is kept between requests
    pub memory_minutes: u64,
    pub providers: Vec<LlmProvider>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 20,
            max_tokens: 400,
            temperature: 0.3,
            extra_prompt: String::new(),
            memory_minutes: 5,
            providers: Vec::new(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct LlmProvider {
    pub name: String,
    pub enabled: bool,
    // OpenAI-compatible base URL, "/chat/completions" is appended
    pub base_url: String,
    // tried in order until one answers
    pub models: Vec<String>,
    // rotated round-robin, a key is skipped for a while after 429/401/403
    pub keys: Vec<String>,
    // allow requests without a key (local Ollama / LM Studio)
    pub keyless: bool,
}

impl Default for LlmProvider {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            base_url: String::new(),
            models: Vec::new(),
            keys: Vec::new(),
            keyless: false,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct VoiceServerConfig {
    // start tools/voice-server together with Jarvis (only if it has been installed by setup.bat)
    pub autostart: bool,
    // extra command-line arguments, e.g. ["--no-tts"]
    pub args: Vec<String>,
    pub health_url: String,
}

impl Default for VoiceServerConfig {
    fn default() -> Self {
        Self {
            autostart: true,
            args: Vec::new(),
            health_url: "http://127.0.0.1:5055/health".into(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SttConfig {
    // "whisper" (local voice server, falls back to Vosk) | "vosk"
    pub engine: String,
    // POST audio/wav (16 kHz mono) -> {"text": "..."}
    pub whisper_url: String,
    pub whisper_timeout_secs: u64,
    pub language: String,
    // shorter utterances are left to Vosk (noise, clicks)
    pub min_audio_ms: u64,
    // after a failure, Whisper is skipped for this long so commands are not delayed
    pub retry_after_secs: u64,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            engine: "whisper".into(),
            whisper_url: "http://127.0.0.1:5055/stt".into(),
            whisper_timeout_secs: 10,
            language: "ru".into(),
            min_audio_ms: 300,
            retry_after_secs: 30,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct TtsConfig {
    // "none" | "sapi" | "http"
    pub backend: String,
    // SAPI voice name substring, empty = system default
    pub sapi_voice: String,
    // SAPI rate, -10..10
    pub sapi_rate: i32,
    // local TTS server (see tools/voice-server), POST {"text": "..."} -> audio/wav
    pub http_url: String,
    pub http_timeout_secs: u64,
    // fall back to SAPI when the HTTP server is unavailable
    pub http_fallback_sapi: bool,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            backend: "sapi".into(),
            sapi_voice: String::new(),
            sapi_rate: 1,
            http_url: "http://127.0.0.1:5055/tts".into(),
            http_timeout_secs: 30,
            http_fallback_sapi: true,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct SafetyConfig {
    // ask "yes/no" before deleting files, shutdown, restart, etc.
    pub confirm_dangerous: bool,
    // files can be found/deleted only inside these folders (env vars like %USERPROFILE% are expanded)
    pub allowed_dirs: Vec<String>,
    // seconds to wait for "yes/no"
    pub confirm_timeout_secs: u64,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            confirm_dangerous: true,
            allowed_dirs: vec![
                "%USERPROFILE%\\Desktop".into(),
                "%USERPROFILE%\\Downloads".into(),
                "%USERPROFILE%\\Documents".into(),
                "%USERPROFILE%\\Pictures".into(),
                "%USERPROFILE%\\Music".into(),
                "%USERPROFILE%\\Videos".into(),
            ],
            confirm_timeout_secs: 20,
        }
    }
}

pub fn path() -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join(FILE_NAME))
}

pub fn init() {
    if CONFIG.get().is_some() {
        return;
    }

    let config = match path() {
        Some(p) => {
            if !p.exists() {
                match fs::write(&p, DEFAULT_TEMPLATE) {
                    Ok(_) => info!("Created default assistant config: {}", p.display()),
                    Err(e) => warn!("Cannot create {}: {}", p.display(), e),
                }
            }
            load_from(&p)
        }
        None => {
            warn!("Config dir is not set, using default assistant config");
            parse(DEFAULT_TEMPLATE).unwrap_or_default()
        }
    };

    info!(
        "Assistant config: llm={} ({} provider(s), {} key(s)), stt={}, tts={}",
        config.llm.enabled,
        config.llm.providers.iter().filter(|p| p.enabled).count(),
        config.llm.providers.iter().map(|p| p.keys.iter().filter(|k| !k.trim().is_empty()).count()).sum::<usize>(),
        config.stt.engine,
        config.tts.backend
    );

    let _ = CONFIG.set(config);
}

fn load_from(p: &PathBuf) -> AssistantConfig {
    match fs::read_to_string(p) {
        Ok(content) => match parse(&content) {
            Ok(c) => c,
            Err(e) => {
                // do not silently run with half of the settings
                error!("Invalid {}: {}. Using defaults.", p.display(), e);
                parse(DEFAULT_TEMPLATE).unwrap_or_default()
            }
        },
        Err(e) => {
            warn!("Cannot read {}: {}", p.display(), e);
            parse(DEFAULT_TEMPLATE).unwrap_or_default()
        }
    }
}

pub fn parse(content: &str) -> Result<AssistantConfig, String> {
    toml::from_str(content).map_err(|e| e.to_string())
}

pub fn get() -> &'static AssistantConfig {
    static EMPTY: once_cell::sync::Lazy<AssistantConfig> =
        once_cell::sync::Lazy::new(|| parse(DEFAULT_TEMPLATE).unwrap_or_default());
    CONFIG.get().unwrap_or(&EMPTY)
}

// expand %VAR% (Windows style) and $VAR / ${VAR}
pub fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '%') {
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                if !name.is_empty() {
                    if let Ok(val) = std::env::var(&name) {
                        out.push_str(&val);
                        i += end + 2;
                        continue;
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn allowed_dirs() -> Vec<PathBuf> {
    get()
        .safety
        .allowed_dirs
        .iter()
        .map(|d| PathBuf::from(expand_env(d)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_template_parses() {
        let c = parse(DEFAULT_TEMPLATE).expect("default template must be valid TOML");
        assert!(c.llm.enabled);
        assert!(!c.llm.providers.is_empty());
        assert!(c.safety.confirm_dangerous);
        assert!(!c.apps.is_empty());
        assert_eq!(c.stt.engine, "whisper");
        assert!(c.stt.whisper_url.ends_with("/stt"));
    }

    #[test]
    fn expand_env_replaces_known_vars() {
        std::env::set_var("JARVIS_TEST_VAR", "C:\\Users\\me");
        assert_eq!(expand_env("%JARVIS_TEST_VAR%\\Desktop"), "C:\\Users\\me\\Desktop");
        assert_eq!(expand_env("100% sure"), "100% sure");
        assert_eq!(expand_env("%NO_SUCH_VAR_123%"), "%NO_SUCH_VAR_123%");
    }
}
