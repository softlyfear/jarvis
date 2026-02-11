mod gliner;

use std::collections::HashMap;
use once_cell::sync::OnceCell;

use crate::commands::{SlotDefinition, SlotValue};
use crate::config::structs::SlotExtractionEngine;
use crate::DB;

static SLOT_ENGINE: OnceCell<SlotExtractionEngine> = OnceCell::new();

pub fn init() -> Result<(), String> {
    if SLOT_ENGINE.get().is_some() {
        return Ok(());
    }

    let engine = DB.get()
        .map(|db| db.read().slot_extraction_engine)
        .unwrap_or(SlotExtractionEngine::None);

    SLOT_ENGINE.set(engine).map_err(|_| "Slot engine already set")?;

    match engine {
        SlotExtractionEngine::None => {
            info!("Slot extraction disabled");
        }
        SlotExtractionEngine::GLiNER => {
            info!("Initializing GLiNER slot extraction backend.");
            gliner::init()?;
            info!("GLiNER slot extraction backend initialized.");
        }
    }

    Ok(())
}

// Extract slot values from text using the configured engine
pub fn extract(
    text: &str,
    slots: &HashMap<String, SlotDefinition>,
) -> HashMap<String, SlotValue> {
    if slots.is_empty() {
        return HashMap::new();
    }

    match SLOT_ENGINE.get().unwrap_or(&SlotExtractionEngine::None) {
        SlotExtractionEngine::None => HashMap::new(),
        SlotExtractionEngine::GLiNER => {
            match gliner::extract(text, slots) {
                Ok(result) => result,
                Err(e) => {
                    error!("GLiNER slot extraction failed: {}", e);
                    HashMap::new()
                }
            }
        }
    }
}