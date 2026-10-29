//! The statistics, kept on this PC only: one line of JSON per dictation with counts and
//! timings - never the text or the audio. Port of Amber for Android's `StatsLedger` and
//! `StatsSummary`, without the per-trust and cost parts.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

pub const DEFAULT_TYPING_WPM: u32 = 40;
/// Below this much speech in total, a speaking pace says nothing.
const MIN_AUDIO_FOR_PACE_MS: u64 = 30_000;

/// One finished dictation as the statistics see it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stat {
    /// Milliseconds since 1970 (UTC).
    pub at: u64,
    /// Local calendar day, as days since 1970.
    pub day: i64,
    pub engine: String,
    pub words: u32,
    pub audio_ms: u64,
    /// From the stop to the text being there: what the user waited.
    pub wait_ms: u64,
    pub program: String,
}

/// Append-only JSON Lines; reading parses them all, which stays fast - a year of heavy use is
/// a few thousand lines.
pub struct Ledger {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Ledger {
    pub fn new(path: &Path) -> Self {
        Ledger { path: path.to_path_buf(), lock: Mutex::new(()) }
    }

    pub fn record(&self, stat: &Stat) {
        let _guard = self.lock.lock().unwrap();
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{}", serde_json::to_string(stat).expect("stat serializes"));
        }
    }

