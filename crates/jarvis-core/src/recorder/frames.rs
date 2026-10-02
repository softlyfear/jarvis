// Continuously drained microphone frames: slow actions must not replay old speech.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Frames(VecDeque<(Instant, Vec<i16>)>);

impl Frames {
    pub fn push(&mut self, frame: Vec<i16>, at: Instant) {
        self.0.push_back((at, frame));
        while self.0.len() > 3 { self.0.pop_front(); }
    }

    pub fn take(&mut self, now: Instant) -> Option<Vec<i16>> {
        while let Some((at, frame)) = self.0.pop_front() {
            if now.saturating_duration_since(at) < Duration::from_millis(100) { return Some(frame); }
        }
        None
    }

    pub fn clear(&mut self) { self.0.clear(); }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slow_consumer_cannot_replay_an_old_command() {
        let mut frames = Frames::default();
        let now = Instant::now();
        for n in 0..50 { frames.push(vec![n], now); }
        assert_eq!(frames.take(now), Some(vec![47]));
        assert_eq!(frames.take(now + Duration::from_secs(2)), None);
        frames.push(vec![1], now);
        frames.clear();
        assert_eq!(frames.take(now), None);
    }
}
