// BASED ON: gline-rs crate source code
// https://github.com/fbilhaut/gline-rs

use std::collections::HashMap;
use std::path::PathBuf;
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use ndarray::Array;
use regex::Regex;
use tokenizers::Tokenizer;
use ort::value::Tensor;

pub mod structs;
use structs::GlinerModelInfo;

use std::fs;

use crate::commands::{SlotDefinition, SlotValue};
use crate::{APP_DIR, i18n};

// MODEL STATE

struct GlinerModel {
    session: ort::session::Session,
    tokenizer: Tokenizer,
    splitter: Regex,
}

unsafe impl Send for GlinerModel {}
unsafe impl Sync for GlinerModel {}

static MODEL: OnceCell<Mutex<GlinerModel>> = OnceCell::new();

// GLiNER defaults (same as gline-rs Parameters::default())
const THRESHOLD: f32 = 0.3;
const MAX_WIDTH: usize = 12;
const MAX_LENGTH: usize = 512;

// applied after decoding
const MIN_CONFIDENCE: f32 = 0.4;

// word splitting regex (gline-rs RegexSplitter default)
const WORD_REGEX: &str = r"\w+(?:[-_]\w+)*|\S";

// INIT

pub fn init() -> Result<(), String> {
    if MODEL.get().is_some() {
        return Ok(());
    }

    let variant = crate::DB.get()
        .map(|db| db.read().gliner_model.clone())
        .unwrap_or_default();

    let language = i18n::get_language();

    let (model_dir, onnx_file) = if variant.is_empty() {
        (select_model_dir(), "model.onnx".to_string())
    } else {
        crate::gliner_models::resolve_model(&variant, &language)
            .unwrap_or_else(|| (select_model_dir(), "model.onnx".to_string()))
    };

    let model_path = model_dir.join("onnx").join(&onnx_file);
    let tokenizer_path = model_dir.join("tokenizer.json");

    info!("Loading GLiNER model from: {}, variant {}", model_dir.display(), variant);

    let session = ort::session::Session::builder()
        .map_err(|e| format!("Failed to create ort session builder: {}", e))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("Failed to load ONNX model: {}", e))?;

    let tokenizer = Tokenizer::from_file(&tokenizer_path)
        .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

    let splitter = Regex::new(WORD_REGEX)
        .map_err(|e| format!("Failed to compile word regex: {}", e))?;

    MODEL.set(Mutex::new(GlinerModel { session, tokenizer, splitter }))
        .map_err(|_| "GLiNER model already initialized".to_string())?;

    info!("GLiNER model loaded");
    Ok(())
}

fn select_model_dir() -> PathBuf {
    let base = APP_DIR.join("resources").join("models");

    match i18n::get_language().as_str() {
        "en" => {
            let path = base.join("gliner_small-v2.1");
            if path.exists() { return path; }
        }
        _ => {}
    }

    // multilingual (covers RU, UA, EN)
    let multi = base.join("gliner_multi-v2.1");
    if multi.exists() { return multi; }

    // fallback
    base.join("gliner_small-v2.1")
}

// WORD SPLITTING

struct WordToken {
    start: usize,
    end: usize,
    text: String,
}

fn split_words(splitter: &Regex, text: &str, limit: Option<usize>) -> Vec<WordToken> {
    let mut tokens = Vec::new();
    for m in splitter.find_iter(text) {
        tokens.push(WordToken {
            start: m.start(),
            end: m.end(),
            text: m.as_str().to_string(),
        });
        if let Some(lim) = limit {
            if tokens.len() >= lim { break; }
        }
    }
    tokens
}

// PROMPT CONSTRUCTION
//
// GLiNER prompt format:
//   [<<ENT>>, label1_w1, label1_w2, <<ENT>>, label2_w1, ..., <<SEP>>, word1, word2, ..., wordN]

