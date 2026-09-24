// models = ["auto"]: pick models from the provider's own list instead of hard-coding names.
// Gemini model names change every few months; the list comes from
// GET {v1beta}/models and is ranked for a voice assistant: the cheapest and fastest first
// (Flash-Lite from 3.5 up, oldest version first), the next ones take over on rate limits.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::Value;

use crate::assistant_config::LlmProvider;

pub const AUTO: &str = "auto";
const CACHE_TTL: Duration = Duration::from_secs(12 * 3600);
// tried per request, enough to survive a couple of retired names
const MAX_AUTO_MODELS: usize = 6;
// models from this version up are preferred; older ones only as a last resort
const MIN_PREFERRED_VERSION: f64 = 3.5;

// used when the list cannot be fetched
pub const GEMINI_FALLBACK: &[&str] = &["gemini-3.5-flash-lite", "gemini-flash-lite-latest", "gemini-flash-latest"];

// not chat models, or not usable through chat/completions with tools
const EXCLUDED: &[&str] = &[
    "embedding", "tts", "image", "live", "audio", "robotics", "computer-use", "aqa", "learnlm",
    "thinking", "vision", "search", "exp-", "-exp", "transcribe", "translate", "omni", "research",
];

static CACHE: Lazy<Mutex<HashMap<String, (Instant, Vec<String>)>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn is_auto(provider: &LlmProvider) -> bool {
    provider.models.iter().any(|m| m.eq_ignore_ascii_case(AUTO))
}

pub fn is_gemini(provider: &LlmProvider) -> bool {
    provider.name.eq_ignore_ascii_case("gemini") || provider.base_url.contains("generativelanguage.googleapis.com")
}

// "gemini-2.5-flash-lite-preview-06-17" -> 2.5
fn version(name: &str) -> f64 {
    name.strip_prefix("gemini-")
        .and_then(|rest| rest.split('-').next())
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn tier(name: &str) -> u8 {
    if name.contains("flash-lite") {
        0
    } else if name.contains("flash") {
        1
    } else if name.contains("pro") {
        // Pro models left the free tier; keep them as a last resort
        3
    } else {
        2
    }
}

fn is_unstable(name: &str) -> bool {
    name.contains("preview") || name.contains("experimental")
}

// (name, supported generation methods) -> usable names, best first
pub fn rank_gemini(models: &[(String, Vec<String>)]) -> Vec<String> {
    let mut usable: Vec<String> = models
        .iter()
        .filter(|(name, methods)| {
            name.starts_with("gemini-")
                && (name.contains("flash") || name.contains("pro"))
                && methods.iter().any(|m| m == "generateContent")
                && !EXCLUDED.iter().any(|x| name.contains(x))
        })
        .map(|(name, _)| name.clone())
        .collect();

    let old = |n: &str| version(n) < MIN_PREFERRED_VERSION;
    usable.sort_by(|a, b| {
        let by_version = if old(a) && old(b) {
            // below the floor: newest first
            version(b).partial_cmp(&version(a))
        } else {
            // cheapest first: 3.5 before 3.6 before 3.7 ...
            version(a).partial_cmp(&version(b))
        };
        // the chosen versions (3.5+) even as previews go before the old ones
        old(a)
            .cmp(&old(b))
            .then(is_unstable(a).cmp(&is_unstable(b)))
            .then(tier(a).cmp(&tier(b)))
            .then(by_version.unwrap_or(std::cmp::Ordering::Equal))
            // aliases like "gemini-2.5-flash" before pinned "gemini-2.5-flash-001"
            .then(a.len().cmp(&b.len()))
            .then(a.cmp(b))
    });
    usable.dedup();
    usable
}

pub fn parse_gemini_list(body: &Value) -> Vec<(String, Vec<String>)> {
    body.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let name = m.get("name")?.as_str()?;
                    let name = name.strip_prefix("models/").unwrap_or(name).to_string();
                    let methods = m
                        .get("supportedGenerationMethods")
                        .and_then(|x| x.as_array())
                        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                        .unwrap_or_default();
                    Some((name, methods))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ".../v1beta/openai" -> ".../v1beta/models"
fn gemini_list_url(base_url: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let base = base.strip_suffix("/openai").unwrap_or(base);
    format!("{}/models?pageSize=1000", base)
}

fn fetch_gemini(provider: &LlmProvider, key: &str, timeout: Duration) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder().timeout(timeout).build().map_err(|e| e.to_string())?;
    let resp = client
        .get(gemini_list_url(&provider.base_url))
        // key in a header, never in the URL (URLs end up in logs)
        .header("x-goog-api-key", key)
        .send()
        .map_err(|e| format!("network: {}", e))?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        return Err(format!("{}: {}", status, text.chars().take(200).collect::<String>()));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("bad JSON: {}", e))?;
    let ranked = rank_gemini(&parse_gemini_list(&v));
    if ranked.is_empty() {
        return Err("no usable models in the list".into());
    }
    Ok(ranked)
}

