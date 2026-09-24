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
    pub assistant: PersonaConfig,
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
pub struct PersonaConfig {
    // how Jarvis addresses the user: "сэр", "мисс" or any word
    pub address: String,
}

impl Default for PersonaConfig {
    fn default() -> Self {
        Self { address: DEFAULT_ADDRESS.into() }
    }
}

pub const DEFAULT_ADDRESS: &str = "сэр";

// the address from the settings, "сэр" when empty
pub fn address() -> String {
    normalize_address(&get().assistant.address)
}

fn normalize_address(a: &str) -> String {
    let a = a.trim();
    if a.is_empty() { DEFAULT_ADDRESS.into() } else { a.to_lowercase() }
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
    // Kilo's free models (no key) after the configured providers, see with_free_fallback
    pub free_fallback: bool,
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
            free_fallback: true,
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

// Kilo gateway: OpenAI-compatible, free models without a key (200 requests an hour per IP),
// "kilo-auto/free" picks the best free model that supports tools
pub const KILO_BASE_URL: &str = "https://api.kilo.ai/api/gateway";
pub const KILO_FREE_MODEL: &str = "kilo-auto/free";

impl LlmConfig {
    // configs written before the setting get Kilo too: it answers when Gemini has no keys,
    // is blocked or times out
    fn with_free_fallback(mut self) -> Self {
        if self.free_fallback && !self.providers.iter().any(|p| p.base_url.contains("kilo.ai")) {
            self.providers.push(LlmProvider {
                name: "kilo".into(),
                base_url: KILO_BASE_URL.into(),
                models: vec![KILO_FREE_MODEL.into()],
                keyless: true,
                ..LlmProvider::default()
            });
        }
        self
    }
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
    // Notepad and PowerShell 5 may save UTF-8 with a BOM, which TOML does not allow
    let mut config: AssistantConfig = toml::from_str(content.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())?;
    config.llm = config.llm.with_free_fallback();
    Ok(config)
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

// ### EDITING FROM THE GUI
// Only a few user-facing values are edited, in place, keeping the comments of the file.

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Default)]
pub struct EditableSettings {
    pub gemini_keys: Vec<String>,
    // "whisper" | "vosk"
    pub stt_engine: String,
    // "http" | "sapi" | "none"
    pub tts_backend: String,
    // "сэр" | "мисс" | any word; empty = keep the file as is
    #[serde(default)]
    pub address: String,
}

