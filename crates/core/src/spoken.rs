//! Spoken punctuation and layout, German and English: "neue Zeile", "neuer Absatz",
//! "Komma", "Fragezeichen", "new line", "question mark" ... become the marks they name.
//!
//! Whisper often writes the spoken command as a word and adds punctuation of its own around
//! it ("morgen, Komma, und"); that guessed punctuation goes, the named mark stays. "Punkt" /
//! "period" only counts at the end of a sentence - "Punkt neun Uhr" keeps its word.

/// What a command turns into.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mark {
    /// Attached to the word before it: "," "." "?" ...
    Attach(char),
    /// A line break; `true` for an empty line in between (a new paragraph).
    Break(bool),
}

/// Commands as lowercase word sequences, longest first so "neuer absatz" wins over parts.
const COMMANDS: &[(&[&str], Mark)] = &[
    (&["neuer", "absatz"], Mark::Break(true)),
    (&["neuen", "absatz"], Mark::Break(true)),
    (&["new", "paragraph"], Mark::Break(true)),
    (&["neue", "zeile"], Mark::Break(false)),
    (&["new", "line"], Mark::Break(false)),
    (&["question", "mark"], Mark::Attach('?')),
    (&["exclamation", "mark"], Mark::Attach('!')),
    (&["exclamation", "point"], Mark::Attach('!')),
    (&["full", "stop"], Mark::Attach('.')),
    (&["komma"], Mark::Attach(',')),
    (&["comma"], Mark::Attach(',')),
    (&["fragezeichen"], Mark::Attach('?')),
    (&["ausrufezeichen"], Mark::Attach('!')),
    (&["doppelpunkt"], Mark::Attach(':')),
    (&["colon"], Mark::Attach(':')),
    (&["semikolon"], Mark::Attach(';')),
    (&["strichpunkt"], Mark::Attach(';')),
    (&["semicolon"], Mark::Attach(';')),
    (&["punkt"], Mark::Attach('.')),
    (&["period"], Mark::Attach('.')),
];

/// Only at the end of a sentence: the next word starts one, or nothing follows.
fn sentence_end_only(words: &[&str]) -> bool {
    matches!(words, ["punkt"] | ["period"] | ["full", "stop"])
}

const TRIM: &[char] = &[',', '.', ';', ':', '!', '?', '…', '"', '„', '“', '”', '«', '»'];

fn core(token: &str) -> String {
    token.trim_matches(TRIM).to_lowercase()
}

/// Turns spoken commands in [text] into the marks they name. Text without commands comes back
/// unchanged.
pub fn apply(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let cores: Vec<String> = tokens.iter().map(|t| core(t)).collect();
    // The output as pieces: words, and marks/breaks glued to what is before them.
    let mut out: Vec<String> = Vec::new();
    // Whether a piece's last mark was spoken - a line break after "Punkt" keeps that full
    // stop, while one Whisper guessed is dropped.
    let mut spoken_end: Vec<bool> = Vec::new();
    // After a sentence end or a break, the next word gets a capital.
    let mut capitalize_next = false;
    let mut changed = false;
    let mut i = 0;
    while i < tokens.len() {
        if let Some((len, mark)) = match_command(&cores, i) {
            let next_word = tokens.get(i + len);
            let at_sentence_end = next_word.is_none_or(|w| w.chars().next().is_some_and(char::is_uppercase))
                || match_command(&cores, i + len).is_some_and(|(_, m)| matches!(m, Mark::Break(_)));
            let words: Vec<&str> = cores[i..i + len].iter().map(String::as_str).collect();
            if !out.is_empty() && (!sentence_end_only(&words) || at_sentence_end) {
                changed = true;
                // Whisper's own guess around the command goes: "morgen, Komma," -> "morgen,".
                let keep = *spoken_end.last().expect("checked");
                let last = out.last_mut().expect("checked");
                while !keep && last.ends_with(TRIM) && !last.ends_with('\n') {
                    last.pop();
                }
                match mark {
                    Mark::Attach(c) => {
                        if keep {
                            // A second spoken mark replaces the first ("Komma Fragezeichen").
                            last.pop();
                        }
                        last.push(c);
                        capitalize_next = matches!(c, '.' | '?' | '!');
                        *spoken_end.last_mut().expect("checked") = true;
                    }
                    Mark::Break(paragraph) => {
                        last.push_str(if paragraph { "\n\n" } else { "\n" });
                        capitalize_next = true;
                        *spoken_end.last_mut().expect("checked") = false;
                    }
                }
                i += len;
                continue;
            }
        }
        let mut word = tokens[i].to_string();
        if capitalize_next {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                word = first.to_uppercase().chain(chars).collect();
            }
            capitalize_next = false;
        }
        out.push(word);
        spoken_end.push(false);
        i += 1;
    }
    if !changed {
        return text.to_string();
    }
    let mut joined = String::new();
    for piece in &out {
        if !joined.is_empty() && !joined.ends_with('\n') {
            joined.push(' ');
        }
        joined.push_str(piece);
    }
    joined.trim_end_matches(' ').to_string()
}