// models to try for this provider; "auto" is expanded, explicit names are kept in order
pub fn resolve(provider: &LlmProvider, key: &str, timeout: Duration) -> Vec<String> {
    let explicit: Vec<String> = provider
        .models
        .iter()
        .filter(|m| !m.eq_ignore_ascii_case(AUTO) && !m.trim().is_empty())
        .cloned()
        .collect();
    if !is_auto(provider) {
        return explicit;
    }
    if !is_gemini(provider) {
        warn!("models = [\"auto\"] is supported for Gemini only (provider {})", provider.name);
        return explicit;
    }

    let cached = CACHE
        .lock()
        .get(&provider.name)
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, list)| list.clone());

    let auto = match cached {
        Some(list) => list,
        None => match fetch_gemini(provider, key, timeout) {
            Ok(list) => {
                info!("Gemini models (auto): {}", list.join(", "));
                CACHE.lock().insert(provider.name.clone(), (Instant::now(), list.clone()));
                list
            }
            Err(e) => {
                warn!("Cannot list Gemini models ({}), using defaults", e);
                GEMINI_FALLBACK.iter().map(|s| s.to_string()).collect()
            }
        },
    };

    // explicit names first (user's choice), then the automatic ones
    let mut out = explicit;
    for m in auto.into_iter().take(MAX_AUTO_MODELS) {
        if !out.contains(&m) {
            out.push(m);
        }
    }
    out
}

// a model the API rejected (retired name): drop it from the cached list
pub fn forget(provider_name: &str, model: &str) {
    if let Some((_, list)) = CACHE.lock().get_mut(provider_name) {
        list.retain(|m| m != model);
    }
}

#[cfg(test)]
pub fn clear_cache() {
    CACHE.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> (String, Vec<String>) {
        (name.to_string(), vec!["generateContent".to_string(), "countTokens".to_string()])
    }

    #[test]
    fn ranking_prefers_cheapest_flash_lite_from_3_5_up() {
        let list = vec![
            m("gemini-3.7-flash-lite"),
            m("gemini-3.5-flash"),
            m("gemini-3.5-flash-lite"),
            m("gemini-3.8-flash-lite"),
            m("gemini-3.6-flash-lite"),
            m("gemini-3.6-flash-preview"),
            m("gemini-3.5-transcribe"),
            m("gemini-3.1-flash-lite"),
            m("gemini-2.0-flash"),
            m("gemini-2.5-pro"),
            m("gemini-2.5-flash-lite"),
            m("gemini-3-flash-preview"),
            m("gemini-2.5-flash"),
            m("gemini-2.5-flash-001"),
            m("gemini-2.5-flash-preview-tts"),
            m("gemini-embedding-001"),
            m("gemini-2.0-flash-exp-image-generation"),
            m("gemma-3-27b-it"),
            ("gemini-2.5-flash-live".to_string(), vec!["bidiGenerateContent".to_string()]),
        ];
        assert_eq!(
            rank_gemini(&list),
            vec![
                "gemini-3.5-flash-lite",
                "gemini-3.6-flash-lite",
                "gemini-3.7-flash-lite",
                "gemini-3.8-flash-lite",
                "gemini-3.5-flash",
                "gemini-3.6-flash-preview",
                "gemini-3.1-flash-lite",
                "gemini-2.5-flash-lite",
                "gemini-2.5-flash",
                "gemini-2.5-flash-001",
                "gemini-2.0-flash",
                "gemini-2.5-pro",
                "gemini-3-flash-preview",
            ]
        );
    }

    #[test]
    fn list_response_is_parsed() {
        let body = serde_json::json!({"models": [
            {"name": "models/gemini-2.5-flash", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/text-embedding-004", "supportedGenerationMethods": ["embedContent"]}
        ]});
        let parsed = parse_gemini_list(&body);
        assert_eq!(parsed[0].0, "gemini-2.5-flash");
        assert_eq!(rank_gemini(&parsed), vec!["gemini-2.5-flash"]);
    }

    #[test]
    fn list_url_is_derived_from_openai_base() {
        assert_eq!(
            gemini_list_url("https://generativelanguage.googleapis.com/v1beta/openai/"),
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000"
        );
    }

    #[test]
    fn explicit_models_are_kept_for_non_auto() {
        let p = LlmProvider { name: "x".into(), models: vec!["a".into(), "b".into()], ..LlmProvider::default() };
        assert_eq!(resolve(&p, "k", Duration::from_secs(1)), vec!["a", "b"]);
    }
}