fn build_prompt(entities: &[&str], words: &[WordToken]) -> (Vec<String>, usize) {
    let mut prompt = Vec::with_capacity(entities.len() * 2 + 1 + words.len());

    for entity in entities {
        prompt.push("<<ENT>>".to_string());
        prompt.push(entity.to_string()); // whole string, no split
    }
    prompt.push("<<SEP>>".to_string());

    let entities_len = prompt.len();

    for w in words {
        prompt.push(w.text.clone());
    }

    (prompt, entities_len)
}

// ENCODING

struct EncodedBatch {
    input_ids: ndarray::Array2<i64>,
    attention_masks: ndarray::Array2<i64>,
    word_masks: ndarray::Array2<i64>,
    text_lengths: ndarray::Array2<i64>,
    num_words: usize,
}

fn encode_single(
    tokenizer: &Tokenizer,
    _text: &str,
    entities: &[&str],
    words: &[WordToken],
) -> Result<EncodedBatch, String> {
    let (prompt, ent_len) = build_prompt(entities, words);
    let text_word_count = words.len();

    let mut word_encodings: Vec<Vec<u32>> = Vec::with_capacity(prompt.len());
    let mut total_tokens: usize = 2; // BOS + EOS
    let mut entity_tokens: usize = 0;

    for (pos, word) in prompt.iter().enumerate() {
        let encoding = tokenizer.encode(word.as_str(), false)
            .map_err(|e| format!("Tokenizer encode error: {}", e))?;
        let ids = encoding.get_ids().to_vec();
        total_tokens += ids.len();
        if pos < ent_len {
            entity_tokens += ids.len();
        }
        word_encodings.push(ids);
    }

    // text_offset: index where text tokens start (after BOS + entity tokens)
    let text_offset = entity_tokens + 1;

    // DEBUG
    debug!("GLiNER prompt ({} total, ent_len={}, text_offset={}):", prompt.len(), ent_len, text_offset);
    for (i, (word, enc)) in prompt.iter().zip(word_encodings.iter()).enumerate() {
        debug!("  [{}]{} '{}' -> {:?}", i, if i < ent_len { " ENT" } else { " TXT" }, word, enc);
    }

    let mut input_ids = Array::zeros((1, total_tokens));
    let mut attention_masks = Array::zeros((1, total_tokens));
    let mut word_masks = Array::zeros((1, total_tokens));

    let mut idx: usize = 0;
    let mut word_id: i64 = 0;

    // BOS
    input_ids[[0, idx]] = 1;
    attention_masks[[0, idx]] = 1;
    idx += 1;

    // encode each word - matching gline-rs idx-based logic exactly
    for word_enc in word_encodings.iter() {
        for (token_idx, &token_id) in word_enc.iter().enumerate() {
            input_ids[[0, idx]] = token_id as i64;
            attention_masks[[0, idx]] = 1;
            // word mask: only for text tokens (past text_offset), first sub-token only
            if idx >= text_offset && token_idx == 0 {
                word_masks[[0, idx]] = word_id;
            }
            idx += 1;
        }
        // increment word_id for any word whose tokens end past text_offset
        if idx >= text_offset {
            word_id += 1;
        }
    }

    // EOS
    input_ids[[0, idx]] = 2;
    attention_masks[[0, idx]] = 1;

    let mut text_lengths = Array::zeros((1, 1));
    text_lengths[[0, 0]] = (text_word_count + 1) as i64;

    debug!("GLiNER input_ids: {:?}", input_ids.as_slice().unwrap());
    debug!("GLiNER word_masks: {:?}", word_masks.as_slice().unwrap());
    debug!("GLiNER text_lengths: {}", text_word_count);

    Ok(EncodedBatch {
        input_ids,
        attention_masks,
        word_masks,
        text_lengths,
        num_words: text_word_count + 1,
    })
}

// SPAN TENSORS

