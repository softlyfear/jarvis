mod intentclassifier;
mod embeddingclassifier;

use std::path::PathBuf;

use crate::{JCommandsList, commands::JCommand, config};
use once_cell::sync::OnceCell;
use crate::config::structs::IntentRecognitionEngine;

use crate::DB;

static IRE_TYPE: OnceCell<IntentRecognitionEngine> = OnceCell::new();

pub async fn init(commands: &Vec<JCommandsList>) -> Result<(), String> {
    if IRE_TYPE.get().is_some() {
        return Ok(());
    } // already initialized

    // set default ire type
    // IRE_TYPE.set(config::DEFAULT_INTENT_RECOGNITION_ENGINE).unwrap();

    // store current ire type
    IRE_TYPE
        .set(DB.get().unwrap().read().intent_recognition_engine)
        .unwrap();

    // load given recorder
    match IRE_TYPE.get().unwrap() {
        IntentRecognitionEngine::IntentClassifier => {
            info!("Initializing IntentClassifier IRE backend.");
            intentclassifier::init(&commands).await?;
            info!("IntentClassifier IRE backend initialized.");
        },
        IntentRecognitionEngine::EmbeddingClassifier => {
            info!("Initializing EmbeddingClassifier IRE backend.");
            embeddingclassifier::init(&commands)?;
            info!("EmbeddingClassifier IRE backend initialized.");
        },
    }

    Ok(())
}

pub async fn classify(text: &str) -> Option<(String, f64)> {
    match IRE_TYPE.get()? {
        IntentRecognitionEngine::IntentClassifier => {
            match intentclassifier::classify(text).await {
                Ok(prediction) => {
                    let confidence = prediction.confidence.value();
                    if confidence >= config::INTENT_CLASSIFIER_MIN_CONFIDENCE {
                        Some((prediction.intent.to_string(), confidence))
                    } else {
                        None
                    }
                }
                Err(e) => {
                    error!("Intent classification error: {}", e);
                    None
                }
            }
        }
        IntentRecognitionEngine::EmbeddingClassifier => {
            match embeddingclassifier::classify(text) {
                Ok((intent_id, confidence)) => {
                    if confidence >= config::EMBEDDING_MIN_CONFIDENCE {
                        Some((intent_id, confidence))
                    } else {
                        None
                    }
                }
                Err(e) => {
                    error!("Embedding classification error: {}", e);
                    None
                }
            }
        }
    }
}

pub fn get_command_by_intent(commands: &'static Vec<JCommandsList>, intent_id: &str) -> Option<(&'static PathBuf, &'static JCommand)> {
    match IRE_TYPE.get()? {
        IntentRecognitionEngine::IntentClassifier => {
            intentclassifier::get_command(commands, intent_id)
        }
        IntentRecognitionEngine::EmbeddingClassifier => {
            embeddingclassifier::get_command(commands, intent_id)
        }
    }
}