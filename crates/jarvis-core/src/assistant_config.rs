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
    pub vision: VisionConfig,
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
    // sent only when set: Gemini 3 degrades below its default of 1.0
    pub temperature: Option<f32>,
    // how much the model thinks before answering ("none", "low", …; empty — its own default):
    // a voice reply waits for every reasoning token
    pub reasoning: String,
    // extra instructions appended to the system prompt
    pub extra_prompt: String,
    // how long the dialog history is kept between requests
    pub memory_minutes: u64,
    pub providers: Vec<LlmProvider>,
    // OpenRouter's free models (the same key) after the paid provider, see with_free_fallback
    pub free_fallback: bool,
    // only the free provider and any keyless custom endpoint
    pub free_only: bool,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_secs: 20,
            max_tokens: 400,
            temperature: None,
            reasoning: "none".into(),
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

// OpenRouter: OpenAI-compatible chat completions. The rolling DeepSeek V4 Flash alias is the
// paid model (catalog checked 2026-11-06: id supports tools). Keys from openrouter.ai/settings/keys
// start with "sk-or-".
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const OPENROUTER_PROVIDER: &str = "openrouter";
pub const OPENROUTER_PAID_MODELS: &[&str] = &["~deepseek/deepseek-v4-flash-latest"];
// Free variants use the same key. Checked 2026-11-06: these ids support tools.
// That day a :free model allowed 20 requests a minute and 50 a day until the account had bought
// $10 of credits, then 1000 a day. A negative balance returns 402 even for :free.
// "openrouter/free" last asks OpenRouter to pick a live free model.
pub const OPENROUTER_FREE_MODELS: &[&str] = &[
    "nvidia/nemotron-3-ultra-550b-a55b:free",
    "dots-studio/dots-3-note-preview:free",
    "openrouter/free",
];
const OPENROUTER_FREE_PROVIDER: &str = "openrouter-free";
// Upgrade only the old default order, preserving deliberately customized model lists.
const LEGACY_PAID_MODELS: &[&[&str]] = &[
    &["google/gemini-3.5-flash-lite", "google/gemini-3.5-flash", "deepseek/deepseek-v4-flash"],
    &["anthropic/claude-haiku-5.5", "google/gemini-3.5-flash-lite", "google/gemini-3.5-flash", "deepseek/deepseek-v4-flash"],
    // the single model of the "paid" block from 24.09.2026
    &["deepseek/deepseek-v4-flash"],
];

// Polza AI: a Russian gateway reachable without a VPN, paid in rubles; keys "sk-polza-…" from polza.ai
pub const POLZA_BASE_URL: &str = "https://polza.ai/api/v1";
pub const POLZA_PROVIDER: &str = "polza";
// Polza does not publish the rolling alias; use its supported V4 Flash model.
pub const POLZA_MODELS: &[&str] = &["deepseek/deepseek-v4-flash"];

// Kilo was the previous gateway. Its blocks are retargeted; a JWT ("eyJ…") is not an OpenRouter key.
const LEGACY_KILO_PROVIDER: &str = "kilo";
const LEGACY_KILO_FREE_PROVIDER: &str = "kilo-free";

// which gateway a pasted key belongs to. A leftover Kilo JWT is neither of the current gateways.
pub fn gateway_of_key(key: &str) -> &'static str {
    let key = clean_key(key);
    if key.starts_with("sk-or-") {
        OPENROUTER_PROVIDER
    } else if key.starts_with("eyJ") {
        LEGACY_KILO_PROVIDER
    } else {
        POLZA_PROVIDER
    }
}
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
    // Gemini blocks of older versions are dropped
    fn without_gemini(mut self) -> Self {
        self.providers.retain(|p| !is_gemini_block(&p.name, &p.base_url));
        self
    }

    // Kilo blocks become OpenRouter. A Kilo JWT cannot call OpenRouter, so it is dropped.
    // The old keyless free list called Kilo and is removed; free models are added later, with the key.
    fn migrate_from_kilo(mut self) -> Self {
        self.providers.retain(|p| {
            !p.name.eq_ignore_ascii_case(LEGACY_KILO_FREE_PROVIDER)
                && !(p.keyless && p.base_url.contains("kilo.ai"))
        });
        for p in &mut self.providers {
            if is_openrouter_block(&p.name) && !p.keyless {
                p.name = OPENROUTER_PROVIDER.into();
                p.base_url = OPENROUTER_BASE_URL.into();
                p.keys.retain(|key| usable_openrouter_key(key));
            }
        }
        self
    }

    fn with_current_paid_models(mut self) -> Self {
        for p in &mut self.providers {
            if (is_openrouter_block(&p.name) || p.name.eq_ignore_ascii_case(POLZA_PROVIDER)) &&
                !p.keyless && LEGACY_PAID_MODELS.iter().any(|old| p.models.iter().map(String::as_str).eq(old.iter().copied())) {
                let models = if p.name.eq_ignore_ascii_case(POLZA_PROVIDER) { POLZA_MODELS } else { OPENROUTER_PAID_MODELS };
                p.models = models.iter().map(|m| m.to_string()).collect();
            }
        }
        self
    }

    // Free OpenRouter models need the account key. They are a separate provider, so a 402 on the
    // paid model does not put the free models to sleep. No key means no free fallback: OpenRouter
    // does not answer without one.
    fn with_free_fallback(mut self) -> Self {
        if self.free_fallback && !self.providers.iter().any(|p| p.name.eq_ignore_ascii_case(OPENROUTER_FREE_PROVIDER)) {
            let keys: Vec<String> = self
                .providers
                .iter()
                .find(|p| p.name.eq_ignore_ascii_case(OPENROUTER_PROVIDER) && !p.keyless)
                .map(|p| p.keys.iter().filter(|key| usable_openrouter_key(key)).cloned().collect())
                .unwrap_or_default();
            if !keys.is_empty() {
                self.providers.push(LlmProvider {
                    name: OPENROUTER_FREE_PROVIDER.into(),
                    base_url: OPENROUTER_BASE_URL.into(),
                    models: OPENROUTER_FREE_MODELS.iter().map(|m| m.to_string()).collect(),
                    keys,
                    keyless: false,
                    ..LlmProvider::default()
                });
            }
        }
        self
    }

    // the providers to ask, in order
    pub fn active_providers(&self) -> Vec<&LlmProvider> {
        self.providers.iter().filter(|p| p.enabled && (!self.free_only || is_free_provider(p))).collect()
    }
}

