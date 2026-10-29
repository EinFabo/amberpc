//! Recognize while speaking: a long dictation is cut at real pauses, and each finished part
//! is recognized while the user keeps talking, so after the stop only the last part is left
//! to wait for. Port of Amber for Android's `ChunkPlanner` and `ChunkedTranscription.join`,
//! tests included.

/// Decides where a running recording can be cut. Cuts land only in real pauses, in the
/// middle of the pause, so no word is split. A chunk must be long enough to carry context
/// ([min_chunk_ms]); a chunk without a pause is cut at [max_chunk_ms] - Whisper's window is
/// 30 s - at the quietest moment of its last stretch.
///
/// Fed one frame at a time with the sample count recorded so far, the frame's level and
/// whether it is speech; returns the finished chunk.
pub struct ChunkPlanner {
    sample_rate: u32,
    frame_ms: u32,
    min_chunk_ms: u64,
    pause_ms: u32,
    max_chunk_ms: u64,
    /// How far back from [max_chunk_ms] a forced cut looks for the quietest frame.
    quiet_search_ms: u64,
    /// Where the current chunk began, in samples since the start of the recording.
    pub chunk_start: usize,
    /// The current chunk contains speech; pure silence is never cut off on its own.
    pub speech_since_cut: bool,
    silence_ms: u32,
    quietest_rms: f32,
    quietest_at: usize,
}

/// Samples [start] until [end] (exclusive) of the recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub start: usize,
    pub end: usize,
}

impl ChunkPlanner {
    pub fn new(sample_rate: u32, min_chunk_ms: u64) -> Self {
        Self::with_limits(sample_rate, min_chunk_ms, 25_000, 2_000)
    }

    pub fn with_limits(sample_rate: u32, min_chunk_ms: u64, max_chunk_ms: u64, quiet_search_ms: u64) -> Self {
        ChunkPlanner {
            sample_rate,
            frame_ms: 20,
            min_chunk_ms,
            pause_ms: 450,
            max_chunk_ms,
            quiet_search_ms,
            chunk_start: 0,
            speech_since_cut: false,
            silence_ms: 0,
            quietest_rms: f32::MAX,
            quietest_at: 0,
        }
    }

    pub fn frame_samples(&self) -> usize {
        (self.sample_rate * self.frame_ms / 1000) as usize
    }

    pub fn on_frame(&mut self, total_samples: usize, rms: f32, speech: bool) -> Option<Chunk> {
        if speech {
            self.speech_since_cut = true;
            self.silence_ms = 0;
        } else {
            self.silence_ms += self.frame_ms;
        }

        let chunk_ms = self.ms(total_samples - self.chunk_start);
        if chunk_ms >= self.max_chunk_ms - self.quiet_search_ms && rms < self.quietest_rms {
            self.quietest_rms = rms;
            self.quietest_at = total_samples;
        }

        if !self.speech_since_cut && chunk_ms >= self.max_chunk_ms {
            // Long silence (holding without speaking): nothing to recognize - skip past it.
            self.chunk_start = total_samples;
            self.quietest_rms = f32::MAX;
            return None;
        }

        let cut = if self.speech_since_cut && chunk_ms >= self.min_chunk_ms && self.silence_ms >= self.pause_ms {
            total_samples - self.samples(self.silence_ms / 2)
        } else if chunk_ms >= self.max_chunk_ms {
            if self.quietest_at > self.chunk_start { self.quietest_at } else { total_samples }
        } else {
            return None;
        };

        let chunk = Chunk { start: self.chunk_start, end: cut };
        self.chunk_start = cut;
        // A forced cut can land in speech, so what follows may still be speech.
        self.speech_since_cut = self.ms(total_samples - cut) > 0 && self.silence_ms == 0;
        self.silence_ms = 0;
        self.quietest_rms = f32::MAX;
        self.quietest_at = 0;
        Some(chunk)
    }

    fn ms(&self, samples: usize) -> u64 {
        samples as u64 * 1000 / self.sample_rate as u64
    }

    fn samples(&self, ms: u32) -> usize {
        (ms as u64 * self.sample_rate as u64 / 1000) as usize
    }
}

/// Speech or pause, frame by frame: above the quietest level heard so far (capped), the same
/// rule `audio::has_speech` judges a whole recording with.
#[derive(Default)]
pub struct FrameVad {
    floor: Option<f32>,
}

impl FrameVad {
    pub fn is_speech(&mut self, rms: f32) -> bool {
        let floor = self.floor.map_or(rms, |f| f.min(rms)).min(0.02);
        self.floor = Some(floor);
        rms > (floor * 4.0).clamp(0.006, 0.03)
    }
}