fn make_span_tensors(num_words: usize, max_width: usize) -> (ndarray::Array3<i64>, ndarray::Array2<bool>) {
    let num_spans = num_words * max_width;

    let mut span_idx = Array::zeros((1, num_spans, 2));
    let mut span_mask = Array::from_elem((1, num_spans), false);

    for start in 0..num_words {
        let remaining = num_words - start;
        let actual_max = max_width.min(remaining);
        for width in 0..actual_max {
            let dim = start * max_width + width;
            span_idx[[0, dim, 0]] = start as i64;
            span_idx[[0, dim, 1]] = (start + width) as i64;
            span_mask[[0, dim]] = true;
        }
    }

    (span_idx, span_mask)
}

// DECODE + GREEDY SEARCH

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

struct Entity {
    text: String,
    label: String,
    prob: f32,
    start: usize,
    end: usize,
}

fn decode_and_search(
    logits_data: &[f32],
    logits_shape: &[usize],
    words: &[WordToken],
    text: &str,
    entities: &[&str],
    max_width: usize,
    threshold: f32,
) -> Vec<Entity> {
    let num_tokens = words.len();

    let dim_mw = logits_shape.get(2).copied().unwrap_or(0);
    let dim_e = logits_shape.get(3).copied().unwrap_or(0);

    let mut spans: Vec<Entity> = Vec::new();

    for start in 1..=num_tokens {
        let max_end = (start + max_width).min(num_tokens + 1);
        for end in start..max_end {
            let width = end - start;
            for (class_idx, &entity_label) in entities.iter().enumerate() {
                let flat_idx = start * dim_mw * dim_e + width * dim_e + class_idx;
                if flat_idx >= logits_data.len() { continue; }

                let raw_score = logits_data[flat_idx];
                let prob = sigmoid(raw_score);
                if prob >= threshold {
                    let w_start = start - 1;
                    let w_end = end - 1;
                    let start_offset = words[w_start].start;
                    let end_offset = words[w_end].end;
                    let span_text = text[start_offset..end_offset].to_string();
                    spans.push(Entity {
                        text: span_text,
                        label: entity_label.to_string(),
                        prob,
                        start: start_offset,
                        end: end_offset,
                    });
                }
            }
        }
    }

    spans.sort_unstable_by(|a, b| (a.start, a.end).cmp(&(b.start, b.end)));
    greedy_flat(&spans)
}

fn greedy_flat(spans: &[Entity]) -> Vec<Entity> {
    if spans.is_empty() {
        return Vec::new();
    }

    let mut result: Vec<Entity> = Vec::new();
    let mut prev = 0usize;
    let mut next = 1usize;

    while next < spans.len() {
        let p = &spans[prev];
        let n = &spans[next];

        if n.start >= p.end || p.start >= n.end {
            result.push(Entity {
                text: p.text.clone(),
                label: p.label.clone(),
                prob: p.prob,
                start: p.start,
                end: p.end,
            });
            prev = next;
        } else if p.prob < n.prob {
            prev = next;
        }
        next += 1;
    }

    let last = &spans[prev];
    result.push(Entity {
        text: last.text.clone(),
        label: last.label.clone(),
        prob: last.prob,
        start: last.start,
        end: last.end,
    });

    result
}

// PUBLIC API