fn ensure_file(p: &std::path::Path) -> Result<(), String> {
    if !p.exists() {
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        fs::write(p, DEFAULT_TEMPLATE).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn read_editable_from(p: &std::path::Path) -> Result<EditableSettings, String> {
    ensure_file(p)?;
    let c = parse(&fs::read_to_string(p).map_err(|e| e.to_string())?)?;
    let gemini_keys = c
        .llm
        .providers
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case("gemini"))
        .map(|p| p.keys.iter().filter(|k| !k.trim().is_empty()).cloned().collect())
        .unwrap_or_default();
    Ok(EditableSettings {
        gemini_keys,
        stt_engine: c.stt.engine,
        tts_backend: c.tts.backend,
        address: normalize_address(&c.assistant.address),
    })
}

pub fn write_editable_to(p: &std::path::Path, s: &EditableSettings) -> Result<(), String> {
    use toml_edit::{value, Array, DocumentMut, Item, Table};

    if !["whisper", "vosk"].contains(&s.stt_engine.as_str()) {
        return Err(format!("unknown stt engine: {}", s.stt_engine));
    }
    if !["http", "sapi", "none"].contains(&s.tts_backend.as_str()) {
        return Err(format!("unknown tts backend: {}", s.tts_backend));
    }
    ensure_file(p)?;
    let text = fs::read_to_string(p).map_err(|e| e.to_string())?;
    let mut doc: DocumentMut = text.trim_start_matches('\u{feff}').parse().map_err(|e: toml_edit::TomlError| e.to_string())?;

    let mut keys = Array::new();
    for k in &s.gemini_keys {
        let k = k.trim();
        if !k.is_empty() && !keys.iter().any(|v| v.as_str() == Some(k)) {
            keys.push(k);
        }
    }

    // [[llm.providers]] with name = "gemini": update keys, or add the block if it is missing
    let llm = doc.entry("llm").or_insert(Item::Table(Table::new()));
    let llm = llm.as_table_mut().ok_or("[llm] is not a table")?;
    let providers = llm
        .entry("providers")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or("llm.providers is not an array of tables")?;
    let existing = providers
        .iter_mut()
        .find(|t| t.get("name").and_then(|n| n.as_str()).map(|n| n.eq_ignore_ascii_case("gemini")).unwrap_or(false));
    match existing {
        Some(t) => {
            t["keys"] = value(keys);
        }
        None => {
            let mut t = Table::new();
            t["name"] = value("gemini");
            t["enabled"] = value(true);
            t["base_url"] = value("https://generativelanguage.googleapis.com/v1beta/openai");
            let mut models = Array::new();
            models.push("auto");
            t["models"] = value(models);
            t["keys"] = value(keys);
            providers.push(t);
        }
    }

    doc.entry("stt").or_insert(Item::Table(Table::new()))["engine"] = value(s.stt_engine.as_str());
    doc.entry("tts").or_insert(Item::Table(Table::new()))["backend"] = value(s.tts_backend.as_str());
    let address = s.address.trim();
    if !address.is_empty() {
        if address.chars().count() > 30 || address.contains(['"', '\n']) {
            return Err(format!("bad address: {}", address));
        }
        doc.entry("assistant").or_insert(Item::Table(Table::new()))["address"] = value(address.to_lowercase());
    }

    let out = doc.to_string();
    // never write a file Jarvis itself cannot read
    parse(&out)?;
    fs::write(p, out).map_err(|e| e.to_string())
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
    fn free_kilo_models_come_after_the_configured_providers() {
        let c = parse(DEFAULT_TEMPLATE).unwrap();
        let last = c.llm.providers.last().unwrap();
        assert_eq!(c.llm.providers[0].name, "gemini");
        assert_eq!((last.name.as_str(), last.keyless), ("kilo", true));
        assert_eq!(last.models, vec![KILO_FREE_MODEL.to_string()]);

        // an old config without the setting gets it too, only once
        let old = "[[llm.providers]]\nname = \"gemini\"\nbase_url = \"https://generativelanguage.googleapis.com/v1beta/openai\"\nmodels = [\"auto\"]\nkeys = []\n";
        assert_eq!(parse(old).unwrap().llm.providers.len(), 2);
        let own = format!("{}\n[[llm.providers]]\nname = \"my\"\nbase_url = \"{}\"\nkeyless = true\n", old, KILO_BASE_URL);
        assert_eq!(parse(&own).unwrap().llm.providers.len(), 2);
        let off = format!("[llm]\nfree_fallback = false\n{}", old);
        assert_eq!(parse(&off).unwrap().llm.providers.len(), 1);
    }

    #[test]
    fn bom_is_ignored() {
        let with_bom = format!("\u{feff}{}", DEFAULT_TEMPLATE);
        assert!(parse(&with_bom).is_ok());
    }

    #[test]
    fn expand_env_replaces_known_vars() {
        std::env::set_var("JARVIS_TEST_VAR", "C:\\Users\\me");
        assert_eq!(expand_env("%JARVIS_TEST_VAR%\\Desktop"), "C:\\Users\\me\\Desktop");
        assert_eq!(expand_env("100% sure"), "100% sure");
        assert_eq!(expand_env("%NO_SUCH_VAR_123%"), "%NO_SUCH_VAR_123%");
    }
    #[test]
    fn editable_settings_round_trip_keeps_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sub").join("assistant.toml");
        let s = read_editable_from(&p).unwrap();
        assert_eq!(
            s,
            EditableSettings { gemini_keys: vec![], stt_engine: "whisper".into(), tts_backend: "sapi".into(), address: "сэр".into() }
        );

        let new = EditableSettings {
            gemini_keys: vec![" AIzaA ".into(), "AIzaB".into(), "AIzaA".into(), "".into()],
            stt_engine: "vosk".into(),
            tts_backend: "http".into(),
            address: "Мисс".into(),
        };
        write_editable_to(&p, &new).unwrap();
        let back = read_editable_from(&p).unwrap();
        assert_eq!(back.gemini_keys, vec!["AIzaA", "AIzaB"]);
        assert_eq!(back.stt_engine, "vosk");
        assert_eq!(back.tts_backend, "http");
        assert_eq!(back.address, "мисс");
        assert_eq!(parse(&fs::read_to_string(&p).unwrap()).unwrap().assistant.address, "мисс");

        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("# Нейросеть — Google Gemini"), "comments must survive");
        assert!(text.contains("\"браузер\" = \"https://ya.ru\""));
        assert!(write_editable_to(&p, &EditableSettings { stt_engine: "x".into(), ..new.clone() }).is_err());
        assert!(write_editable_to(&p, &EditableSettings { address: "a\"b".into(), ..new.clone() }).is_err());
        // an empty address (old window) keeps the file's value
        write_editable_to(&p, &EditableSettings { address: String::new(), ..new.clone() }).unwrap();
        assert_eq!(read_editable_from(&p).unwrap().address, "мисс");
    }

    #[test]
    fn missing_gemini_block_is_added() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[llm]\nenabled = true\n").unwrap();
        write_editable_to(&p, &EditableSettings { gemini_keys: vec!["K".into()], stt_engine: "whisper".into(), tts_backend: "none".into(), address: String::new() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.llm.providers[0].keys, vec!["K"]);
        assert_eq!(c.llm.providers[0].models, vec!["auto"]);
        assert_eq!(c.tts.backend, "none");
    }
}
