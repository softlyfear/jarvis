// End of an utterance from the audio level alone, so the voice server recognizes speech
// without the Vosk speech recognizer running alongside. The noise floor follows the
// microphone: a quiet headset and a noisy laptop mic both work without a setting.

const SAMPLE_RATE: usize = 16_000;
// shorter sounds (a click, a cough) are not an utterance
const MIN_SPEECH_SAMPLES: usize = SAMPLE_RATE / 4;
// a pause this long ends the utterance (Vosk waits about as long)
const END_SILENCE_SAMPLES: usize = SAMPLE_RATE * 6 / 10;
// a TV or a fan never falls silent: cut the utterance here
const MAX_UTTERANCE_SAMPLES: usize = SAMPLE_RATE * 15;
// speech is this much louder than the noise floor, and never quieter than the absolute minimum
const SPEECH_OVER_FLOOR: f32 = 3.0;
const MIN_SPEECH_RMS: f32 = 150.0;
// audio kept before the first loud frame: soft first consonants
const PREROLL_SAMPLES: usize = SAMPLE_RATE * 3 / 10;

#[derive(Debug, Clone)]
pub struct Endpointer {
    floor: Option<f32>,
    speech: usize,
    silence: usize,
    total: usize,
    // length of the utterance that has just ended, with the preroll
    ended: usize,
}

impl Default for Endpointer {
    fn default() -> Self {
        Self { floor: None, speech: 0, silence: 0, total: 0, ended: 0 }
    }
}

impl Endpointer {
    // true when this frame ends an utterance that had speech in it
    pub fn push(&mut self, frame: &[i16]) -> bool {
        let level = rms(frame);
        let floor = *self.floor.get_or_insert(level);
        let speaking = level > (floor * SPEECH_OVER_FLOOR).max(MIN_SPEECH_RMS);
        // the floor drops at once to a quieter frame and rises slowly through steady noise
        self.floor = Some(if level < floor { floor * 0.7 + level * 0.3 } else if speaking { floor } else { floor * 0.98 + level * 0.02 });

        if speaking {
            self.speech += frame.len();
            self.silence = 0;
        } else if self.speech > 0 {
            self.silence += frame.len();
        }
        if self.speech > 0 {
            self.total += frame.len();
        }

        let ended = self.speech >= MIN_SPEECH_SAMPLES && (self.silence >= END_SILENCE_SAMPLES || self.total >= MAX_UTTERANCE_SAMPLES);
        // a short noise followed by silence is forgotten
        let noise = self.speech < MIN_SPEECH_SAMPLES && self.silence >= END_SILENCE_SAMPLES;
        if ended {
            self.ended = self.total + PREROLL_SAMPLES;
        }
        if ended || noise {
            self.reset();
        }
        ended
    }

    // how many of the last samples the utterance that has just ended takes
    pub fn utterance_samples(&self) -> usize {
        self.ended
    }

    // forget the current utterance; the noise floor is kept
    pub fn reset(&mut self) {
        self.speech = 0;
        self.silence = 0;
        self.total = 0;
    }
}

fn rms(frame: &[i16]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    let sum: f64 = frame.iter().map(|&s| (s as f64) * (s as f64)).sum();
    (sum / frame.len() as f64).sqrt() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: usize = 512;

    fn tone(amplitude: f32, seconds: f32) -> Vec<Vec<i16>> {
        let frames = (seconds * SAMPLE_RATE as f32) as usize / FRAME;
        (0..frames)
            .map(|f| (0..FRAME).map(|i| (amplitude * ((f * FRAME + i) as f32 * 0.05).sin()) as i16).collect())
            .collect()
    }

    // the frame index (from the start) at which the utterance ended
    fn end_at(e: &mut Endpointer, parts: &[(f32, f32)]) -> Option<usize> {
        let mut n = 0;
        for &(amplitude, seconds) in parts {
            for frame in tone(amplitude, seconds) {
                n += 1;
                if e.push(&frame) {
                    return Some(n);
                }
            }
        }
        None
    }

    #[test]
    fn speech_then_a_pause_ends_the_utterance() {
        let mut e = Endpointer::default();
        // quiet room, a second of speech, then silence: ends about 0.6 s into the pause
        let at = end_at(&mut e, &[(50.0, 1.0), (3000.0, 1.0), (50.0, 2.0)]).unwrap();
        let pause_frames = at - 2 * (SAMPLE_RATE / FRAME);
        assert!((17..=21).contains(&pause_frames), "{}", pause_frames);
    }

    #[test]
    fn a_short_pause_inside_a_phrase_does_not_end_it() {
        let mut e = Endpointer::default();
        assert_eq!(end_at(&mut e, &[(50.0, 0.5), (3000.0, 0.8), (50.0, 0.3), (3000.0, 0.8)]), None);
    }

    #[test]
    fn clicks_and_steady_noise_are_not_utterances() {
        let mut e = Endpointer::default();
        assert_eq!(end_at(&mut e, &[(50.0, 0.5), (3000.0, 0.1), (50.0, 2.0)]), None);
        // a fan at a level a fixed threshold of 150 would call speech forever
        let mut e = Endpointer::default();
        assert_eq!(end_at(&mut e, &[(400.0, 5.0)]), None);
        // speech over that fan still ends normally
        assert!(end_at(&mut e, &[(4000.0, 1.0), (400.0, 1.0)]).is_some());
    }

    #[test]
    fn endless_sound_is_cut() {
        let mut e = Endpointer::default();
        assert!(end_at(&mut e, &[(50.0, 0.5), (3000.0, 20.0)]).is_some());
    }
}
