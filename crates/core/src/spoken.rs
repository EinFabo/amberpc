//! Spoken punctuation and layout, German and English: "neue Zeile", "neuer Absatz",
//! "Komma", "Fragezeichen", "new line", "question mark" ... become the marks they name.
//!
//! Careful with words that are also nouns. "Punkt" only counts at the end of the dictation or
//! before a spoken line break - or where Whisper itself ended a sentence around it - and
//! never after an article or adjective ("ein guter Punkt", "von Punkt A nach Punkt B" stay).
//! "Komma" after an article or between two numbers stays a word ("ein Komma", "drei Komma
//! fünf"). English "period" is left out: "trial period" is far more common than the command.
//!
//! Whisper often writes the command as a word with punctuation of its own around it
//! ("morgen, Komma, und"); its guessed commas go, the named mark stays. Its sentence ends and
//! quotes are kept.

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
];

/// After these, "Punkt" / "Komma" is the noun: articles, possessives, prepositions and the
/// adjectives that usually stand before "Punkt".
const NOUN_BEFORE: &[&str] = &[
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "eines", "kein", "keinen", "keinem", "mein", "meinen",
    "dein", "deinen", "sein", "seinen", "ihr", "ihren", "unser", "unseren", "euer", "jeden", "diesen", "diesem", "welchen", "ab", "auf",
    "um", "von", "nach", "zum", "bis", "im", "am", "vom", "zu", "guter", "guten", "wichtiger", "wichtigen", "wichtigste", "wichtigsten",
    "letzter", "letzten", "nächster", "nächsten", "erster", "ersten", "zweiter", "zweiten", "dritter", "dritten", "springende",
    "springenden", "kritischer", "kritischen", "schwacher", "schwachen", "starker", "starken", "entscheidende", "entscheidenden",
    "toter", "toten", "neuer", "neuen", "a", "an", "the", "this", "that",
];

