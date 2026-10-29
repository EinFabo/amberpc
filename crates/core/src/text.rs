//! From what the engine heard to what lands in the window.

use regex::Regex;
use std::sync::LazyLock;

/// Sound annotations such as "[Musik]", "(Lachen)", "*Applaus*", "[BLANK_AUDIO]".
static ANNOTATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*]|\([^)]*\)|\*[^*]*\*|♪+").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t]+").unwrap());
static SPACE_BEFORE_PUNCT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" ([.,!?;:])").unwrap());

/// Whisper was trained on subtitles and fills silence with their credits. Kept in the
/// language Whisper hears them in - they match speech data, not UI text.
static HALLUCINATIONS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)^untertitel (im auftrag des zdf|der amara\.org-community|von stephanie geiges).*$",
        r"(?i)^untertitelung (des zdf|aufgrund der amara\.org-community).*$",
        r"(?i)^(vielen dank|danke) fürs zuschauen[.!]?$",
        r"(?i)^(thanks for watching|thank you for watching)[.!]?$",
        r"(?i)^(subtitles by the amara\.org community)[.!]?$",
    ]
    .iter()
    .map(|p| Regex::new(p).unwrap())
    .collect()
});

/// Removes annotations and hallucinated credits, collapses whitespace. Empty means: nothing
/// was said.
pub fn clean(raw: &str) -> String {
    let lines: Vec<String> = raw
        .lines()
        .map(|line| {
            let line = ANNOTATION.replace_all(line, " ");
            let line = SPACES.replace_all(&line, " ");
            SPACE_BEFORE_PUNCT.replace_all(line.trim(), "$1").into_owned()
        })
        .filter(|l| !l.is_empty())
        .collect();
    let text = lines.join(" ");
    if HALLUCINATIONS.iter().any(|h| h.is_match(&text)) {
        String::new()
    } else {
        text
    }
}

/// The text exactly as it is inserted.
pub fn compose(cleaned: &str, trailing_space: bool) -> String {
    if trailing_space && !cleaned.is_empty() {
        format!("{cleaned} ")
    } else {
        cleaned.to_string()
    }
}

/// Word count for the log; the text itself only goes there when the user allows it.
pub fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_whisper_output() {
        assert_eq!(clean(" Hallo  Welt .\n"), "Hallo Welt.");
        assert_eq!(clean("[Musik] Ja, genau. (Lachen)"), "Ja, genau.");
        assert_eq!(clean(" [BLANK_AUDIO]\n"), "");
        assert_eq!(clean("Erste Zeile.\n Zweite Zeile."), "Erste Zeile. Zweite Zeile.");
        assert_eq!(clean("♪♪ *Applaus*"), "");
    }

    #[test]
    fn drops_subtitle_hallucinations() {
        assert_eq!(clean("Untertitel der Amara.org-Community"), "");
        assert_eq!(clean(" Vielen Dank fürs Zuschauen!"), "");
        assert_eq!(clean("Thanks for watching."), "");
        // A real sentence that merely contains the words stays.
        assert_eq!(clean("Ich sage danke fürs Zuschauen, Leute, und jetzt zum Thema."), "Ich sage danke fürs Zuschauen, Leute, und jetzt zum Thema.");
    }

    #[test]
    fn compose_and_count() {
        assert_eq!(compose("Hallo.", true), "Hallo. ");
        assert_eq!(compose("Hallo.", false), "Hallo.");
        assert_eq!(compose("", true), "");
        assert_eq!(words("Das ist  ein Test."), 4);
    }
}