/// Joins chunk texts with single spaces; a chunk never starts with stray punctuation.
pub fn join(parts: &[String]) -> String {
    parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).fold(String::new(), |acc, part| {
        if acc.is_empty() {
            part.to_string()
        } else if part.starts_with([',', '.', ';', ':', '!', '?']) {
            acc + part
        } else {
            acc + " " + part
        }
    })
}

/// What was recognized while the user was still speaking.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parts {
    pub texts: Vec<String>,
    /// Where the tail begins, in samples of the recording.
    pub cut_at: usize,
    /// A part failed: the parts are worthless as a set, the whole recording goes instead.
    pub failed: bool,
    /// The language the first part was recognized in, kept for the rest.
    pub language: Option<String>,
    /// The engine the parts went to, for the history and statistics.
    pub engine: String,
}

impl Parts {
    /// Worth using for a recording of [total] samples: something recognized, nothing failed.
    pub fn usable(&self, total: usize) -> bool {
        !self.failed && self.cut_at > 0 && self.cut_at <= total && !self.texts.is_empty()
    }

    /// The whole dictation: the parts and the tail, each cleaned on its own - Whisper's
    /// hallucinated credits are recognized only as a whole part, not inside the joined text.
    pub fn with_tail(&self, tail: Option<&str>) -> String {
        let mut all: Vec<String> = self.texts.iter().map(|t| crate::text::clean(t)).collect();
        if let Some(t) = tail {
            all.push(crate::text::clean(t));
        }
        join(&all)
    }
}

/// Whisper names the language it detected ("german"); the next request wants the code.
pub fn language_code(name: &str) -> Option<&'static str> {
    const NAMES: &[(&str, &str)] = &[
        ("english", "en"),
        ("german", "de"),
        ("french", "fr"),
        ("spanish", "es"),
        ("italian", "it"),
        ("dutch", "nl"),
        ("polish", "pl"),
        ("turkish", "tr"),
        ("portuguese", "pt"),
        ("russian", "ru"),
        ("ukrainian", "uk"),
        ("czech", "cs"),
        ("swedish", "sv"),
        ("danish", "da"),
        ("norwegian", "no"),
        ("finnish", "fi"),
        ("greek", "el"),
        ("hungarian", "hu"),
        ("romanian", "ro"),
        ("croatian", "hr"),
        ("serbian", "sr"),
        ("arabic", "ar"),
        ("japanese", "ja"),
        ("chinese", "zh"),
        ("korean", "ko"),
    ];
    let lower = name.trim().to_lowercase();
    NAMES.iter().find(|(n, c)| *n == lower || *c == lower).map(|(_, c)| *c)
}