pub fn extract(
    text: &str,
    slots: &HashMap<String, SlotDefinition>,
) -> Result<HashMap<String, SlotValue>, String> {
    let mut model = MODEL.get().ok_or("GLiNER not initialized")?.lock();

    let mut label_to_slots: HashMap<&str, Vec<&str>> = HashMap::new();
    for (slot_name, def) in slots {
        if !def.entity.is_empty() {
            label_to_slots
                .entry(def.entity.as_str())
                .or_default()
                .push(slot_name.as_str());
        }
    }

    if label_to_slots.is_empty() {
        return Ok(HashMap::new());
    }

    let labels: Vec<&str> = label_to_slots.keys().copied().collect();

    debug!("GLiNER extract: text='{}', labels={:?}", text, labels);

    let words = split_words(&model.splitter, text, Some(MAX_LENGTH));
    if words.is_empty() {
        return Ok(HashMap::new());
    }

    let encoded = encode_single(&model.tokenizer, text, &labels, &words)?;

    let (span_idx, span_mask) = make_span_tensors(encoded.num_words, MAX_WIDTH);

    let t_input_ids = Tensor::from_array(encoded.input_ids).map_err(|e| format!("tensor: {}", e))?;
    let t_attn = Tensor::from_array(encoded.attention_masks).map_err(|e| format!("tensor: {}", e))?;
    let t_words = Tensor::from_array(encoded.word_masks).map_err(|e| format!("tensor: {}", e))?;
    let t_lengths = Tensor::from_array(encoded.text_lengths).map_err(|e| format!("tensor: {}", e))?;
    let t_span_idx = Tensor::from_array(span_idx).map_err(|e| format!("tensor: {}", e))?;
    let t_span_mask = Tensor::from_array(span_mask).map_err(|e| format!("tensor: {}", e))?;

    let outputs = model.session.run(
        ort::inputs! {
            "input_ids" => t_input_ids,
            "attention_mask" => t_attn,
            "words_mask" => t_words,
            "text_lengths" => t_lengths,
            "span_idx" => t_span_idx,
            "span_mask" => t_span_mask,
        }
    ).map_err(|e| format!("ort inference error: {}", e))?;

    let (shape, logits_data) = outputs["logits"]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("Failed to extract logits: {}", e))?;

    let logits_shape: Vec<usize> = shape.iter().map(|&d| d as usize).collect();

    debug!("GLiNER logits shape: {:?}, data len: {}", logits_shape, logits_data.len());
    let max_logit = logits_data.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    debug!("GLiNER max logit: {:.4}, sigmoid: {:.4}", max_logit, sigmoid(max_logit));

    // dump all scores above 5%
    let num_words = logits_shape.get(1).copied().unwrap_or(0);
    let dim_mw = logits_shape.get(2).copied().unwrap_or(0);
    let dim_e = logits_shape.get(3).copied().unwrap_or(0);
    for start in 0..num_words {
        for width in 0..dim_mw.min(num_words - start) {
            for class_idx in 0..dim_e {
                let flat_idx = start * dim_mw * dim_e + width * dim_e + class_idx;
                if flat_idx < logits_data.len() {
                    let score = logits_data[flat_idx];
                    let prob = sigmoid(score);
                    if prob > 0.05 {
                        let end = start + width;
                        let w_start = if start < words.len() { &words[start].text } else { "?" };
                        let w_end = if end < words.len() { &words[end].text } else { "?" };
                        debug!("  span[{}..{}] '{}'->'{}' label={} score={:.3} prob={:.3}",
                            start, end, w_start, w_end, labels.get(class_idx).unwrap_or(&"?"), score, prob);
                    }
                }
            }
        }
    }

    let entities = decode_and_search(
        logits_data, &logits_shape, &words, text, &labels, MAX_WIDTH, THRESHOLD,
    );

    let mut result = HashMap::new();

    for entity in &entities {
        if entity.prob < MIN_CONFIDENCE {
            continue;
        }

        debug!("GLiNER entity: '{}' -> '{}' ({:.1}%)",
            entity.text, entity.label, entity.prob * 100.0);

        if let Some(slot_names) = label_to_slots.get(entity.label.as_str()) {
            for &slot_name in slot_names {
                if !result.contains_key(slot_name) {
                    let value = parse_slot_value(&entity.text);
                    result.insert(slot_name.to_string(), value);
                }
            }
        }
    }

    Ok(result)
}

fn parse_slot_value(text: &str) -> SlotValue {
    if let Ok(n) = text.parse::<f64>() {
        return SlotValue::Number(n);
    }
    SlotValue::Text(text.to_string())
}