/// Number words, for "drei Komma fünf".
const NUMBER_WORDS: &[&str] = &[
    "null", "eins", "ein", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun", "zehn", "elf", "zwölf", "zero", "one",
    "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

const GUESSED: &[char] = &[',', ';', ':'];
const SENTENCE_END: &[char] = &['.', '!', '?'];
const TRIM: &[char] = &[',', '.', ';', ':', '!', '?', '…', '"', '„', '“', '”', '«', '»'];

fn core(token: &str) -> String {
    token.trim_matches(TRIM).to_lowercase()
}

fn is_number(word: &str) -> bool {
    word.chars().all(|c| c.is_ascii_digit()) && !word.is_empty() || NUMBER_WORDS.contains(&word)
}

/// Turns spoken commands in [text] into the marks they name. Text without commands comes back
/// unchanged.
pub fn apply(text: &str) -> String {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let cores: Vec<String> = tokens.iter().map(|t| core(t)).collect();
    // The output as pieces: words, and marks/breaks glued to what is before them.
    let mut out: Vec<String> = Vec::new();
    // After a sentence end or a break, the next word gets a capital.
    let mut capitalize_next = false;
    let mut changed = false;
    let mut i = 0;
    while i < tokens.len() {
        if let Some((len, mark)) = match_command(&cores, i) {
            if fires(&tokens, &cores, i, len, mark) {
                changed = true;
                match (mark, out.last_mut()) {
                    (Mark::Break(paragraph), None) => {
                        // A line break as the first thing said: the dictation starts a line.
                        out.push(if paragraph { "\n\n" } else { "\n" }.to_string());
                        capitalize_next = true;
                    }
                    (Mark::Break(paragraph), Some(last)) => {
                        // Whisper's guessed comma before a break goes; its sentence end stays.
                        while last.ends_with(GUESSED) {
                            last.pop();
                        }
                        last.push_str(if paragraph { "\n\n" } else { "\n" });
                        capitalize_next = true;
                    }
                    (Mark::Attach(c), Some(last)) => {
                        // Whisper's own guess right before the named mark goes: "morgen, Komma".
                        while last.ends_with(GUESSED) || last.ends_with(SENTENCE_END) {
                            last.pop();
                        }
                        last.push(c);
                        capitalize_next = SENTENCE_END.contains(&c);
                    }
                    (Mark::Attach(_), None) => unreachable!("fires() refuses marks with nothing before them"),
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

/// Whether the command at [i] is meant as one.
fn fires(tokens: &[&str], cores: &[String], i: usize, len: usize, mark: Mark) -> bool {
    let before = i.checked_sub(1).map(|b| cores[b].as_str());
    let after = cores.get(i + len).map(String::as_str);
    match mark {
        Mark::Break(_) => true,
        Mark::Attach(_) if before.is_none() => false,
        Mark::Attach(c) => {
            let before = before.expect("checked");
            match cores[i].as_str() {
                "punkt" => {
                    if NOUN_BEFORE.contains(&before) {
                        return false;
                    }
                    let at_end = after.is_none();
                    let before_break = match_command(cores, i + len).is_some_and(|(_, m)| matches!(m, Mark::Break(_)));
                    // Mid-text only where Whisper itself ended a sentence around it.
                    let whisper_ended = tokens[i - 1].ends_with(SENTENCE_END) || tokens[i + len - 1].ends_with(SENTENCE_END);
                    at_end || before_break || whisper_ended
                }
                "komma" | "comma" => !NOUN_BEFORE.contains(&before) && !(is_number(before) && after.is_some_and(is_number)),
                _ => c != ',' || !NOUN_BEFORE.contains(&before),
            }
        }
    }
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
        assert_eq!(apply("see you tomorrow comma bye full stop"), "see you tomorrow, bye.");
        assert_eq!(apply("are you there question mark"), "are you there?");
    }

    #[test]
    fn whispers_guesses_go_its_sentence_ends_and_quotes_stay() {
        assert_eq!(apply("Ich komme morgen, Komma, aber nur kurz."), "Ich komme morgen, aber nur kurz.");
        assert_eq!(apply("Kommst du? Fragezeichen."), "Kommst du?");
        assert_eq!(apply("Hallo. Neue Zeile. Wie geht's?"), "Hallo.\nWie geht's?");
        assert_eq!(apply("Wie geht's? Neue Zeile."), "Wie geht's?\n");
        assert_eq!(apply("Er sagte „Hallo“, Komma, und ging."), "Er sagte „Hallo“, und ging.");
    }

    #[test]
    fn line_breaks_and_paragraphs() {
        assert_eq!(apply("Hallo Anna neue Zeile wie geht es dir"), "Hallo Anna\nWie geht es dir");
        assert_eq!(apply("Erster Teil Punkt neuer Absatz zweiter Gedanke"), "Erster Teil.\n\nZweiter Gedanke");
        assert_eq!(apply("first line new line second line"), "first line\nSecond line");
        assert_eq!(apply("Danke Punkt neue Zeile Gruß Fabi"), "Danke.\nGruß Fabi");
        // Dictating sentence by sentence: a dictation that is only the break.
        assert_eq!(apply("Neue Zeile."), "\n");
        assert_eq!(apply("Neuer Absatz Liebe Grüße"), "\n\nLiebe Grüße");
    }

    #[test]
    fn punkt_the_noun_stays() {
        for text in [
            "Wir treffen uns Punkt neun Uhr.",
            "Das ist ein guter Punkt.",
            "Von Punkt A nach Punkt B.",
            "Punkt Mitternacht geht es los.",
            "Der wichtigste Punkt ist das Geld.",
            "Es geht um einen Punkt",
            "for a long period.",
            "During this period I learned a lot.",
        ] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn punkt_the_command() {
        assert_eq!(apply("Das ist toll Punkt"), "Das ist toll.");
        // Whisper ended the sentence itself around the spoken word.
        assert_eq!(apply("Das war's. Punkt. Morgen geht es weiter."), "Das war's. Morgen geht es weiter.");
    }

    #[test]
    fn komma_the_noun_and_the_decimal_stay() {
        for text in ["Da fehlt ein Komma.", "Das kostet drei Komma fünf Euro.", "It costs 3 comma 5 dollars."] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn plain_text_is_untouched() {
        for text in ["Das ist ein ganz normaler Satz.", "Er hat ein Komma-Problem.", "Zeile für Zeile.", "", "Punkt", "Komma"] {
            assert_eq!(apply(text), text);
        }
    }
}
