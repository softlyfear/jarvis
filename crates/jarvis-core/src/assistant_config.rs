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
    // only the free models: providers with keys are skipped
    pub free_only: bool,
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
            free_only: false,
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
// paid ones with the account's key (app.kilo.ai -> Your Profile, one key per account)
pub const KILO_BASE_URL: &str = "https://api.kilo.ai/api/gateway";
// free models in order, by a test of Jarvis's own requests (24.09.2026, 20 phrases, 31 tools:
// Nemotron 3 Ultra 18/20 in 2.1 s, Ling 3.0 Flash 16/20 in 1.4 s but wrong at arithmetic);
// "kilo-auto/free" last picks whatever free model is alive
pub const KILO_FREE_MODELS: &[&str] = &[
    "nvidia/nemotron-3-ultra-550b-a55b:free",
    "inclusionai/ling-3.0-flash-fin:free",
    "dots-studio/dots-3-note-preview:free",
    "nvidia/nemotron-3-super-120b-a12b:free",
    "kilo-auto/free",
];
// paid models for the Kilo key, same benchmark with the key (24.09.2026, 20 phrases, 31 tools):
// Gemini 3.5 Flash-Lite 19/20 in 1.4 s (~$0.0005 a call), Gemini 3.5 Flash 20/20 in 1.9 s but
// 8 times dearer, DeepSeek V4 Flash 19/20 in 2.1 s
pub const KILO_PAID_MODELS: &[&str] = &["google/gemini-3.5-flash-lite", "google/gemini-3.5-flash", "deepseek/deepseek-v4-flash"];
// the provider block the settings window and the installer write the key into
pub const KILO_PROVIDER: &str = "kilo";
// written by a build of 24.09.2026 for the same key
const LEGACY_PAID_PROVIDER: &str = "paid";

// Google's own API, used by versions before Kilo
fn is_gemini_block(name: &str, base_url: &str) -> bool {
    name.eq_ignore_ascii_case("gemini") || base_url.contains("generativelanguage.googleapis.com")
}

// a key copied from the Kilo profile page may come wrapped over several lines
pub fn clean_key(key: &str) -> String {
    key.chars().filter(|c| !c.is_whitespace()).collect()
}

impl LlmConfig {
    // Gemini blocks of older versions are dropped: Kilo is the only gateway
    fn without_gemini(mut self) -> Self {
        self.providers.retain(|p| !is_gemini_block(&p.name, &p.base_url));
        self
    }

    // configs written before the setting get the free models too: they answer when there is
    // no key, the key has no credits left (402) or the paid models time out
    fn with_free_fallback(mut self) -> Self {
        if self.free_fallback && !self.providers.iter().any(|p| p.keyless && p.base_url.contains("kilo.ai")) {
            self.providers.push(LlmProvider {
                name: "kilo-free".into(),
                base_url: KILO_BASE_URL.into(),
                models: KILO_FREE_MODELS.iter().map(|m| m.to_string()).collect(),
                keyless: true,
                ..LlmProvider::default()
            });
        }
        self
    }