/// What the next chunk gets as its prompt: the user's words and the end of the text so far -
/// Whisper's prompt holds about 224 tokens, and the end is what carries the sentence.
pub fn prompt(vocabulary: &str, so_far: &[String]) -> String {
    const PROMPT_CHARS: usize = 200;
    let text = join(so_far);
    let skip = text.chars().count().saturating_sub(PROMPT_CHARS);
    let tail: String = text.chars().skip(skip).collect();
    [vocabulary.trim(), tail.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;
    const FRAME: usize = (RATE / 50) as usize;

    fn seconds(s: f64) -> usize {
        (s * 50.0) as usize
    }

    /// Feeds frames of speech or silence; returns the chunks and the samples fed.
    fn feed(planner: &mut ChunkPlanner, pattern: &[(bool, usize)], start: usize) -> (Vec<Chunk>, usize) {
        let mut chunks = Vec::new();
        let mut samples = start;
        for &(speech, frames) in pattern {
            for _ in 0..frames {
                samples += FRAME;
                if let Some(c) = planner.on_frame(samples, if speech { 2_000.0 } else { 50.0 }, speech) {
                    chunks.push(c);
                }
            }
        }
        (chunks, samples)
    }

    #[test]
    fn cuts_in_the_middle_of_a_pause_once_the_chunk_is_long_enough() {
        let mut planner = ChunkPlanner::new(RATE, 4_000);
        let (chunks, _) = feed(&mut planner, &[(true, seconds(5.0)), (false, seconds(0.6))], 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start, 0);
        // The pause reaches 450 ms after 23 frames (460 ms); the cut goes 230 ms back into
        // the middle of the pause: 5 s of speech + 230 ms.
        assert_eq!(chunks[0].end, 5 * RATE as usize + 230 * RATE as usize / 1000);
    }

    #[test]
    fn a_short_phrase_followed_by_a_pause_is_not_cut_off_on_its_own() {
        let mut planner = ChunkPlanner::new(RATE, 4_000);
        let (chunks, _) = feed(&mut planner, &[(true, seconds(2.0)), (false, seconds(0.8)), (true, seconds(1.0))], 0);
        assert!(chunks.is_empty(), "too short to carry context: {chunks:?}");
    }

    #[test]
    fn a_breath_shorter_than_the_pause_does_not_cut() {
        let mut planner = ChunkPlanner::new(RATE, 4_000);
        let (chunks, _) = feed(&mut planner, &[(true, seconds(5.0)), (false, seconds(0.3)), (true, seconds(2.0))], 0);
        assert!(chunks.is_empty());
    }

    #[test]
    fn speech_without_any_pause_is_cut_at_the_quietest_moment_before_the_limit() {
        let mut planner = ChunkPlanner::with_limits(RATE, 4_000, 10_000, 2_000);
        let mut chunks = Vec::new();
        let mut samples = 0;
        for i in 0..seconds(11.0) {
            samples += FRAME;
            // One quieter (but still speech) frame at 9 s.
            let rms = if i == seconds(9.0) { 900.0 } else { 2_000.0 };
            if let Some(c) = planner.on_frame(samples, rms, true) {
                chunks.push(c);
            }
        }
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].end, (seconds(9.0) + 1) * FRAME);
    }

    #[test]
    fn long_silence_without_speech_is_skipped_not_chunked() {
        let mut planner = ChunkPlanner::with_limits(RATE, 4_000, 10_000, 2_000);
        let (chunks, end) = feed(&mut planner, &[(false, seconds(12.0))], 0);
        assert!(chunks.is_empty());
        assert!(planner.chunk_start > 0 && planner.chunk_start <= end);
        assert!(!planner.speech_since_cut);
    }

    #[test]
    fn a_pause_cut_leaves_no_speech_pending_new_speech_does() {
        let mut planner = ChunkPlanner::new(RATE, 4_000);
        feed(&mut planner, &[(true, seconds(5.0)), (false, seconds(0.6))], 0);
        assert!(!planner.speech_since_cut);
        planner.on_frame(seconds(6.0) * FRAME, 2_000.0, true);
        assert!(planner.speech_since_cut);
    }

    #[test]
    fn chunk_texts_join_with_single_spaces_and_keep_punctuation_attached() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(join(&s(&["Hallo Welt", ", wie geht's?"])), "Hallo Welt, wie geht's?");
        assert_eq!(join(&s(&[" Erster Satz. ", "", "Zweiter Satz."])), "Erster Satz. Zweiter Satz.");
    }

    #[test]
    fn the_prompt_carries_the_words_and_the_end_of_the_text() {
        assert_eq!(prompt("", &[]), "");
        assert_eq!(prompt("Tauri", &["Das ist".into()]), "Tauri Das ist");
        let long = "ä".repeat(300);
        let p = prompt("", &[long]);
        assert_eq!(p.chars().count(), 200, "cut by characters, not bytes");
    }

    #[test]
    fn parts_join_with_the_tail_each_cleaned() {
        let parts = Parts { texts: vec![" Das ist".into(), "ein Test.".into()], cut_at: 100, ..Parts::default() };
        assert!(parts.usable(200) && !parts.usable(50));
        assert_eq!(parts.with_tail(Some(" und noch mehr.")), "Das ist ein Test. und noch mehr.");
        // A credit Whisper invents on the quiet tail is dropped, not glued to the text.
        assert_eq!(parts.with_tail(Some(" Untertitel der Amara.org-Community")), "Das ist ein Test.");
        assert_eq!(parts.with_tail(None), "Das ist ein Test.");
        assert!(!Parts { failed: true, ..parts.clone() }.usable(200));
        assert!(!Parts { texts: vec![], ..parts }.usable(200));
    }

    #[test]
    fn language_names_become_codes() {
        assert_eq!(language_code("german"), Some("de"));
        assert_eq!(language_code(" English "), Some("en"));
        assert_eq!(language_code("de"), Some("de"));
        assert_eq!(language_code("klingon"), None);
    }

    #[test]
    fn the_frame_vad_tells_voice_from_room_noise() {
        let mut vad = FrameVad::default();
        assert!(!vad.is_speech(0.002));
        assert!(!vad.is_speech(0.004));
        assert!(vad.is_speech(0.08));
        assert!(!vad.is_speech(0.003));
    }
}
