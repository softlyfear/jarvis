// nnnoiseless - used for both noise suppression and VAD.
// each consumer needs its own DenoiseState (stateful per-stream),
// so this doesn't go through the registry. just centralizes creation.

use nnnoiseless::DenoiseState;
use crate::rnnoise_stream::FrameAdapter;
use crate::config;

// noise suppression instance
pub struct NnnoiselessNS {
    state: Box<DenoiseState<'static>>,
    adapter: FrameAdapter,
}

impl NnnoiselessNS {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            adapter: FrameAdapter::new(),
        }
    }

    pub fn process(&mut self, input: &[i16]) -> Vec<i16> {
        let state = &mut self.state;
        self.adapter.process(input, |out, frame| state.process_frame(out, frame)).0
    }

    pub fn reset(&mut self) {
        self.state = DenoiseState::new();
        self.adapter = FrameAdapter::new();
    }
}

// VAD instance
pub struct NnnoiselessVAD {
    state: Box<DenoiseState<'static>>,
    adapter: FrameAdapter,
}

impl NnnoiselessVAD {
    pub fn new() -> Self {
        Self {
            state: DenoiseState::new(),
            adapter: FrameAdapter::new(),
        }
    }

    pub fn detect(&mut self, input: &[i16]) -> (bool, f32) {
        let state = &mut self.state;
        let (_, confidence) = self.adapter.process(input, |out, frame| state.process_frame(out, frame));
        (confidence >= config::VAD_NNNOISELESS_THRESHOLD, confidence)
    }

    pub fn reset(&mut self) {
        self.state = DenoiseState::new();
        self.adapter = FrameAdapter::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resetting_noise_suppression_discards_model_state_and_pending_audio() {
        let mut used = NnnoiselessNS::new();
        used.process(&[20_000; 511]);
        used.reset();
        let mut fresh = NnnoiselessNS::new();
        let input: Vec<i16> = (0..512).map(|i| (5000.0 * (i as f32 / 16.0).sin()) as i16).collect();
        assert_eq!(used.process(&input), fresh.process(&input));
        assert_eq!(used.process(&[0; 512]), fresh.process(&[0; 512]));
    }

    #[test]
    fn actual_denoiser_preserves_callback_sizes_and_silent_vad() {
        let mut ns = NnnoiselessNS::new();
        let mut vad = NnnoiselessVAD::new();
        for size in [1, 7, 160, 511, 512, 1000] {
            assert_eq!(ns.process(&vec![0; size]), vec![0; size]);
            assert_eq!(vad.detect(&vec![0; size]), (false, 0.0));
        }
        vad.detect(&[15_000; 511]);
        vad.reset();
        assert_eq!(vad.detect(&[0; 7]), NnnoiselessVAD::new().detect(&[0; 7]));
    }
}
