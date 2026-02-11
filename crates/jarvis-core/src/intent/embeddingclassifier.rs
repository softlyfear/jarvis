use parking_lot::Mutex;
use std::path::PathBuf;
use std::fs;

// use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use fastembed::{TextEmbedding, UserDefinedEmbeddingModel, TokenizerFiles, InitOptionsUserDefined, Pooling, QuantizationMode, OutputKey};
use once_cell::sync::OnceCell;

use crate::commands::JCommandsList;
use crate::i18n::get_language;
use crate::{APP_CONFIG_DIR, APP_DIR, i18n};

static CLASSIFIER: OnceCell<Mutex<EmbeddingClassifier>> = OnceCell::new();

struct IntentVector {
    id: String,
    vector: Vec<f32>,
}

struct EmbeddingClassifier {
    model: TextEmbedding,
    intents: Vec<IntentVector>,
}

const CACHE_FILE: &str = "embedding_intents.json";
const HASH_FILE: &str = "embedding_hash.txt";

pub fn init(commands: &[JCommandsList]) -> Result<(), String> {
    if CLASSIFIER.get().is_some() {
        return Ok(());
    }

    info!("Initializing embedding model...");

    // let mut model = TextEmbedding::try_new(
    //     InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(true),
    // ).map_err(|e| format!("Failed to load embedding model: {}", e))?;

    let model_dir;
    match i18n::get_language().as_str() {
        "en" => {
            // smaller model for English
            info!("Loading all-MiniLM-L6-v2 ...");
            model_dir = APP_DIR.join("resources").join("models").join("all-MiniLM-L6-v2");
        },
        _ => {
            // bigger model for any other languages (multilingual)
            info!("Loading paraphrase-multilingual-MiniLM-L12-v2-onnx-Q ...");
            model_dir = APP_DIR.join("resources").join("models").join("paraphrase-multilingual-MiniLM-L12-v2-onnx-Q");
        }
    }

    // info!("{}", model_dir.display());

    let user_model = UserDefinedEmbeddingModel {
        onnx_file: std::fs::read(model_dir.join("model.onnx"))
            .map_err(|e| format!("Failed to read model.onnx: {}", e))?,
        tokenizer_files: TokenizerFiles {
            tokenizer_file: std::fs::read(model_dir.join("tokenizer.json"))
                .map_err(|e| format!("Failed to read tokenizer.json: {}", e))?,
            config_file: std::fs::read(model_dir.join("config.json"))
                .map_err(|e| format!("Failed to read config.json: {}", e))?,
            special_tokens_map_file: std::fs::read(model_dir.join("special_tokens_map.json"))
                .map_err(|e| format!("Failed to read special_tokens_map.json: {}", e))?,
            tokenizer_config_file: std::fs::read(model_dir.join("tokenizer_config.json"))
                .map_err(|e| format!("Failed to read tokenizer_config.json: {}", e))?,
        },
        pooling: Some(Pooling::Mean),
        quantization: QuantizationMode::None,
        output_key: Some(OutputKey::ByName("last_hidden_state")),
    };

    let mut model = TextEmbedding::try_new_from_user_defined(user_model, Default::default())
        .map_err(|e| format!("Failed to load embedding model: {}", e))?;

    info!("Embedding model loaded");

    let current_hash = crate::commands::commands_hash(commands);
    let config_dir = APP_CONFIG_DIR.get().ok_or("Config dir not set")?;
    let hash_path = config_dir.join(HASH_FILE);
    let cache_path = config_dir.join(CACHE_FILE);

    // check if cached vectors are still valid
    let should_retrain = if hash_path.exists() && cache_path.exists() {
        let stored_hash = fs::read_to_string(&hash_path).unwrap_or_default();
        stored_hash.trim() != current_hash
    } else {
        true
    };

    let intents = if should_retrain {
        info!("Building intent vectors from commands...");
        let intents = build_intent_vectors(&mut model, commands)?;
        
        // cache to disk
        if let Ok(json) = serde_json::to_string(&intents_to_cache(&intents)) {
            let _ = fs::write(&cache_path, json);
            let _ = fs::write(&hash_path, &current_hash);
            info!("Intent vectors cached");
        }
        
        intents
    } else {
        info!("Loading cached intent vectors...");
        load_cached_intents(&cache_path)?
    };

    info!("Embedding classifier ready with {} intents", intents.len());

    CLASSIFIER.set(Mutex::new(EmbeddingClassifier { model, intents }))
        .map_err(|_| "Classifier already set")?;

    Ok(())
}

fn build_intent_vectors(
    model: &mut TextEmbedding,
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
            
            let embeddings = model.embed(texts, None)
                .map_err(|e| format!("Embedding failed for '{}': {}", cmd.id, e))?;

            // average all phrase vectors into one intent vector
            let dim = embeddings[0].len();
            let mut avg = vec![0.0f32; dim];
            
            for emb in &embeddings {
                for (i, val) in emb.iter().enumerate() {
                    avg[i] += val;
                }
            }
            
            let count = embeddings.len() as f32;
            for val in &mut avg {
                *val /= count;
            }

            // normalize
            let norm: f32 = avg.iter().map(|v| v * v).sum::<f32>().sqrt();
            if norm > 0.0 {
                for val in &mut avg {
                    *val /= norm;
                }
            }

            intents.push(IntentVector {
                id: cmd.id.clone(),
                vector: avg,
            });
        }
    }

    Ok(intents)
}

pub fn classify(text: &str) -> Result<(String, f64), String> {
    let mut classifier = CLASSIFIER.get().ok_or("Classifier not initialized")?.lock();
    
    let embeddings = classifier.model.embed(vec![text], None)
        .map_err(|e| format!("Failed to embed query: {}", e))?;
    
    let mut query_vec = embeddings.into_iter().next()
        .ok_or("Empty embedding result")?;

    // normalize query
    let norm: f32 = query_vec.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for val in &mut query_vec {
            *val /= norm;
        }
    }

    // cosine similarity against all intents (dot product of normalized vectors)
    let mut best_id = String::new();
    let mut best_score: f64 = -1.0;

    for intent in &classifier.intents {
        let score: f64 = query_vec.iter()
            .zip(intent.vector.iter())
            .map(|(a, b)| (*a as f64) * (*b as f64))
            .sum();

        if score > best_score {
            best_score = score;
            best_id = intent.id.clone();
        }
    }

    debug!("Embedding classify: '{}' -> '{}' ({:.2}%)", text, best_id, best_score * 100.0);

    Ok((best_id, best_score))
}

pub fn get_command<'a>(
    commands: &'a [JCommandsList],
    intent_id: &str,
) -> Option<(&'a PathBuf, &'a crate::commands::JCommand)> {
    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            if cmd.id == intent_id {
                return Some((&cmd_list.path, cmd));
            }
        }
    }
    None
}

// ### CACHE HELPERS

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
    
    Ok(cached.into_iter().map(|c| IntentVector {
        id: c.id,
        vector: c.vector,
    }).collect())
}