    // the providers to ask, in order
    pub fn active_providers(&self) -> Vec<&LlmProvider> {
        self.providers.iter().filter(|p| p.enabled && (!self.free_only || p.keyless)).collect()
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
    // "Джарвис" said over a long reply stops it and listens for the next command
    pub barge_in: bool,
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
            barge_in: true,
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
    config.llm = config.llm.without_gemini().with_free_fallback();
    config.llm.extra_prompt = without_address_rule(&config.llm.extra_prompt);
    Ok(config)
}

// Old versions put the address into extra_prompt ("Обращайся к пользователю «мисс», …"): it
// overrode [assistant] address, so the model kept saying the old word after a switch
pub fn without_address_rule(prompt: &str) -> String {
    let mut out = prompt.to_string();
    while let Some(start) = out.find("Обращайся к пользователю").or_else(|| out.find("обращайся к пользователю")) {
        let Some(close) = out[start..].find('»').map(|i| start + i + '»'.len_utf8()) else { break };
        let end = close + out[close..].find(|c: char| !matches!(c, ',' | '.' | ';' | ' ')).unwrap_or(out.len() - close);
        out.replace_range(start..end, "");
    }
    let out = out.trim();
    // "отвечай коротко…" left at the start becomes "Отвечай коротко…"
    let mut chars = out.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
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
    // the Kilo key for paid models; empty = free models only
    #[serde(default)]
    pub kilo_key: String,
    #[serde(default)]
    pub free_only: bool,
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

fn is_kilo_block(name: &str) -> bool {
    name.eq_ignore_ascii_case(KILO_PROVIDER) || name == LEGACY_PAID_PROVIDER
}

pub fn read_editable_from(p: &std::path::Path) -> Result<EditableSettings, String> {
    ensure_file(p)?;
    let c = parse(&fs::read_to_string(p).map_err(|e| e.to_string())?)?;
    let kilo_key = c
        .llm
        .providers
        .iter()
        .filter(|p| !p.keyless && is_kilo_block(&p.name))
        .find_map(|p| p.keys.iter().find(|k| !k.trim().is_empty()).cloned())
        .unwrap_or_default();
    Ok(EditableSettings {
        kilo_key,
        free_only: c.llm.free_only,
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

    let kilo_key = clean_key(&s.kilo_key);
    if !kilo_key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err("ключ Kilo: только латинские буквы, цифры и символы . _ -".into());
    }

    let llm = doc.entry("llm").or_insert(Item::Table(Table::new()));
    let llm = llm.as_table_mut().ok_or("[llm] is not a table")?;
    let providers = llm
        .entry("providers")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or("llm.providers is not an array of tables")?;

    // the Kilo block goes first: with credits the paid models answer; without them (402) the key
    // rests and the free models take over
    let is_kilo = |t: &Table| t.get("name").and_then(|n| n.as_str()).is_some_and(is_kilo_block);
    let old = providers.iter().find(|t| is_kilo(t)).cloned();
    // Gemini blocks of older versions are removed from the file with their keys
    let is_gemini = |t: &Table| {
        let s = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        is_gemini_block(&s("name"), &s("base_url"))
    };
    let others: Vec<Table> = providers.iter().filter(|t| !is_kilo(t) && !is_gemini(t)).cloned().collect();
    // the older "paid" block had another model order: take the current one
    let legacy = old.as_ref().is_some_and(|t| t.get("name").and_then(|n| n.as_str()) == Some(LEGACY_PAID_PROVIDER));
    let mut t = old.unwrap_or_default();
    if legacy {
        t.remove("models");
    }
    t["name"] = value(KILO_PROVIDER);
    t["enabled"] = value(true);
    t["base_url"] = value(KILO_BASE_URL);
    if !t.contains_key("models") {
        let mut models = Array::new();
        for m in KILO_PAID_MODELS {
            models.push(*m);
        }
        t["models"] = value(models);
    }
    let mut keys = Array::new();
    if !kilo_key.is_empty() {
        keys.push(kilo_key.as_str());
    }
    t["keys"] = value(keys);
    // tables print by their position in the file, not by the array order: hand the existing
    // positions out again in the new order (a new block shares the first one and wins the tie)
    let mut positions: Vec<Option<isize>> = providers.iter().filter(|t| !is_gemini(t)).map(|t| t.position()).collect();
    positions.sort_by_key(|p| (p.is_none(), *p));
    if positions.len() < others.len() + 1 {
        positions.insert(0, positions.first().copied().flatten());
    }
    let mut ordered = toml_edit::ArrayOfTables::new();
    for (mut t, pos) in std::iter::once(t).chain(others).zip(positions) {
        t.set_position(pos);
        ordered.push(t);
    }
    *providers = ordered;
    llm["free_only"] = value(s.free_only);
    if let Some(extra) = llm.get("extra_prompt").and_then(|v| v.as_str()) {
        let clean = without_address_rule(extra);
        if clean != extra {
            llm["extra_prompt"] = value(clean);
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
        assert_eq!(c.llm.providers[0].name, KILO_PROVIDER);
        assert_eq!(c.llm.providers[0].models, KILO_PAID_MODELS.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        assert_eq!((last.name.as_str(), last.keyless), ("kilo-free", true));
        assert_eq!(last.models.last().map(|m| m.as_str()), Some("kilo-auto/free"));

        // an old config (Gemini only): Gemini is dropped, the free models come in, only once
        let old = "[[llm.providers]]\nname = \"gemini\"\nbase_url = \"https://generativelanguage.googleapis.com/v1beta/openai\"\nmodels = [\"auto\"]\nkeys = [\"AIzaOld\"]\n";
        let names = |text: &str| parse(text).unwrap().llm.providers.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(old), vec!["kilo-free"]);
        let own = format!("{}\n[[llm.providers]]\nname = \"my\"\nbase_url = \"{}\"\nkeyless = true\n", old, KILO_BASE_URL);
        assert_eq!(names(&own), vec!["my"]);
        let off = format!("[llm]\nfree_fallback = false\n{}", old);
        assert!(names(&off).is_empty());
    }

    #[test]
    fn free_only_skips_providers_with_keys() {
        let mut c = parse(DEFAULT_TEMPLATE).unwrap().llm;
        assert_eq!(c.active_providers().len(), 2);
        c.free_only = true;
        let names: Vec<&str> = c.active_providers().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["kilo-free"]);
    }

    #[test]
    fn kilo_key_goes_first_and_gemini_leaves_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        // a config of an older version: Gemini with a key and its comment, the "paid" block of 24.09.2026
        let old = format!(
            "[llm]\nenabled = true\n\n# Нейросеть — Google Gemini. Ключи:\n#   https://aistudio.google.com/apikey\n\n[[llm.providers]]\nname = \"gemini\"\nbase_url = \"g\"\nmodels = [\"auto\"]\nkeys = [\"AIzaOld\"]\n\n\
             [[llm.providers]]\nname = \"paid\"\nbase_url = \"{}\"\nmodels = [\"deepseek/deepseek-v4-flash\"]\nkeys = [\"eyJold\"]\n",
            KILO_BASE_URL
        );
        fs::write(&p, old).unwrap();
        assert_eq!(read_editable_from(&p).unwrap().kilo_key, "eyJold");

        let base = EditableSettings { stt_engine: "whisper".into(), tts_backend: "http".into(), ..Default::default() };
        // the key comes wrapped over lines, as copied from the profile page
        write_editable_to(&p, &EditableSettings { kilo_key: " eyJhb\r\nGci.Oi-J_9 \n".into(), free_only: true, ..base.clone() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        let names: Vec<&str> = c.llm.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["kilo", "kilo-free"]);
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.to_lowercase().contains("gemini\"") && !text.contains("AIzaOld") && !text.contains("Google Gemini"), "Gemini must leave the file: {}", text);
        assert_eq!(c.llm.providers[0].keys, vec!["eyJhbGci.Oi-J_9"]);
        assert_eq!(c.llm.providers[0].models, KILO_PAID_MODELS.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        assert!(c.llm.free_only);
        assert_eq!(read_editable_from(&p).unwrap(), EditableSettings { kilo_key: "eyJhbGci.Oi-J_9".into(), free_only: true, address: "сэр".into(), ..base.clone() });

        // models edited by hand survive; an empty key leaves the block without keys
        let text = fs::read_to_string(&p).unwrap().replace("\"google/gemini-3.5-flash-lite\", ", "");
        fs::write(&p, text).unwrap();
        write_editable_to(&p, &base).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.llm.providers[0].models[0], "google/gemini-3.5-flash");
        assert!(c.llm.providers[0].keys.is_empty());
        assert!(!c.llm.free_only);
        assert!(write_editable_to(&p, &EditableSettings { kilo_key: "ключ\"".into(), ..base }).is_err());
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
            EditableSettings { stt_engine: "whisper".into(), tts_backend: "sapi".into(), address: "сэр".into(), ..Default::default() }
        );

        let new = EditableSettings {
            kilo_key: "eyJkey".into(),
            stt_engine: "vosk".into(),
            tts_backend: "http".into(),
            address: "Мисс".into(),
            ..Default::default()
        };
        write_editable_to(&p, &new).unwrap();
        let back = read_editable_from(&p).unwrap();
        assert_eq!(back.kilo_key, "eyJkey");
        assert_eq!(back.stt_engine, "vosk");
        assert_eq!(back.tts_backend, "http");
        assert_eq!(back.address, "мисс");
        assert_eq!(parse(&fs::read_to_string(&p).unwrap()).unwrap().assistant.address, "мисс");

        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("# Нейросеть — шлюз Kilo"), "comments must survive");
        assert!(text.contains("\"браузер\" = \"https://ya.ru\""));
        assert!(write_editable_to(&p, &EditableSettings { stt_engine: "x".into(), ..new.clone() }).is_err());
        assert!(write_editable_to(&p, &EditableSettings { address: "a\"b".into(), ..new.clone() }).is_err());
        // an empty address (old window) keeps the file's value
        write_editable_to(&p, &EditableSettings { address: String::new(), ..new.clone() }).unwrap();
        assert_eq!(read_editable_from(&p).unwrap().address, "мисс");
    }

