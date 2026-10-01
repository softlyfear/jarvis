// Stream adapter for the 16 kHz microphone and RNNoise's 48 kHz, 10 ms frames.
// This is used for VAD/gating; the recognizers still receive the original microphone audio.

use std::collections::VecDeque;

const RATIO: usize = 3;
const FRAME_48KHZ: usize = nnnoiseless::DenoiseState::FRAME_SIZE;
const FRAME_16KHZ: usize = FRAME_48KHZ / RATIO;

pub(crate) struct FrameAdapter {
    previous: f32,
    pending: Vec<f32>,
    output: VecDeque<i16>,
    confidence: f32,
}

impl FrameAdapter {
    pub(crate) fn new() -> Self {
        Self {
            previous: 0.0,
            pending: Vec::new(),
            // One frame of buffering makes the output length independent of callback size.
            // RNNoise has its own additional algorithmic delay.
            output: std::iter::repeat_n(0, FRAME_16KHZ).collect(),
            confidence: 0.0,
        }
    }

    pub(crate) fn process(
        &mut self,
        input: &[i16],
        mut denoise: impl FnMut(&mut [f32; FRAME_48KHZ], &[f32; FRAME_48KHZ]) -> f32,
    ) -> (Vec<i16>, f32) {
        // Causal linear interpolation preserves phase across microphone callbacks.
        for &sample in input {
            let sample = sample as f32;
            for step in 1..=RATIO {
                self.pending.push(self.previous + (sample - self.previous) * step as f32 / RATIO as f32);
            }
            self.previous = sample;
        }

        let frames = self.pending.len() / FRAME_48KHZ;
        let mut sum = 0.0;
        for frame in self.pending[..frames * FRAME_48KHZ].chunks_exact(FRAME_48KHZ) {
            let mut samples = [0.0; FRAME_48KHZ];
            samples.copy_from_slice(frame);
            let mut out = [0.0; FRAME_48KHZ];
            sum += denoise(&mut out, &samples);
            // Average before decimation to reduce high-frequency components in the VAD input.
            self.output.extend(out.chunks_exact(RATIO).map(|chunk| {
                (chunk.iter().sum::<f32>() / RATIO as f32).clamp(i16::MIN as f32, i16::MAX as f32) as i16
            }));
        }
        self.pending.drain(..frames * FRAME_48KHZ);
        if frames > 0 {
            self.confidence = sum / frames as f32;
        }

        // A pending partial frame contains fewer than FRAME_16KHZ microphone samples;
        // the initial output frame covers that deficit without passthrough or duplication.
        let output = (0..input.len()).map(|_| self.output.pop_front().expect("RNNoise stream alignment")).collect();
        (output, self.confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passthrough(out: &mut [f32; FRAME_48KHZ], input: &[f32; FRAME_48KHZ]) -> f32 {
        out.copy_from_slice(input);
        0.9
    }

    fn tone() -> Vec<i16> {
        (0..16_000).map(|i| (10_000.0 * (i as f32 * std::f32::consts::TAU * 1_000.0 / 16_000.0).sin()) as i16).collect()
    }

    #[test]
    fn callbacks_preserve_duration_and_sample_order() {
        let samples = tone();
        let expected = FrameAdapter::new().process(&samples, passthrough).0;
        for size in [1, 7, 160, 480, 511, 512, 1000] {
            let mut adapter = FrameAdapter::new();
            let mut actual = Vec::new();
            for chunk in samples.chunks(size) {
                let (out, _) = adapter.process(chunk, passthrough);
                assert_eq!(out.len(), chunk.len());
                actual.extend(out);
            }
            assert_eq!(actual, expected, "callback size {}", size);
            assert!(adapter.pending.len() < FRAME_48KHZ);
            assert!(adapter.output.len() <= FRAME_16KHZ);
        }
    }

    #[test]
    fn denoiser_receives_48khz_and_pcm_amplitude() {
        let samples = tone();
        let mut received = Vec::new();
        let (output, _) = FrameAdapter::new().process(&samples, |out, input| {
            received.extend_from_slice(input);
            passthrough(out, input)
        });
        assert_eq!(received.len(), 48_000);
        let crossings = received.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((999..=1001).contains(&crossings));
        assert!(received.iter().any(|s| s.abs() > 9000.0));
        assert!(output[..FRAME_16KHZ].iter().all(|s| *s == 0));
        let paired = output[FRAME_16KHZ..].iter().zip(&samples);
        let rms_error = (paired.map(|(out, raw)| (*out as f64 - *raw as f64).powi(2)).sum::<f64>() / (samples.len() - FRAME_16KHZ) as f64).sqrt();
        assert!(rms_error < 1000.0, "16/48/16 kHz round-trip error: {}", rms_error);
    }

    #[test]
    fn partial_frames_do_not_pretend_to_be_speech_or_repeat_input() {
        let mut adapter = FrameAdapter::new();
        let (out, confidence) = adapter.process(&[1000; 7], passthrough);
        assert_eq!(out, vec![0; 7]);
        assert_eq!(confidence, 0.0);
        let (_, confidence) = adapter.process(&[1000; FRAME_16KHZ], passthrough);
        assert_eq!(confidence, 0.9);
        let (out, confidence) = adapter.process(&[], passthrough);
        assert!(out.is_empty());
        assert_eq!(confidence, 0.9);
    }

    #[test]
    fn denoised_output_stays_in_pcm_range() {
        let (out, _) = FrameAdapter::new().process(&[32767; 512], |out, _| {
            out.fill(1e6);
            1.0
        });
        assert_eq!(out.len(), 512);
        assert_eq!(out[FRAME_16KHZ], i16::MAX);
    }
}
