// Microphone level and spectrum for the GUI orb visualizer.
// One 512-sample frame at 16 kHz (32 ms) -> overall level + BANDS log-spaced bands, all 0..1.

use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use realfft::{RealFftPlanner, RealToComplex};

pub const BANDS: usize = 16;
const SAMPLE_RATE: f32 = 16_000.0;
const MIN_FREQ: f32 = 80.0;
const MAX_FREQ: f32 = 7_500.0;
// dB range mapped to 0..1
const LEVEL_FLOOR_DB: f32 = -60.0;
const LEVEL_CEIL_DB: f32 = -10.0;
const BAND_FLOOR_DB: f32 = -75.0;
const BAND_CEIL_DB: f32 = -20.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub level: f32,
    pub bands: Vec<f32>,
}

struct Analyzer {
    len: usize,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
}

static ANALYZER: Lazy<Mutex<Option<Analyzer>>> = Lazy::new(|| Mutex::new(None));

fn to_unit(db: f32, floor: f32, ceil: f32) -> f32 {
    ((db - floor) / (ceil - floor)).clamp(0.0, 1.0)
}

fn round2(x: f32) -> f32 {
    (x * 100.0).round() / 100.0
}

pub fn analyze(samples: &[i16]) -> Frame {
    let n = samples.len();
    if n < 64 {
        return Frame { level: 0.0, bands: vec![0.0; BANDS] };
    }

    let mut guard = ANALYZER.lock();
    if guard.as_ref().map(|a| a.len != n).unwrap_or(true) {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let window = (0..n)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32).cos())
            .collect();
        *guard = Some(Analyzer { len: n, fft, window });
    }
    let analyzer = guard.as_ref().expect("initialized above");

    // overall level from RMS
    let norm: Vec<f32> = samples.iter().map(|s| *s as f32 / 32768.0).collect();
    let rms = (norm.iter().map(|x| x * x).sum::<f32>() / n as f32).sqrt();
    let level = to_unit(20.0 * rms.max(1e-9).log10(), LEVEL_FLOOR_DB, LEVEL_CEIL_DB);

    let mut input: Vec<f32> = norm.iter().zip(&analyzer.window).map(|(x, w)| x * w).collect();
    let mut spectrum = analyzer.fft.make_output_vec();
    if analyzer.fft.process(&mut input, &mut spectrum).is_err() {
        return Frame { level: round2(level), bands: vec![0.0; BANDS] };
    }

    let bin_hz = SAMPLE_RATE / n as f32;
    let ratio = (MAX_FREQ / MIN_FREQ).powf(1.0 / BANDS as f32);
    let bands = (0..BANDS)
        .map(|b| {
            let lo = MIN_FREQ * ratio.powi(b as i32);
            let hi = lo * ratio;
            let lo_bin = ((lo / bin_hz).floor() as usize).max(1);
            let hi_bin = ((hi / bin_hz).ceil() as usize).clamp(lo_bin + 1, spectrum.len());
            let peak = spectrum[lo_bin..hi_bin].iter().map(|c| c.norm()).fold(0.0f32, f32::max);
            // amplitude relative to a full-scale sine through the Hann window (n / 4)
            let db = 20.0 * (peak / (n as f32 / 4.0)).max(1e-9).log10();
            round2(to_unit(db, BAND_FLOOR_DB, BAND_CEIL_DB))
        })
        .collect();

    Frame { level: round2(level), bands }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, amp: f32) -> Vec<i16> {
        (0..512)
            .map(|i| (amp * 32767.0 * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE).sin()) as i16)
            .collect()
    }

    #[test]
    fn silence_is_zero() {
        let f = analyze(&[0i16; 512]);
        assert_eq!(f.level, 0.0);
        assert!(f.bands.iter().all(|b| *b == 0.0));
        assert_eq!(f.bands.len(), BANDS);
    }

    #[test]
    fn loud_tone_lights_its_band_only() {
        let f = analyze(&sine(1000.0, 0.5));
        assert!(f.level > 0.8, "{:?}", f);
        let (peak_band, _) = f.bands.iter().enumerate().fold((0, 0.0f32), |acc, (i, v)| if *v > acc.1 { (i, *v) } else { acc });
        // 1 kHz on a log scale 80..7500 Hz lands in the middle
        assert!((7..=10).contains(&peak_band), "peak in band {} of {:?}", peak_band, f.bands);
        assert!(f.bands[0] < 0.3 && f.bands[BANDS - 1] < 0.3, "{:?}", f.bands);
    }

    #[test]
    fn quiet_tone_is_lower_than_loud() {
        assert!(analyze(&sine(500.0, 0.01)).level < analyze(&sine(500.0, 0.5)).level);
    }
}