fn match_command(cores: &[String], at: usize) -> Option<(usize, Mark)> {
    COMMANDS.iter().find_map(|(words, mark)| {
        let end = at + words.len();
        (end <= cores.len() && cores[at..end].iter().zip(words.iter()).all(|(c, w)| c == w)).then_some((words.len(), *mark))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_replace_their_names() {
        assert_eq!(apply("Ich komme morgen Komma aber nur kurz Punkt"), "Ich komme morgen, aber nur kurz.");
        assert_eq!(apply("Kommst du Fragezeichen"), "Kommst du?");
        assert_eq!(apply("Das ist toll Ausrufezeichen"), "Das ist toll!");
        assert_eq!(apply("Folgendes Doppelpunkt Brot und Milch"), "Folgendes: Brot und Milch");
        assert_eq!(apply("see you tomorrow comma bye period"), "see you tomorrow, bye.");
        assert_eq!(apply("are you there question mark"), "are you there?");
    }

    #[test]
    fn whispers_own_punctuation_around_a_command_goes() {
        assert_eq!(apply("Ich komme morgen, Komma, aber nur kurz."), "Ich komme morgen, aber nur kurz.");
        assert_eq!(apply("Kommst du? Fragezeichen."), "Kommst du?");
        assert_eq!(apply("Hallo. Neue Zeile. Wie geht's?"), "Hallo\nWie geht's?");
    }

    #[test]
    fn line_breaks_and_paragraphs() {
        assert_eq!(apply("Hallo Anna neue Zeile wie geht es dir"), "Hallo Anna\nWie geht es dir");
        // "Punkt" before a break is a full stop - the usual way to end a dictated paragraph.
        assert_eq!(apply("Erster Teil Punkt neuer Absatz zweiter Gedanke"), "Erster Teil.\n\nZweiter Gedanke");
        assert_eq!(apply("first line new line second line"), "first line\nSecond line");
        assert_eq!(apply("Danke Punkt neue Zeile Gruß Fabi"), "Danke.\nGruß Fabi");
    }

    #[test]
    fn punkt_only_at_a_sentence_end() {
        assert_eq!(apply("Wir treffen uns Punkt neun Uhr."), "Wir treffen uns Punkt neun Uhr.");
        assert_eq!(apply("Das war's Punkt Morgen geht es weiter Punkt"), "Das war's. Morgen geht es weiter.");
        assert_eq!(apply("Der wichtigste Punkt ist das Geld."), "Der wichtigste Punkt ist das Geld.");
        // A sentence end with a capital after it - and the capital stays.
        assert_eq!(apply("Gut Punkt Bis morgen"), "Gut. Bis morgen");
    }

    #[test]
    fn plain_text_is_untouched() {
        for text in ["Das ist ein ganz normaler Satz.", "Er hat ein Komma-Problem.", "Zeile für Zeile.", "", "Punkt"] {
            assert_eq!(apply(text), text);
        }
    }
}