    #[test]
    fn old_address_rule_leaves_extra_prompt() {
        assert_eq!(
            without_address_rule("Обращайся к пользователю «мисс», отвечай коротко и с лёгкой иронией, как Джарвис."),
            "Отвечай коротко и с лёгкой иронией, как Джарвис."
        );
        assert_eq!(without_address_rule("Будь вежлив. Обращайся к пользователю «сэр»."), "Будь вежлив.");
        assert_eq!(without_address_rule("Обращайся к пользователю «мисс»"), "");
        assert_eq!(without_address_rule("Отвечай коротко."), "Отвечай коротко.");
        // an unclosed quote is left alone instead of looping
        assert_eq!(without_address_rule("Обращайся к пользователю «мисс"), "Обращайся к пользователю «мисс");

        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        let old = DEFAULT_TEMPLATE.replace(
            "extra_prompt = \"Отвечай",
            "extra_prompt = \"Обращайся к пользователю «мисс», отвечай",
        );
        assert!(old.contains("«мисс»"));
        fs::write(&p, &old).unwrap();
        assert_eq!(parse(&old).unwrap().llm.extra_prompt, "Отвечай коротко и с лёгкой иронией, как Джарвис.");
        let s = read_editable_from(&p).unwrap();
        write_editable_to(&p, &s).unwrap();
        assert!(!fs::read_to_string(&p).unwrap().contains("«мисс»"));
    }

    #[test]
    fn missing_kilo_block_is_added() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[llm]\nenabled = true\n").unwrap();
        write_editable_to(&p, &EditableSettings { kilo_key: "K".into(), stt_engine: "whisper".into(), tts_backend: "none".into(), ..Default::default() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.llm.providers[0].keys, vec!["K"]);
        assert_eq!(c.llm.providers[0].base_url, KILO_BASE_URL);
        assert_eq!(c.tts.backend, "none");
    }
}
