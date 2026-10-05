// Synthesize a first sentence while HTTP streams; only validated final replies may play it.
use super::RequestControl;
use std::sync::mpsc::Receiver;

#[derive(Default)]
pub struct SpeechPreview {
    raw: String,
    prepared: Option<(String, Receiver<Vec<u8>>)>,
    attempted: bool,
}
impl SpeechPreview {
    pub fn push(&mut self, text: &str, control: &RequestControl) {
        if self.attempted || crate::assistant_config::get().tts.backend != "http" {
            return;
        }
        if self.raw.len() + text.len() > 8192 {
            self.attempted = true;
            return;
        }
        self.raw.push_str(text);
        let Some(sentence) = first_sentence(&self.raw) else {
            return;
        };
        self.attempted = true;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        self.prepared = Some((sentence.clone(), rx));
        let control = control.clone();
        std::thread::spawn(move || {
            if control.check().is_err() {
                return;
            }
            if let Ok(bytes) =
                crate::tts::synthesize_within(&sentence, std::time::Duration::from_secs(8))
            {
                if bytes.len() <= 16 * 1024 * 1024 && control.check().is_ok() {
                    let _ = tx.send(bytes);
                }
            }
        });
    }
    pub fn reset(&mut self) {
        self.raw.clear();
        self.prepared = None;
        // Keep attempted: a tool loop must not enqueue many speculative GPU jobs.
    }
    pub fn take(&mut self, final_speech: &str) -> Option<(String, Vec<u8>)> {
        let (sentence, rx) = self.prepared.take()?;
        let suffix = final_speech.strip_prefix(&sentence)?;
        if !suffix.is_empty() && !suffix.starts_with(char::is_whitespace) {
            return None;
        }
        Some((sentence, rx.try_recv().ok()?))
    }
}

fn first_sentence(raw: &str) -> Option<String> {
    // Preview must not synthesize hidden reasoning, source code or links.
    if raw.contains(['<', '>', '`']) || raw.contains("http") || crate::llm::is_leaked_reasoning(raw)
    {
        return None;
    }
    let mut end = None;
    for (index, ch) in raw.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let next = index + ch.len_utf8();
            if raw[next..].starts_with(char::is_whitespace) {
                end = Some(next);
                break;
            }
        }
    }
    let sentence = crate::llm::clean_for_speech(&raw[..end?]);
    let count = sentence.chars().count();
    (count >= 24
        && count <= 240
        && sentence
            .chars()
            .any(|c| ('а'..='я').contains(&c.to_lowercase().next().unwrap_or(c))))
    .then_some(sentence)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn waits_for_complete_sentence_and_filters_private_material() {
        assert!(first_sentence("Сегодня хорошая погода и").is_none());
        assert_eq!(
            first_sentence("Сегодня в городе хорошая погода. Завтра"),
            Some("Сегодня в городе хорошая погода.".into())
        );
        for raw in [
            "<think>Сейчас я рассуждаю об ответе. </think>",
            "```Секретный программный код. ```",
            "Откройте https://example.org. Продолжение",
            "I need to inspect the current tools. Then",
        ] {
            assert!(first_sentence(raw).is_none(), "{}", raw);
        }
    }
    #[test]
    fn changed_or_failed_reply_cannot_play_preview() {
        let mut preview = SpeechPreview::default();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![1]).unwrap();
        preview.prepared = Some(("Программа успешно открыта.".into(), rx));
        assert!(preview.take("Часть действий не выполнена.").is_none());
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![1]).unwrap();
        preview.prepared = Some(("Сегодня в городе хорошая погода.".into(), rx));
        assert!(preview
            .take("Сегодня в городе хорошая погода. Завтра теплее.")
            .is_some());
    }

    #[test]
    fn a_tool_round_discards_preview_without_queuing_another_gpu_job() {
        let mut preview = SpeechPreview::default();
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(vec![1]).unwrap();
        preview.prepared = Some(("Сегодня в городе хорошая погода.".into(), rx));
        preview.attempted = true;
        preview.reset();
        preview.push(
            "Следующее законченное предложение. Продолжение",
            &RequestControl::default(),
        );
        assert!(preview
            .take("Следующее законченное предложение. Продолжение")
            .is_none());
    }
}