fn is_free_provider(provider: &LlmProvider) -> bool {
    provider.keyless || provider.name.eq_ignore_ascii_case(OPENROUTER_FREE_PROVIDER)
}

fn usable_openrouter_key(key: &str) -> bool {
    let key = clean_key(key);
    !key.is_empty() && !key.starts_with("eyJ")
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
    // POST audio/wav (16 kHz mono) -> {"text": "..."}
    pub whisper_url: String,
    pub whisper_timeout_secs: u64,
    pub language: String,
    // shorter utterances are not recognized (noise, clicks)
    pub min_audio_ms: u64,
    // after a failure, Whisper is skipped for this long so commands are not delayed
    pub retry_after_secs: u64,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self {
            whisper_url: "http://127.0.0.1:5055/stt".into(),
            whisper_timeout_secs: 10,
            language: "ru".into(),
            min_audio_ms: 300,
            retry_after_secs: 30,
        }
    }
}

// Looking at the screen: a free Google AI Studio key, its own; without it the LLM cannot see
#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct VisionConfig {
    pub google_key: String,
    // OpenAI-compatible endpoint of the Gemini API
    pub base_url: String,
    pub timeout_secs: u64,
}

pub const VISION_MODEL: &str = "gemini-flash-lite-latest";

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            google_key: String::new(),
            base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(),
            timeout_secs: 25,
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default)]
pub struct TtsConfig {
    // "none" | "http" (GPU voice only)
    pub backend: String,
    // local TTS server (see tools/voice-server), POST {"text": "..."} -> audio/wav
    pub http_url: String,
    pub http_timeout_secs: u64,
    // "Джарвис" said over a long reply stops it and listens for the next command
    pub barge_in: bool,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            backend: "http".into(),
            http_url: "http://127.0.0.1:5055/tts".into(),
            http_timeout_secs: 30,
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
        "Assistant config: llm={} ({} provider(s), {} key(s)), stt=whisper, tts={}",
        config.llm.enabled,
        config.llm.providers.iter().filter(|p| p.enabled).count(),
        config.llm.providers.iter().map(|p| p.keys.iter().filter(|k| !k.trim().is_empty()).count()).sum::<usize>(),
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
    let content = content.trim_start_matches('\u{feff}');
    let mut config: AssistantConfig = toml::from_str(content).map_err(|e: toml::de::Error| config_parse_error(content, e.span()))?;
    config.llm = config.llm.without_gemini().migrate_from_kilo().with_current_paid_models().with_free_fallback();
    config.llm.extra_prompt = without_address_rule(&config.llm.extra_prompt);
    // 0.3 was the template value of older versions, not a choice of the user
    if config.llm.temperature.is_some_and(|t| (t - 0.3).abs() < 1e-6) {
        config.llm.temperature = None;
    }
    // Migrate the retired Windows voice; "none" remains an explicit user choice.
    if config.tts.backend != "none" { config.tts.backend = "http".into(); }
    config.voice_server.args = gpu_voice_args(&config.voice_server.args);
    Ok(config)
}

// Discard retired TTS flags while keeping speech recognition options from old configs.
fn gpu_voice_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if ["--voice", "--tts-engine"].contains(&arg.as_str()) {
            it.next();
        } else if arg.starts_with("--voice=") || arg.starts_with("--tts-engine=") || arg == "--device=cpu" {
            continue;
        } else if arg == "--device" {
            if let Some(device) = it.next() {
                if device != "cpu" { result.extend([arg.clone(), device.clone()]); }
            }
        } else { result.push(arg.clone()); }
    }
    result
}