    /// Every recorded dictation; lines that do not parse (a crash mid-write) are skipped.
    pub fn all(&self) -> Vec<Stat> {
        let _guard = self.lock.lock().unwrap();
        std::fs::read_to_string(&self.path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    pub fn clear(&self) {
        let _guard = self.lock.lock().unwrap();
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The local calendar day of [at_ms], given the local offset from UTC in minutes.
pub fn day_of(at_ms: u64, utc_offset_minutes: i32) -> i64 {
    (at_ms as i64 + utc_offset_minutes as i64 * 60_000).div_euclid(24 * 60 * 60 * 1000)
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub words_today: u32,
    pub dictations_today: u32,
    /// Words per day, oldest first, ending today.
    pub last_days: Vec<u32>,
    pub total_words: u64,
    pub total_dictations: u32,
    /// Typing time the words would have taken minus the time speaking and waiting took.
    pub minutes_saved_today: f64,
    pub minutes_saved_total: f64,
    /// Words per program, most first.
    pub words_by_program: Vec<(String, u64)>,
    /// Median wait after stopping, per engine.
    pub median_wait_ms: BTreeMap<String, u64>,
    /// Consecutive days with a dictation, ending today or yesterday.
    pub streak_days: u32,
    pub longest_audio_ms: u64,
    /// Words a minute while speaking, over all dictations.
    pub speaking_wpm: Option<u32>,
    pub first_day: Option<i64>,
}

pub fn summary(stats: &[Stat], today: i64, typing_wpm: u32, days: usize) -> Summary {
    let todays: Vec<&Stat> = stats.iter().filter(|s| s.day == today).collect();
    let mut by_day: HashMap<i64, u32> = HashMap::new();
    for s in stats {
        *by_day.entry(s.day).or_default() += s.words;
    }
    let mut streak = 0;
    let mut day = if by_day.contains_key(&today) { today } else { today - 1 };
    while by_day.contains_key(&day) {
        streak += 1;
        day -= 1;
    }
    let mut by_program: HashMap<String, u64> = HashMap::new();
    for s in stats.iter().filter(|s| !s.program.is_empty()) {
        *by_program.entry(s.program.clone()).or_default() += s.words as u64;
    }
    let mut words_by_program: Vec<(String, u64)> = by_program.into_iter().collect();
    words_by_program.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut waits: HashMap<String, Vec<u64>> = HashMap::new();
    for s in stats {
        waits.entry(s.engine.clone()).or_default().push(s.wait_ms);
    }
    let total_audio: u64 = stats.iter().map(|s| s.audio_ms).sum();
    let total_words: u64 = stats.iter().map(|s| s.words as u64).sum();
    Summary {
        words_today: todays.iter().map(|s| s.words).sum(),
        dictations_today: todays.len() as u32,
        last_days: (0..days as i64).rev().map(|back| by_day.get(&(today - back)).copied().unwrap_or(0)).collect(),
        total_words,
        total_dictations: stats.len() as u32,
        minutes_saved_today: saved(todays.iter().copied(), typing_wpm),
        minutes_saved_total: saved(stats.iter(), typing_wpm),
        words_by_program,
        median_wait_ms: waits.into_iter().map(|(k, v)| (k, median(v))).collect(),
        streak_days: streak,
        longest_audio_ms: stats.iter().map(|s| s.audio_ms).max().unwrap_or(0),
        speaking_wpm: (total_audio >= MIN_AUDIO_FOR_PACE_MS).then(|| (total_words * 60_000 / total_audio) as u32),
        first_day: stats.iter().map(|s| s.day).min(),
    }
}

fn saved<'a>(stats: impl Iterator<Item = &'a Stat>, typing_wpm: u32) -> f64 {
    let (words, spent_ms) = stats.fold((0u64, 0u64), |(w, t), s| (w + s.words as u64, t + s.audio_ms + s.wait_ms));
    (words as f64 / typing_wpm.max(1) as f64 - spent_ms as f64 / 60_000.0).max(0.0)
}

fn median(mut values: Vec<u64>) -> u64 {
    if values.is_empty() {
        return 0;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(day: i64, words: u32, audio_ms: u64, wait_ms: u64, engine: &str, program: &str) -> Stat {
        Stat { at: day as u64 * 86_400_000, day, engine: engine.into(), words, audio_ms, wait_ms, program: program.into() }
    }

    #[test]
    fn today_streak_and_saved_time() {
        let stats = vec![
            stat(98, 40, 15_000, 500, "local", "discord.exe"),
            stat(99, 20, 8_000, 400, "local", "outlook.exe"),
            stat(100, 80, 30_000, 600, "groq", "discord.exe"),
            stat(100, 10, 4_000, 300, "local", ""),
        ];
        let s = summary(&stats, 100, 40, 7);
        assert_eq!((s.words_today, s.dictations_today), (90, 2));
        assert_eq!(s.streak_days, 3);
        assert_eq!(s.last_days, vec![0, 0, 0, 0, 40, 20, 90]);
        assert_eq!(s.words_by_program, vec![("discord.exe".to_string(), 120), ("outlook.exe".to_string(), 20)]);
        assert_eq!(s.median_wait_ms["local"], 400);
        // 90 words typed at 40 wpm = 2.25 min, spoken and waited 34.9 s.
        assert!((s.minutes_saved_today - (2.25 - 34.9 / 60.0)).abs() < 1e-9);
        assert_eq!(s.speaking_wpm, Some(150 * 60_000 / 57_000));
        assert_eq!(s.first_day, Some(98));
    }

    #[test]
    fn a_streak_may_end_yesterday_and_empty_is_empty() {
        let s = summary(&[stat(99, 5, 2_000, 100, "local", "")], 100, 40, 7);
        assert_eq!(s.streak_days, 1);
        assert_eq!(s.speaking_wpm, None, "too little speech for a pace");
        let empty = summary(&[], 100, 40, 7);
        assert_eq!((empty.total_dictations, empty.streak_days, empty.first_day), (0, 0, None));
    }

    #[test]
    fn local_days_follow_the_offset() {
        // 23:30 UTC on day 100 is already day 101 in Berlin (UTC+1).
        let at = 100 * 86_400_000 + 23 * 3_600_000 + 30 * 60_000;
        assert_eq!(day_of(at, 0), 100);
        assert_eq!(day_of(at, 60), 101);
        assert_eq!(day_of(at, -300), 100);
    }

    #[test]
    fn the_ledger_appends_and_skips_broken_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.jsonl");
        let ledger = Ledger::new(&path);
        ledger.record(&stat(1, 3, 1_000, 100, "local", "x.exe"));
        std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{broken\n").unwrap();
        ledger.record(&stat(2, 4, 1_000, 100, "local", "x.exe"));
        assert_eq!(ledger.all().len(), 2);
        ledger.clear();
        assert!(ledger.all().is_empty());
    }
}
