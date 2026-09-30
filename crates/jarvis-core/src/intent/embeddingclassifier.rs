use std::path::PathBuf;
use std::sync::Arc;
use std::fs;

use once_cell::sync::OnceCell;

use crate::commands::JCommandsList;
use crate::i18n;
use crate::APP_CONFIG_DIR;
use crate::models::embedding::EmbeddingModel;
use crate::embedding_vectors;

// no outer Mutex needed - state is immutable after init.
// the embedding model has its own internal Mutex.
static CLASSIFIER: OnceCell<EmbeddingClassifierState> = OnceCell::new();

struct IntentVector {
    id: String,
    vector: Vec<f32>,
}

struct EmbeddingClassifierState {
    model: Arc<EmbeddingModel>,
    intents: Vec<IntentVector>,
}

// model is Arc (Send+Sync), intents are read-only after init
unsafe impl Send for EmbeddingClassifierState {}
unsafe impl Sync for EmbeddingClassifierState {}

const CACHE_FILE: &str = "embedding_intents.json";
const HASH_FILE: &str = "embedding_hash.txt";

// init with a model loaded through the registry
pub fn init_with_model(model: Arc<EmbeddingModel>, commands: &[JCommandsList]) -> Result<(), String> {
    if CLASSIFIER.get().is_some() {
        return Ok(());
    }

    info!("Initializing embedding classifier...");

    let current_hash = format!("{}:{}:{}", model.model_id, i18n::get_language(), crate::commands::commands_hash(commands));
    let config_dir = APP_CONFIG_DIR.get().ok_or("Config dir not set")?;
    let hash_path = config_dir.join(HASH_FILE);
    let cache_path = config_dir.join(CACHE_FILE);

    // A corrupt cache is disposable; model identity and language are part of its key.
    let cached = if fs::read_to_string(&hash_path).is_ok_and(|hash| hash.trim() == current_hash) {
        load_cached_intents(&cache_path).ok()
    } else {
        None
    };
    let intents = match cached {
        Some(intents) => intents,
        None => {
            info!("Building intent vectors from commands...");
            let intents = build_intent_vectors(&model, commands)?;
            if let Ok(json) = serde_json::to_string(&intents_to_cache(&intents)) {
                let result = crate::storage::atomic_write(&cache_path, json.as_bytes())
                    .and_then(|_| crate::storage::atomic_write(&hash_path, current_hash.as_bytes()));
                match result {
                    Ok(()) => info!("Intent vectors cached"),
                    Err(e) => warn!("Could not cache intent vectors: {}", e),
                }
            }
            intents
        }
    };
    if intents.is_empty() {
        return Err("No command phrases available for embedding classification".into());
    }

    info!("Embedding classifier ready with {} intents", intents.len());

    CLASSIFIER.set(EmbeddingClassifierState { model, intents })
        .map_err(|_| "Classifier already set".to_string())?;

    Ok(())
}

fn build_intent_vectors(
    model: &EmbeddingModel,
    commands: &[JCommandsList],
) -> Result<Vec<IntentVector>, String> {
    let lang = i18n::get_language();
    let mut intents = Vec::new();

    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            let phrases = cmd.get_phrases(&lang);
            if phrases.is_empty() {
                continue;
            }

            let texts: Vec<&str> = phrases.iter().map(|s| s.as_str()).collect();
            
            let embeddings = model.embedding.lock().embed(texts, None)
                .map_err(|e| format!("Embedding failed for '{}': {}", cmd.id, e))?;

            let avg = embedding_vectors::average(&embeddings)?;

            intents.push(IntentVector {
                id: cmd.id.clone(),
                vector: avg,
            });
        }
    }

    Ok(intents)
}

pub fn classify(text: &str) -> Result<(String, f64), String> {
    let state = CLASSIFIER.get().ok_or("Classifier not initialized")?;
    
    // only the embedding model needs locking, intents are read-only
    let embeddings = state.model.embedding.lock().embed(vec![text], None)
        .map_err(|e| format!("Failed to embed query: {}", e))?;
    
    let mut query_vec = embeddings.into_iter().next()
        .ok_or("Empty embedding result")?;

    embedding_vectors::normalize(&mut query_vec)?;
    let (best_idx, best_score) = embedding_vectors::best_match(
        &query_vec, state.intents.iter().map(|intent| intent.vector.as_slice()),
    )?;

    let best_id = state.intents[best_idx].id.clone();
    debug!("Embedding classify: '{}' -> '{}' ({:.2}%)", text, best_id, best_score * 100.0);

    Ok((best_id, best_score))
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedIntent {
    id: String,
    vector: Vec<f32>,
}

fn intents_to_cache(intents: &[IntentVector]) -> Vec<CachedIntent> {
    intents.iter().map(|i| CachedIntent {
        id: i.id.clone(),
        vector: i.vector.clone(),
    }).collect()
}

fn load_cached_intents(path: &PathBuf) -> Result<Vec<IntentVector>, String> {
    let json = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read cache: {}", e))?;
    
    let cached: Vec<CachedIntent> = serde_json::from_str(&json)
        .map_err(|e| format!("Failed to parse cache: {}", e))?;
    
    cached.into_iter().map(|mut c| {
        embedding_vectors::normalize(&mut c.vector)?;
        Ok(IntentVector { id: c.id, vector: c.vector })
    }).collect()
}
