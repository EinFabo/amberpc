//! What Amber remembers about dictations: the history (encrypted for the Windows account
//! with DPAPI, like the API keys), the statistics (counts and timings only), and the last
//! recording whose recognition failed, for "Recognize again".

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use amber_core::history::{Entry, History};
use amber_core::settings::Settings;
use amber_core::stats::{self, Ledger, Stat};

/// How long a failed recording waits for "Recognize again".
const KEEP_FAILED: Duration = Duration::from_secs(10 * 60);

/// A recording whose recognition failed - only in memory, never on disk.
pub struct Failed {
    pub wav: Vec<u8>,
    pub spoken_ms: u64,
    pub program: Option<String>,
    at: Instant,
}

pub struct Memory {
    history_path: PathBuf,
    history: Mutex<History>,
    /// The history switch as it is now, not as a dictation's settings snapshot had it.
    enabled: std::sync::atomic::AtomicBool,
    pub ledger: Ledger,
    failed: Mutex<Option<Failed>>,
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Memory {
    pub fn load(data_dir: &Path, settings: &Settings) -> Memory {
        let history_path = data_dir.join("history.bin");
        let mut history = match std::fs::read(&history_path) {
            Ok(blob) => match crate::secrets::unprotect(&blob) {
                Ok(plain) => History::from_bytes(&plain),
                Err(e) => {
                    // Another Windows account, a restored backup: kept aside, not overwritten.
                    let aside = data_dir.join(format!("history.unreadable-{}.bin", now_ms()));
                    crate::diag::warn(format!("history unreadable ({e}), kept as {}", aside.display()));
                    let _ = std::fs::rename(&history_path, aside);
                    History::default()
                }
            },
            Err(_) => History::default(),
        };
        history.prune(now_ms(), settings.history_days);
        Memory {
            history_path,
            history: Mutex::new(history),
            enabled: std::sync::atomic::AtomicBool::new(settings.history),
            ledger: Ledger::new(&data_dir.join("stats.jsonl")),
            failed: Mutex::new(None),
        }
    }

    fn save(&self, history: &History) {
        let result = crate::secrets::protect(&history.to_bytes()).and_then(|blob| {
            let tmp = self.history_path.with_extension("tmp");
            std::fs::write(&tmp, blob).and_then(|_| std::fs::rename(&tmp, &self.history_path)).map_err(|e| e.to_string())
        });
        if let Err(e) = result {
            crate::diag::warn(format!("history not saved: {e}"));
        }
    }

    /// Keeps a finished dictation in the history and counts it in the statistics, as the
    /// settings allow.
    pub fn remember(&self, s: &Settings, text: &str, program: &str, engine: &str, inserted: bool, audio_ms: u64, wait_ms: u64) {
        let at = now_ms();
        if s.history {
            // Under the lock, so switching the history off (which clears under it too) wins
            // over a dictation that was still being recognized.
            let mut history = self.history.lock().unwrap();
            if !self.enabled.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            history.add(Entry { at, text: text.to_string(), program: program.to_string(), engine: engine.to_string(), inserted }, at, s.history_days);
            self.save(&history);
        }
        if s.statistics {
            self.ledger.record(&Stat {
                at,
                day: stats::day_of(at, utc_offset_minutes()),
                engine: engine.to_string(),
                words: amber_core::text::words(text) as u32,
                audio_ms,
                wait_ms,
                program: program.to_string(),
            });
        }
    }

    pub fn history(&self) -> Vec<Entry> {
        self.history.lock().unwrap().entries().to_vec()
    }

    pub fn latest(&self) -> Option<Entry> {
        self.history.lock().unwrap().latest().cloned()
    }

    pub fn forget(&self, at: u64) {
        let mut history = self.history.lock().unwrap();
        history.remove(at);
        self.save(&history);
    }

    /// Follows the history switch: off clears what is kept and refuses what is still coming.
    pub fn set_enabled(&self, on: bool) {
        let _guard = self.history.lock().unwrap();
        self.enabled.store(on, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn clear_history(&self) {
        let mut history = self.history.lock().unwrap();
        history.clear();
        let _ = std::fs::remove_file(&self.history_path);
    }

    /// After the settings changed how long dictations are kept.
    pub fn prune(&self, days: u32) {
        let mut history = self.history.lock().unwrap();
        history.prune(now_ms(), days);
        self.save(&history);
    }

    pub fn keep_failed(&self, wav: Vec<u8>, spoken_ms: u64, program: Option<String>) {
        *self.failed.lock().unwrap() = Some(Failed { wav, spoken_ms, program, at: Instant::now() });
    }

    /// A taken recording back after another failed try - only into an empty slot, and with
    /// its own age, so it still goes after ten minutes.
    pub fn return_failed(&self, failed: Failed) {
        let mut slot = self.failed.lock().unwrap();
        if slot.is_none() {
            *slot = Some(failed);
        }
    }

    /// The failed recording, if it is recent enough - taken, so it is recognized once.
    pub fn take_failed(&self) -> Option<Failed> {
        self.failed.lock().unwrap().take().filter(|f| f.at.elapsed() < KEEP_FAILED)
    }
}

/// The local time zone's offset from UTC in minutes, daylight saving included - for the
/// statistics' calendar days.
#[cfg(windows)]
pub fn utc_offset_minutes() -> i32 {
    use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    // 2 = TIME_ZONE_ID_DAYLIGHT. Bias is "UTC = local + bias".
    let id = unsafe { GetTimeZoneInformation(&mut tz) };
    let bias = tz.Bias + if id == 2 { tz.DaylightBias } else { tz.StandardBias };
    -bias
}

#[cfg(not(windows))]
pub fn utc_offset_minutes() -> i32 {
    0
}
