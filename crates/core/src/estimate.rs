//! How long recognizing a piece of audio takes, learned per engine and model from the
//! dictations on this PC - for the bubble's progress ring and the wait shown next to each
//! model. Port of Amber for Android's `RecognitionEstimate`, kept across restarts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

/// How far one new measurement pulls the learned rate.
const LEARNING_RATE: f64 = 0.35;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// Milliseconds of work per millisecond of audio.
    pub per_ms: f64,
    pub samples: u32,
}

pub struct Estimates {
    path: Option<PathBuf>,
    rates: Mutex<HashMap<String, Rate>>,
}

impl Estimates {
    pub fn load(path: &Path) -> Self {
        let rates = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Estimates { path: Some(path.to_path_buf()), rates: Mutex::new(rates) }
    }

    pub fn in_memory() -> Self {
        Estimates { path: None, rates: Mutex::new(HashMap::new()) }
    }

    /// Expected recognition time for [audio_ms] of audio: the learned rate, or [typical]
    /// before anything was measured, plus [overhead_ms] of fixed cost.
    pub fn expected_ms(&self, key: &str, audio_ms: u64, typical: f64, overhead_ms: u64) -> u64 {
        let rate = self.rates.lock().unwrap().get(key).map(|r| r.per_ms).unwrap_or(typical);
        overhead_ms + (audio_ms as f64 * rate) as u64
    }

    pub fn rate(&self, key: &str) -> Option<Rate> {
        self.rates.lock().unwrap().get(key).copied()
    }

    /// Records that [audio_ms] of audio took [took_ms], of which [overhead_ms] was fixed.
    pub fn learn(&self, key: &str, audio_ms: u64, took_ms: u64, overhead_ms: u64) {
        if audio_ms == 0 {
            return;
        }
        let observed = took_ms.saturating_sub(overhead_ms) as f64 / audio_ms as f64;
        let snapshot = {
            let mut rates = self.rates.lock().unwrap();
            let next = match rates.get(key) {
                Some(r) => Rate { per_ms: r.per_ms + (observed - r.per_ms) * LEARNING_RATE, samples: r.samples.saturating_add(1) },
                None => Rate { per_ms: observed, samples: 1 },
            };
            rates.insert(key.to_string(), next);
            serde_json::to_string(&*rates).ok()
        };
        if let (Some(path), Some(text)) = (&self.path, snapshot) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, path);
            }
        }
    }
}

/// Where the progress ring stands after [elapsed_ms] of an expected [expected_ms]: linear up
/// to 90 %, then creeping towards - never reaching - full, so an underestimate does not leave
/// a finished-looking ring waiting. Completion is shown by the done state, not here.
pub fn progress(elapsed_ms: f64, expected_ms: f64) -> f64 {
    const LINEAR_UNTIL: f64 = 0.9;
    const CEILING: f64 = 0.98;
    if expected_ms <= 0.0 {
        return 0.0;
    }
    let t = elapsed_ms / expected_ms;
    if t <= LINEAR_UNTIL {
        return t.max(0.0);
    }
    LINEAR_UNTIL + (CEILING - LINEAR_UNTIL) * (1.0 - (-(t - LINEAR_UNTIL) * 2.0).exp())
}

/// The key a measurement is filed under: where the voice went and which model.
pub fn key(engine: &str, runtime: &str, model: &str) -> String {
    format!("{engine}:{runtime}:{model}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_towards_what_this_pc_measures() {
        let e = Estimates::in_memory();
        let k = key("local", "cuda", "acft");
        assert_eq!(e.expected_ms(&k, 10_000, 0.1, 200), 1_200, "typical rate before any measurement");
        e.learn(&k, 10_000, 700, 200);
        assert_eq!(e.expected_ms(&k, 10_000, 0.1, 200), 700);
        e.learn(&k, 10_000, 1_200, 200);
        assert_eq!(e.expected_ms(&k, 10_000, 0.1, 200), 200 + 675, "0.05 pulled 35 % of the way to 0.1");
        assert_eq!(e.rate(&k).unwrap().samples, 2);
        e.learn(&k, 0, 999, 0);
        assert_eq!(e.rate(&k).unwrap().samples, 2, "no audio, nothing learned");
    }

    #[test]
    fn kept_across_restarts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("estimates.json");
        Estimates::load(&path).learn("a", 1_000, 300, 0);
        assert_eq!(Estimates::load(&path).rate("a").unwrap().per_ms, 0.3);
        std::fs::write(&path, "broken").unwrap();
        assert!(Estimates::load(&path).rate("a").is_none(), "a broken file starts fresh");
    }

    #[test]
    fn the_ring_never_claims_done() {
        assert_eq!(progress(0.0, 1_000.0), 0.0);
        assert_eq!(progress(500.0, 1_000.0), 0.5);
        assert!((progress(900.0, 1_000.0) - 0.9).abs() < 1e-9);
        let late = progress(10_000.0, 1_000.0);
        assert!(late > 0.97 && late < 0.98);
        assert_eq!(progress(100.0, 0.0), 0.0);
    }
}
