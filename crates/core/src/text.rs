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

/// Hesitation sounds, German and English - only unambiguous ones. Not "um" and "eh": in
/// German they are words ("um vier Uhr", "das ist eh egal").
const FILLERS: [&str; 12] = ["äh", "ähm", "ähh", "öh", "öhm", "hm", "hmm", "mhm", "mh", "uh", "uhm", "erm"];

/// Drops hesitation sounds with the commas around them, and gives the sentence its capital
/// back when it started with one. Cannot invent anything, costs nothing, always on.
pub fn strip_fillers(text: &str) -> String {
    const QUOTES: &str = "\"'„“”‚‘’«»";
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut kept: Vec<String> = Vec::with_capacity(tokens.len());
    let mut dropped_first = false;
    // Opening quotes of a dropped filler belong to the next word.
    let mut carry = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let core = token.trim_matches(|c: char| ",.;:!?-…".contains(c) || QUOTES.contains(c)).to_lowercase();
        if !FILLERS.contains(&core.as_str()) {
            kept.push(format!("{}{token}", std::mem::take(&mut carry)));
            continue;
        }
        if i == 0 {
            dropped_first = true;
        }
        carry.extend(token.chars().take_while(|c| QUOTES.contains(*c)));
        let tail: String = token.chars().rev().take_while(|c| !c.is_alphanumeric()).collect::<Vec<_>>().into_iter().rev().collect();
        let Some(last) = kept.last_mut() else { continue };
        // "ist, ähm, gut" -> "ist gut", "„ja, ähm“" -> "„ja“": a filler that ends a clause
        // (with a comma, a full stop or a quote) took the comma before it along.
        if !tail.is_empty() && last.ends_with(',') {
            last.pop();
        }
        // A filler that ended the sentence hands its full stop to the word before it - unless
        // that word already ends one ("Gut. Hm." stays "Gut.").
        if let Some(end) = tail.chars().find(|c| ".!?".contains(*c)) {
            let trimmed = last.trim_end_matches(',').to_string();
            *last = if trimmed.ends_with(['.', '!', '?', ':', ';']) { trimmed } else { format!("{trimmed}{end}") };
        }
        // Closing quotes stay with the sentence they close.
        last.extend(tail.chars().filter(|c| QUOTES.contains(*c)));
    }
    let mut out = kept.join(" ");
    if dropped_first {
        out = out.trim_start_matches(|c: char| c == ',' || c.is_whitespace()).to_string();
        // Past an opening quote, the first letter gets its capital back.
        let lead: String = out.chars().take_while(|c| QUOTES.contains(*c)).collect();
        let mut rest = out[lead.len()..].chars();
        if let Some(first) = rest.next().filter(|c| c.is_lowercase()) {
            out = format!("{lead}{}{}", first.to_uppercase(), rest.as_str());
        }
    }
    out
}

/// Which rule steps run - see [by_rule].
#[derive(Clone, Copy, Debug)]
pub struct Rules<'a> {
    pub fillers: bool,
    pub spoken_punctuation: bool,
    pub german_commas: bool,
    /// The set language ("de"), or None for automatic.
    pub language: Option<&'a str>,
}

/// The steps that are rules - instant, local, nothing invented - in an order that keeps them
/// out of each other's way: fillers first (so "ähm Komma" keeps its comma, and no line break
/// exists yet to be flattened), then spoken punctuation, then German commas line by line.
pub fn by_rule(text: &str, rules: Rules) -> String {
    let mut out = if rules.fillers { strip_fillers(text) } else { text.to_string() };
    if rules.spoken_punctuation {
        out = crate::spoken::apply(&out);
    }
    if rules.german_commas && crate::commas::is_german(rules.language, &out) {
        out = out.split('\n').map(crate::commas::apply).collect::<Vec<_>>().join("\n");
    }
    out
}

/// The text exactly as it is inserted.
pub fn compose(cleaned: &str, trailing_space: bool) -> String {
    // After a spoken line break the next dictation starts on the new line, not after a space.
    if trailing_space && !cleaned.is_empty() && !cleaned.ends_with('\n') {
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
    fn fillers_go_words_stay() {
        assert_eq!(strip_fillers("Ähm, lass uns um drei treffen."), "Lass uns um drei treffen.");
        assert_eq!(strip_fillers("Das ist, ähm, gut."), "Das ist gut.");
        assert_eq!(strip_fillers("Das ist äh gut"), "Das ist gut");
        assert_eq!(strip_fillers("Wir sehen uns morgen, hm."), "Wir sehen uns morgen.");
        assert_eq!(strip_fillers("Hmm."), "");
        // The comma before a filler counts as the filler's; for "So, uh," that costs a real
        // one. The smart cleanup puts punctuation right afterwards.
        assert_eq!(strip_fillers("So, uh, I think we can, erm, go."), "So I think we can go.");
        // German words that look like English fillers stay.
        assert_eq!(strip_fillers("Um vier Uhr ist das eh egal."), "Um vier Uhr ist das eh egal.");
        // Only whole words.
        assert_eq!(strip_fillers("Das Ähm-Problem und die Hummel."), "Das Ähm-Problem und die Hummel.");
        // Review findings: no doubled or mixed sentence ends, quotes survive.
        assert_eq!(strip_fillers("Gut. Hm."), "Gut.");
        assert_eq!(strip_fillers("Echt? Hm."), "Echt?");
        assert_eq!(strip_fillers("Folgendes: ähm."), "Folgendes:");
        assert_eq!(strip_fillers("\"Ähm, hallo\", sagte er."), "\"Hallo\", sagte er.");
        assert_eq!(strip_fillers("Er sagte „ja, ähm“ und ging."), "Er sagte „ja“ und ging.");
    }

    #[test]
    fn the_whole_chain_keeps_line_breaks_and_spoken_commas() {
        let all = Rules { fillers: true, spoken_punctuation: true, german_commas: true, language: None };
        assert_eq!(by_rule("Hallo Anna neue Zeile wie geht es dir", all), "Hallo Anna\nWie geht es dir");
        assert_eq!(by_rule("Danke Punkt neue Zeile Gruß Fabi", all), "Danke.\nGruß Fabi");
        assert_eq!(by_rule("Das ist ähm Komma gut so", all), "Das ist, gut so");
        assert_eq!(
            by_rule("Ähm ich glaube dass er kommt neuer Absatz Bis morgen weil es spät ist", all),
            "Ich glaube, dass er kommt\n\nBis morgen, weil es spät ist"
        );
        let none = Rules { fillers: false, spoken_punctuation: false, german_commas: false, language: None };
        assert_eq!(by_rule("ähm Komma", none), "ähm Komma");
    }

    #[test]
    fn compose_and_count() {
        assert_eq!(compose("Hallo.", true), "Hallo. ");
        assert_eq!(compose("Hallo.", false), "Hallo.");
        assert_eq!(compose("", true), "");
        assert_eq!(compose("Gruß\n", true), "Gruß\n");
        assert_eq!(words("Das ist  ein Test."), 4);
    }
}