// Parser Display includes the offending source line, which may contain API keys.
fn config_parse_error(content: &str, span: Option<std::ops::Range<usize>>) -> String {
    let before = content.get(..span.map(|s| s.start).unwrap_or(0)).unwrap_or("");
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    format!("Некорректный assistant.toml: строка {}, столбец {}. Проверьте формат и типы значений.", line, column)
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
        if chars[i] == '$' {
            let braced = chars.get(i + 1) == Some(&'{');
            let start = i + if braced { 2 } else { 1 };
            let end = if braced {
                chars[start..].iter().position(|c| *c == '}').map(|n| start + n)
            } else {
                Some(start + chars[start..].iter().take_while(|c| c.is_ascii_alphanumeric() || **c == '_').count())
            };
            if let Some(end) = end {
                let name: String = chars[start..end].iter().collect();
                if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                    if let Ok(value) = std::env::var(&name) {
                        out.push_str(&value);
                        i = end + usize::from(braced);
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
    // the OpenRouter key; empty = this gateway is not called. A Kilo JWT is not accepted.
    #[serde(default, alias = "kilo_key")]
    pub openrouter_key: String,
    // the Polza AI key; empty = no Polza block
    #[serde(default)]
    pub polza_key: String,
    // "openrouter" | "polza": asked first, the other one is the fallback. "kilo" and empty mean OpenRouter.
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub free_only: bool,
    // "http" | "none"
    pub tts_backend: String,
    // "сэр" | "мисс" | any word; empty = keep the file as is
    #[serde(default)]
    pub address: String,
    // Google AI Studio key for looking at the screen; empty = no vision
    #[serde(default)]
    pub vision_key: String,
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

fn is_openrouter_block(name: &str) -> bool {
    name.eq_ignore_ascii_case(OPENROUTER_PROVIDER)
        || name.eq_ignore_ascii_case(LEGACY_KILO_PROVIDER)
        || name == LEGACY_PAID_PROVIDER
}

pub fn read_editable_from(p: &std::path::Path) -> Result<EditableSettings, String> {
    ensure_file(p)?;
    let c = parse(&fs::read_to_string(p).map_err(|e| e.to_string())?)?;
    let key_of = |is_block: &dyn Fn(&str) -> bool| {
        c.llm
            .providers
            .iter()
            .filter(|p| !p.keyless && is_block(&p.name))
            .find_map(|p| p.keys.iter().find(|k| !k.trim().is_empty()).cloned())
            .unwrap_or_default()
    };
    let openrouter_key = c
        .llm
        .providers
        .iter()
        .filter(|p| !p.keyless && is_openrouter_block(&p.name))
        .find_map(|p| p.keys.iter().find(|key| usable_openrouter_key(key)).cloned())
        .unwrap_or_default();
    let polza_key = key_of(&|n: &str| n.eq_ignore_ascii_case(POLZA_PROVIDER));
    // the gateway whose block comes first
    let gateway = c
        .llm
        .providers
        .iter()
        .find_map(|p| {
            if p.keyless {
                None
            } else if is_openrouter_block(&p.name) {
                Some(OPENROUTER_PROVIDER)
            } else if p.name.eq_ignore_ascii_case(POLZA_PROVIDER) {
                Some(POLZA_PROVIDER)
            } else {
                None
            }
        })
        .unwrap_or(OPENROUTER_PROVIDER)
        .to_string();
    Ok(EditableSettings {
        openrouter_key,
        polza_key,
        gateway,
        free_only: c.llm.free_only,
        tts_backend: c.tts.backend,
        address: normalize_address(&c.assistant.address),
        vision_key: c.vision.google_key,
    })
}

pub fn write_editable_to(p: &std::path::Path, s: &EditableSettings) -> Result<(), String> {
    use toml_edit::{value, Array, DocumentMut, Item, Table};

    if !["http", "none"].contains(&s.tts_backend.as_str()) {
        return Err(format!("unknown tts backend: {}", s.tts_backend));
    }
    ensure_file(p)?;
    let text = fs::read_to_string(p).map_err(|e| e.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    // Reject invalid table shapes before indexing the editable document.
    parse(text)?;
    let mut doc: DocumentMut = text.parse().map_err(|e: toml_edit::TomlError| config_parse_error(text, e.span()))?;
    // Preserve hand-edited settings and comments outside the fields exposed by the GUI.
    // OpenClaw was removed (10.10.2026): its block goes away with the Gateway token.
    doc.remove("agent");

    let openrouter_key = clean_key(&s.openrouter_key);
    let polza_key = clean_key(&s.polza_key);
    let vision_key = clean_key(&s.vision_key);
    // A Kilo JWT is not a key for either gateway. Reject it before the file is rewritten.
    if openrouter_key.starts_with("eyJ") || polza_key.starts_with("eyJ") {
        return Err("это ключ Kilo (eyJ…). Для OpenRouter создайте ключ на openrouter.ai — он начинается с sk-or-. Для Polza — ключ с polza.ai.".into());
    }
    for (key, gateway) in [(&openrouter_key, "OpenRouter"), (&polza_key, "Polza AI"), (&vision_key, "Google AI Studio")] {
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
            return Err(format!("ключ {}: только латинские буквы, цифры и символы . _ -", gateway));
        }
    }
    let polza_first = s.gateway == POLZA_PROVIDER;

    let llm = doc.entry("llm").or_insert(Item::Table(Table::new()));
    let llm = llm.as_table_mut().ok_or("[llm] is not a table")?;
    let providers = llm
        .entry("providers")
        .or_insert(Item::ArrayOfTables(toml_edit::ArrayOfTables::new()))
        .as_array_of_tables_mut()
        .ok_or("llm.providers is not an array of tables")?;

    let name_of = |t: &Table| t.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    let is_router = |t: &Table| is_openrouter_block(&name_of(t));
    let is_polza = |t: &Table| name_of(t).eq_ignore_ascii_case(POLZA_PROVIDER);
    let is_retired_free = |t: &Table| {
        name_of(t).eq_ignore_ascii_case(LEGACY_KILO_FREE_PROVIDER)
            || (t.get("keyless").and_then(|v| v.as_bool()) == Some(true)
                && t.get("base_url").and_then(|v| v.as_str()).unwrap_or("").contains("kilo.ai"))
    };
    // Gemini blocks of older versions are removed from the file with their keys
    let is_gemini = |t: &Table| {
        let s = |k: &str| t.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        is_gemini_block(&s("name"), &s("base_url"))
    };
    let others: Vec<Table> = providers.iter().filter(|t| !is_router(t) && !is_polza(t) && !is_gemini(t) && !is_retired_free(t)).cloned().collect();

    // a gateway block with its key; models the user edited stay
    let block = |old: Option<Table>, name: &str, base_url: &str, models: &[&str], key: &str| {
        let mut t = old.unwrap_or_default();
        // Replace only a known default list. A hand-edited list, including one on the old "paid" block, stays.
        let old_defaults = t.get("keyless").and_then(|v| v.as_bool()) != Some(true) &&
            t.get("models").and_then(|v| v.as_array()).is_some_and(|a|
            LEGACY_PAID_MODELS.iter().any(|old| a.len() == old.len() && a.iter().zip(*old).all(|(v, m)| v.as_str() == Some(*m))));
        if old_defaults {
            t.remove("models");
        }
        t["name"] = value(name);
        t["enabled"] = value(true);
        t["base_url"] = value(base_url);
        if !t.contains_key("models") {
            let mut list = Array::new();
            for m in models {
                list.push(*m);
            }
            t["models"] = value(list);
        }
        // The GUI edits the first key only. Saving unrelated settings must retain rotation keys.
        let unchanged = t.get("keys").and_then(|v| v.as_array()).and_then(|a| a.get(0)).and_then(|v| v.as_str()).is_some_and(|old| clean_key(old) == key);
        if !unchanged {
            let mut keys = Array::new();
            if !key.is_empty() { keys.push(key); }
            t["keys"] = value(keys);
        }
        t
    };
    let openrouter = block(
        providers.iter().find(|t| is_router(t) && t.get("keyless").and_then(|v| v.as_bool()) != Some(true)).cloned(),
        OPENROUTER_PROVIDER,
        OPENROUTER_BASE_URL,
        OPENROUTER_PAID_MODELS,
        &openrouter_key,
    );
    let old_polza = providers.iter().find(|t| is_polza(t)).cloned();
    let polza = (old_polza.is_some() || !polza_key.is_empty() || polza_first)
        .then(|| block(old_polza, POLZA_PROVIDER, POLZA_BASE_URL, POLZA_MODELS, &polza_key));
    // the chosen gateway is asked first. A 402 rests that key; the other gateway, then the free
    // OpenRouter models (same key, separate provider) take over.
    let gateways: Vec<Table> =
        if polza_first { polza.into_iter().chain(Some(openrouter)).collect() } else { Some(openrouter).into_iter().chain(polza).collect() };

    // tables print by their position in the file, not by the array order: hand the existing
    // positions out again in the new order (new blocks share the first one and win the tie)
    let mut positions: Vec<Option<isize>> = providers.iter().filter(|t| !is_gemini(t)).map(|t| t.position()).collect();
    positions.sort_by_key(|p| (p.is_none(), *p));
    while positions.len() < gateways.len() + others.len() {
        positions.insert(0, positions.first().copied().flatten());
    }
    let mut ordered = toml_edit::ArrayOfTables::new();
    for (mut t, pos) in gateways.into_iter().chain(others).zip(positions) {
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

    if let Some(stt) = doc.get_mut("stt").and_then(Item::as_table_mut) { stt.remove("engine"); }
    doc.entry("tts").or_insert(Item::Table(Table::new()))["backend"] = value(s.tts_backend.as_str());
    let vision = doc.entry("vision").or_insert(Item::Table(Table::new())).as_table_mut().ok_or("[vision] is not a table")?;
    vision["google_key"] = value(vision_key.as_str());
    if let Some(tts) = doc["tts"].as_table_mut() {
        for key in ["sapi_voice", "sapi_rate", "http_fallback_sapi"] { tts.remove(key); }
    }
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
    crate::storage::atomic_write(p, out.as_bytes()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_cpu_and_voice_flags_leave_stt_options_intact() {
        let c = parse(r#"[voice_server]
args = ["--tts-engine", "xtts", "--voice=voice", "--voice", "old", "--device", "cpu", "--device=cpu", "--whisper-device", "cpu", "--no-tts"]
"#).unwrap();
        assert_eq!(c.voice_server.args, ["--whisper-device", "cpu", "--no-tts"]);
        let c = parse(r#"[voice_server]
args = ["--device", "cuda:1", "--whisper-model", "medium"]
"#).unwrap();
        assert_eq!(c.voice_server.args, ["--device", "cuda:1", "--whisper-model", "medium"]);
    }

    #[test]
    fn retired_sapi_settings_use_the_gpu_voice() {
        let config = parse("[tts]\nbackend = \"sapi\"\nsapi_rate = 2\nhttp_fallback_sapi = true\n").unwrap();
        assert_eq!(config.tts.backend, "http");
        assert_eq!(parse("[tts]\nbackend = \"none\"\n").unwrap().tts.backend, "none");
    }

    #[test]
    fn invalid_config_errors_hide_values_and_writes_leave_original_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        for text in ["[llm]\nenabled = \"private-secret-token\"", "stt = \"private-secret-token\"", "keys = [\"private-secret-token\""] {
            let error = parse(text).unwrap_err();
            assert!(!error.contains("private-secret-token"));
            fs::write(&p, text).unwrap();
            let settings = EditableSettings { tts_backend: "http".into(), ..Default::default() };
            assert!(write_editable_to(&p, &settings).is_err());
            assert_eq!(fs::read_to_string(&p).unwrap(), text);
        }
    }

    #[test]
    fn saving_other_settings_retains_additional_gateway_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[[llm.providers]]\nname = 'kilo'\nkeys = ['sk-or-first', 'sk-or-second']\n").unwrap();
        let mut settings = read_editable_from(&p).unwrap();
        settings.address = "мисс".into();
        write_editable_to(&p, &settings).unwrap();
        let saved = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(saved.llm.providers[0].name, OPENROUTER_PROVIDER);
        assert_eq!(saved.llm.providers[0].keys, vec!["sk-or-first", "sk-or-second"]);
    }

    #[test]
    fn saving_drops_the_removed_openclaw_block_with_its_token() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[agent]\nbackend = 'openclaw'\n[agent.openclaw]\napi_key = 'gateway-secret'\n[tts]\nbackend = 'http'\n").unwrap();
        let settings = read_editable_from(&p).unwrap();
        write_editable_to(&p, &settings).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.contains("[agent") && !text.contains("gateway-secret"), "{}", text);
    }

    #[test]
    fn the_vision_key_is_saved_in_its_own_block() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[tts]\nbackend = 'http'\n").unwrap();
        let mut settings = read_editable_from(&p).unwrap();
        assert_eq!(settings.vision_key, "");
        settings.vision_key = " AIzaSyTest_key-1 \n".into();
        write_editable_to(&p, &settings).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.vision.google_key, "AIzaSyTest_key-1");
        assert_eq!(VISION_MODEL, "gemini-flash-lite-latest");
        settings.vision_key = "ключ с пробелами".into();
        assert!(write_editable_to(&p, &settings).is_err());
    }

    #[test]
    fn default_template_parses() {
        let c = parse(DEFAULT_TEMPLATE).expect("default template must be valid TOML");
        assert!(c.llm.enabled);
        assert!(!c.llm.providers.is_empty());
        assert!(c.safety.confirm_dangerous);
        assert!(!c.apps.is_empty());
        assert!(c.stt.whisper_url.ends_with("/stt"));
    }

    #[test]
    fn old_vision_model_lists_are_ignored_on_upgrade() {
        let c = parse("[vision]\nmodels = ['gemini-3.8-flash', 'gemini-2.5-flash']\ntimeout_secs = 40\ngoogle_key = 'AIza-test'\n").unwrap();
        assert_eq!(c.vision.google_key, "AIza-test");
        assert_eq!(VISION_MODEL, "gemini-flash-lite-latest");
    }

    #[test]
    fn temperature_is_sent_only_when_the_user_chose_it() {
        assert_eq!(parse(DEFAULT_TEMPLATE).unwrap().llm.temperature, None);
        // the template value of older versions is dropped
        assert_eq!(parse("[llm]\ntemperature = 0.3\n").unwrap().llm.temperature, None);
        assert_eq!(parse("[llm]\ntemperature = 0.7\n").unwrap().llm.temperature, Some(0.7));
    }

    #[test]
    fn openrouter_free_models_follow_a_key_and_kilo_does_not() {
        let c = parse(DEFAULT_TEMPLATE).unwrap();
        assert_eq!(c.llm.providers[0].name, OPENROUTER_PROVIDER);
        assert_eq!(c.llm.providers[0].base_url, OPENROUTER_BASE_URL);
        assert_eq!(OPENROUTER_PAID_MODELS, ["~deepseek/deepseek-v4-flash-latest"]);
        assert_eq!(POLZA_MODELS, ["deepseek/deepseek-v4-flash"]);
        assert_eq!(c.llm.providers[0].models, OPENROUTER_PAID_MODELS.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        // no key yet: OpenRouter has nothing to call, including the free models
        assert_eq!(c.llm.providers.len(), 1);

        let keyed = "[[llm.providers]]\nname = \"kilo\"\nbase_url = \"https://api.kilo.ai/api/gateway\"\nmodels = [\"~deepseek/deepseek-v4-flash-latest\"]\nkeys = [\"sk-or-live\", \"eyJdead\"]\n\n[[llm.providers]]\nname = \"kilo-free\"\nbase_url = \"https://api.kilo.ai/api/gateway\"\nkeyless = true\nmodels = [\"kilo-auto/free\"]\n";
        let migrated = parse(keyed).unwrap();
        assert_eq!(migrated.llm.providers[0].name, OPENROUTER_PROVIDER);
        assert_eq!(migrated.llm.providers[0].base_url, OPENROUTER_BASE_URL);
        assert_eq!(migrated.llm.providers[0].keys, vec!["sk-or-live"]);
        let free = migrated.llm.providers.last().unwrap();
        assert_eq!((free.name.as_str(), free.keyless), (OPENROUTER_FREE_PROVIDER, false));
        assert_eq!(free.keys, vec!["sk-or-live"]);
        assert_eq!(free.models.last().map(String::as_str), Some("openrouter/free"));
        assert!(!migrated.llm.providers.iter().any(|p| p.base_url.contains("kilo.ai") || p.name.contains("kilo")));

        // an old config (Gemini only): Gemini is dropped, and there is no keyless fallback
        let old = "[[llm.providers]]\nname = \"gemini\"\nbase_url = \"https://generativelanguage.googleapis.com/v1beta/openai\"\nmodels = [\"auto\"]\nkeys = [\"AIzaOld\"]\n";
        let names = |text: &str| parse(text).unwrap().llm.providers.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert!(names(old).is_empty());
        let own = format!("{old}\n[[llm.providers]]\nname = \"my\"\nbase_url = \"http://127.0.0.1:9/v1\"\nkeyless = true\n");
        assert_eq!(names(&own), vec!["my"]);
        let off = format!("[llm]\nfree_fallback = false\n{keyed}");
        assert_eq!(names(&off), vec![OPENROUTER_PROVIDER]);
    }

    #[test]
    fn old_gateway_defaults_upgrade_on_load_and_save() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("assistant.toml");
        for old in LEGACY_PAID_MODELS {
            for name in [LEGACY_KILO_PROVIDER, POLZA_PROVIDER, LEGACY_PAID_PROVIDER] {
                let models = if name == POLZA_PROVIDER { POLZA_MODELS } else { OPENROUTER_PAID_MODELS };
                let text = format!("[llm]\nfree_only = true\n[[llm.providers]]\nname = '{name}'\nmodels = {old:?}\nkeys = ['first', 'second']\n# Keep this comment\n[assistant]\naddress = 'мисс'\n");
                let loaded = parse(&text).unwrap();
                assert_eq!(loaded.llm.providers[0].models, models);
                assert!(loaded.llm.free_only);
                assert_eq!(loaded.llm.providers[0].keys, ["first", "second"]);
                fs::write(&path, &text).unwrap();
                let settings = read_editable_from(&path).unwrap();
                write_editable_to(&path, &settings).unwrap();
                let saved = fs::read_to_string(&path).unwrap();
                let raw: AssistantConfig = toml::from_str(&saved).unwrap();
                assert_eq!(raw.llm.providers[0].models, models);
                assert_eq!(raw.llm.providers[0].keys, ["first", "second"]);
                assert_eq!(raw.assistant.address, "мисс");
                assert!(saved.contains("# Keep this comment"));
                write_editable_to(&path, &settings).unwrap();
                assert_eq!(fs::read_to_string(&path).unwrap(), saved);
            }
        }
    }

    #[test]
    fn paid_model_migration_preserves_custom_providers_and_lists() {
        let old_models = format!("{:?}", LEGACY_PAID_MODELS[0]);
        for (name, models, keyless) in [
            ("custom", old_models.as_str(), false),
            (LEGACY_KILO_PROVIDER, old_models.as_str(), true),
            (POLZA_PROVIDER, "['google/gemini-3.5-flash']", false),
            (LEGACY_KILO_PROVIDER, "[]", false),
            (LEGACY_KILO_PROVIDER, "['deepseek/deepseek-v4-flash', 'google/gemini-3.5-flash-lite']", false),
        ] {
            let text = format!("[[llm.providers]]\nname = '{name}'\nmodels = {models}\nkeyless = {keyless}\n");
            let raw: AssistantConfig = toml::from_str(&text).unwrap();
            assert_eq!(parse(&text).unwrap().llm.providers[0].models, raw.llm.providers[0].models);
        }
    }

    #[test]
    fn free_only_keeps_the_free_provider_and_a_keyless_endpoint() {
        let keyed = "[[llm.providers]]\nname = 'openrouter'\nkeys = ['sk-or-live']\nmodels = ['~deepseek/deepseek-v4-flash-latest']\n[[llm.providers]]\nname = 'polza'\nkeys = ['sk-polza-1']\n[[llm.providers]]\nname = 'my'\nkeyless = true\n";
        let mut c = parse(keyed).unwrap().llm;
        assert_eq!(c.active_providers().len(), 4);
        c.free_only = true;
        let names: Vec<&str> = c.active_providers().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["my", OPENROUTER_FREE_PROVIDER]);
        let mut bare = parse(DEFAULT_TEMPLATE).unwrap().llm;
        bare.free_only = true;
        assert!(bare.active_providers().is_empty());
    }

    #[test]
    fn openrouter_key_goes_first_and_gemini_leaves_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        // a config of an older version: Gemini with a key and its comment, the "paid" block of 24.09.2026
        let old = "\
[llm]\nenabled = true\n\n# Нейросеть — Google Gemini. Ключи:\n#   https://aistudio.google.com/apikey\n\n[[llm.providers]]\nname = \"gemini\"\nbase_url = \"g\"\nmodels = [\"auto\"]\nkeys = [\"AIzaOld\"]\n\n\
[[llm.providers]]\nname = \"paid\"\nbase_url = \"https://api.kilo.ai/api/gateway\"\nmodels = [\"deepseek/deepseek-v4-flash\"]\nkeys = [\"eyJold\"]\n";
        fs::write(&p, old).unwrap();
        // the Kilo JWT is not an OpenRouter key, so the field stays empty until a real one is pasted
        assert_eq!(read_editable_from(&p).unwrap().openrouter_key, "");

        let base = EditableSettings { tts_backend: "http".into(), ..Default::default() };
        assert!(write_editable_to(&p, &EditableSettings { openrouter_key: "eyJhbGci.Oi-J_9".into(), ..base.clone() }).is_err());
        // the key comes wrapped over lines, as copied from the keys page
        write_editable_to(&p, &EditableSettings { openrouter_key: " sk-or-v1-\r\nabc.def_9 \n".into(), free_only: true, ..base.clone() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        let names: Vec<&str> = c.llm.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec![OPENROUTER_PROVIDER, OPENROUTER_FREE_PROVIDER]);
        let text = fs::read_to_string(&p).unwrap();
        assert!(!text.to_lowercase().contains("gemini\"") && !text.contains("AIzaOld") && !text.contains("Google Gemini"), "Gemini must leave the file: {}", text);
        assert!(!text.contains("kilo.ai"), "{}", text);
        assert_eq!(c.llm.providers[0].keys, vec!["sk-or-v1-abc.def_9"]);
        assert_eq!(c.llm.providers[0].models, OPENROUTER_PAID_MODELS.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        assert!(c.llm.free_only);
        assert_eq!(
            read_editable_from(&p).unwrap(),
            EditableSettings { openrouter_key: "sk-or-v1-abc.def_9".into(), free_only: true, address: "сэр".into(), gateway: OPENROUTER_PROVIDER.into(), ..base.clone() }
        );

        // models edited by hand survive; an empty key leaves the block without keys
        let text = fs::read_to_string(&p).unwrap()
            .replace(OPENROUTER_PAID_MODELS[0], "google/gemini-3.5-flash");
        fs::write(&p, text).unwrap();
        write_editable_to(&p, &base).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.llm.providers[0].models[0], "google/gemini-3.5-flash");
        assert!(c.llm.providers[0].keys.is_empty());
        assert!(!c.llm.free_only);
        assert!(write_editable_to(&p, &EditableSettings { openrouter_key: "ключ\"".into(), ..base }).is_err());
    }

    #[test]
    fn saving_keeps_a_hand_edited_paid_list_and_rejects_a_kilo_jwt_for_polza() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(
            &p,
            "[[llm.providers]]\nname = \"paid\"\nbase_url = \"https://api.kilo.ai/api/gateway\"\nmodels = [\"google/gemini-3.5-flash\"]\nkeys = [\"sk-or-keep\"]\n",
        )
        .unwrap();
        let base = EditableSettings { tts_backend: "http".into(), openrouter_key: "sk-or-keep".into(), ..Default::default() };
        write_editable_to(&p, &base).unwrap();
        let saved = fs::read_to_string(&p).unwrap();
        let c = parse(&saved).unwrap();
        assert_eq!(c.llm.providers[0].name, OPENROUTER_PROVIDER);
        assert_eq!(c.llm.providers[0].base_url, OPENROUTER_BASE_URL);
        assert_eq!(c.llm.providers[0].models, vec!["google/gemini-3.5-flash".to_string()]);
        assert!(!saved.contains("kilo.ai"), "{saved}");

        let err = write_editable_to(
            &p,
            &EditableSettings { polza_key: "eyJhbGci.Oi-J_9".into(), gateway: "polza".into(), ..base },
        )
        .unwrap_err();
        assert!(err.contains("eyJ"), "{err}");
        assert_eq!(fs::read_to_string(&p).unwrap(), saved);
    }

    #[test]
    fn polza_goes_first_and_openrouter_stays_as_the_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, DEFAULT_TEMPLATE).unwrap();
        let base = EditableSettings { tts_backend: "http".into(), ..Default::default() };

        let s = EditableSettings { polza_key: "sk-polza-abc\n".into(), openrouter_key: "sk-or-live".into(), gateway: "polza".into(), ..base.clone() };
        write_editable_to(&p, &s).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        let names: Vec<&str> = c.llm.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["polza", OPENROUTER_PROVIDER, OPENROUTER_FREE_PROVIDER]);
        assert_eq!(c.llm.providers[0].base_url, POLZA_BASE_URL);
        assert_eq!(c.llm.providers[0].keys, vec!["sk-polza-abc"]);
        assert_eq!(c.llm.providers[0].models, POLZA_MODELS.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        assert_eq!(c.llm.providers[1].base_url, OPENROUTER_BASE_URL);
        let back = read_editable_from(&p).unwrap();
        assert_eq!((back.gateway.as_str(), back.polza_key.as_str(), back.openrouter_key.as_str()), ("polza", "sk-polza-abc", "sk-or-live"));
        // the comments of the template survive
        assert!(fs::read_to_string(&p).unwrap().contains("# Нейросеть — шлюз OpenRouter"));

        // back to OpenRouter: it goes first, Polza keeps its key as the fallback.
        // An old window that still says "kilo" means the same choice.
        write_editable_to(&p, &EditableSettings { gateway: "kilo".into(), ..s.clone() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        let names: Vec<&str> = c.llm.providers.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec![OPENROUTER_PROVIDER, "polza", OPENROUTER_FREE_PROVIDER]);
        assert_eq!(read_editable_from(&p).unwrap().gateway, OPENROUTER_PROVIDER);

        // no Polza key and never chosen: no Polza block
        let p2 = tmp.path().join("fresh.toml");
        write_editable_to(&p2, &EditableSettings { openrouter_key: "sk-or-live".into(), ..base.clone() }).unwrap();
        assert!(!parse(&fs::read_to_string(&p2).unwrap()).unwrap().llm.providers.iter().any(|p| p.name == "polza"));
        assert!(write_editable_to(&p2, &EditableSettings { polza_key: "sk-ключ".into(), ..base }).is_err());

        assert_eq!(gateway_of_key(" eyJhbGci"), "kilo");
        assert_eq!(gateway_of_key("sk-or-v1-abc"), OPENROUTER_PROVIDER);
        assert_eq!(gateway_of_key("sk-polza-1"), "polza");
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
        assert_eq!(expand_env("$JARVIS_TEST_VAR/Desktop"), "C:\\Users\\me/Desktop");
        assert_eq!(expand_env("${JARVIS_TEST_VAR}/Desktop"), "C:\\Users\\me/Desktop");
        assert_eq!(expand_env("$NO_SUCH_VAR_123 ${unfinished $5 $"), "$NO_SUCH_VAR_123 ${unfinished $5 $");
    }
    #[test]
    fn editable_settings_round_trip_keeps_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sub").join("assistant.toml");
        let s = read_editable_from(&p).unwrap();
        assert_eq!(
            s,
            EditableSettings { tts_backend: "http".into(), address: "сэр".into(), gateway: OPENROUTER_PROVIDER.into(), ..Default::default() }
        );

        let new = EditableSettings {
            openrouter_key: "sk-or-key".into(),
            tts_backend: "http".into(),
            address: "Мисс".into(),
            ..Default::default()
        };
        write_editable_to(&p, &new).unwrap();
        let back = read_editable_from(&p).unwrap();
        assert_eq!(back.openrouter_key, "sk-or-key");
        assert_eq!(back.tts_backend, "http");
        assert_eq!(back.address, "мисс");
        assert_eq!(parse(&fs::read_to_string(&p).unwrap()).unwrap().assistant.address, "мисс");

        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("# Нейросеть — шлюз OpenRouter"), "comments must survive");
        assert!(text.contains("\"браузер\" = \"https://ya.ru\""));
        assert!(write_editable_to(&p, &EditableSettings { address: "a\"b".into(), ..new.clone() }).is_err());
        // an empty address (old window) keeps the file's value
        write_editable_to(&p, &EditableSettings { address: String::new(), ..new.clone() }).unwrap();
        assert_eq!(read_editable_from(&p).unwrap().address, "мисс");
    }

    #[test]
    fn retired_recognition_choice_is_ignored_and_removed_on_save() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        std::fs::write(&p, "[stt]\nengine = 'vosk'\nwhisper_timeout_secs = 7\n").unwrap();
        let settings = read_editable_from(&p).unwrap();
        write_editable_to(&p, &settings).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("engine ="));
        assert_eq!(parse(&text).unwrap().stt.whisper_timeout_secs, 7);
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
    fn missing_openrouter_block_is_added() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("assistant.toml");
        fs::write(&p, "[llm]\nenabled = true\n").unwrap();
        write_editable_to(&p, &EditableSettings { openrouter_key: "sk-or-v1-test".into(), tts_backend: "none".into(), ..Default::default() }).unwrap();
        let c = parse(&fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(c.llm.providers[0].name, OPENROUTER_PROVIDER);
        assert_eq!(c.llm.providers[0].keys, vec!["sk-or-v1-test"]);
        assert_eq!(c.llm.providers[0].base_url, OPENROUTER_BASE_URL);
        assert_eq!(c.tts.backend, "none");
    }
}
