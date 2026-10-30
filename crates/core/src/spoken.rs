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
    "toter", "toten", "neuer", "neuen", "meinem", "deinem", "seinem", "ihrem", "unserem", "eurem", "dieser", "dieses", "jeder",
    "jedem", "anderer", "anderen", "weiterer", "weiteren", "interessanter", "interessanten", "zentraler", "zentralen",
    "wesentlicher", "wesentlichen", "einzelnen", "gemeinsamen", "einer", "meine", "deine", "seine", "ihre", "unsere", "eure",
    "keine", "meiner", "deiner", "seiner", "ihrer", "unserer", "eurer", "keiner", "diese", "jede", "jene", "jener", "welche",
    "welcher", "a", "an", "the", "this", "that", "your", "my", "our", "their", "his", "her", "its", "every", "each", "no",
];

/// After these, a mark's name is the thing itself: "mit Komma", "ohne Punkt". Not for every
/// command - "Kommst du mit Fragezeichen" asks.
const WITH_OR_WITHOUT: &[&str] = &["mit", "ohne", "with", "without"];

/// Number words, for "drei Komma fünf".
const NUMBER_WORDS: &[&str] = &[
    "null", "eins", "ein", "zwei", "drei", "vier", "fünf", "sechs", "sieben", "acht", "neun", "zehn", "elf", "zwölf", "zero", "one",
    "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
];

const GUESSED: &[char] = &[',', ';', ':'];
const OPENING_QUOTES: &[char] = &['"', '„', '“', '«', '‚'];
const CLOSING_QUOTES: &[char] = &['"', '“', '”', '»', '‘'];
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
    // Opening quotes that stood on a command word belong to the next word.
    let mut carry = String::new();
    let mut changed = false;
    let mut i = 0;
    while i < tokens.len() {
        if let Some((len, mark)) = match_command(&cores, i) {
            if fires(&tokens, &cores, i, len, mark) {
                changed = true;
                carry.extend(tokens[i].chars().take_while(|c| OPENING_QUOTES.contains(c)));
                let closing: String = tokens[i + len - 1].chars().rev().take_while(|c| CLOSING_QUOTES.contains(c)).collect();
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
                // Closing quotes on a command word close what came before it.
                if let Some(last) = out.last_mut() {
                    let breaks = last.len() - last.trim_end_matches('\n').len();
                    let at = last.len() - breaks;
                    last.insert_str(at, &closing.chars().rev().collect::<String>());
                }
                i += len;
                continue;
            }
        }
        let mut word = format!("{}{}", std::mem::take(&mut carry), tokens[i]);
        if capitalize_next {
            // Past any opening quote, the first letter gets the capital.
            let lead: String = word.chars().take_while(|c| OPENING_QUOTES.contains(c)).collect();
            let mut chars = word[lead.len()..].chars();
            if let Some(first) = chars.next() {
                word = format!("{lead}{}{}", first.to_uppercase(), chars.as_str());
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

/// Whether the command at [i] is meant as one. After an article, a possessive, a preposition
/// or a typical adjective the word is the noun ("ein Fragezeichen", "einen neuen Absatz").
fn fires(tokens: &[&str], cores: &[String], i: usize, len: usize, mark: Mark) -> bool {
    let before = i.checked_sub(1).map(|b| cores[b].as_str());
    let after = cores.get(i + len).map(String::as_str);
    if before.is_some_and(|b| NOUN_BEFORE.contains(&b)) {
        return false;
    }
    match (mark, before) {
        // A line break as the first thing said starts a line.
        (Mark::Break(_), _) => true,
        (Mark::Attach(_), None) => false,
        (Mark::Attach(_), Some(before)) => match cores[i].as_str() {
            "punkt" => {
                // "Der 1. Punkt", "Punkt 3": a numbered point.
                if is_number(before.trim_end_matches('.')) || after.is_some_and(is_number) || WITH_OR_WITHOUT.contains(&before) {
                    return false;
                }
                at_a_sentence_end(tokens, cores, i, len)
            }
            // "colon cancer": English "colon" is a word first, like "Punkt".
            "colon" => at_a_sentence_end(tokens, cores, i, len),
            "komma" | "comma" => !WITH_OR_WITHOUT.contains(&before) && !(is_number(before) && after.is_some_and(is_number)),
            _ => true,
        },
    }
}

/// The command at the end of the dictation, before a spoken line break, or where Whisper
/// put a sentence end on the spoken word itself ("weiter Punkt. Morgen") - not merely
/// before it ("morgen. Punkt neun Uhr").
fn at_a_sentence_end(tokens: &[&str], cores: &[String], i: usize, len: usize) -> bool {
    let at_end = i + len >= cores.len();
    let before_break = match_command(cores, i + len).is_some_and(|(_, m)| matches!(m, Mark::Break(_)));
    at_end || before_break || tokens[i + len - 1].ends_with(SENTENCE_END)
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
        assert_eq!(apply("Danke Punkt neue Zeile Gruß Anna"), "Danke.\nGruß Anna");
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
    fn nouns_after_articles_stay_and_numbered_points_too() {
        for text in [
            "Wir starten morgen. Punkt neun Uhr geht es los.",
            "Der 1. Punkt ist das Geld.",
            "Das ist ein interessanter Punkt.",
            "Noch ein weiteren Punkt.",
            "Setz da ein Fragezeichen hin.",
            "Was bedeutet das Fragezeichen?",
            "Ich habe einen neuen Absatz eingefügt.",
            "Die neue Zeile ist leer.",
        ] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn command_words_after_possessives_and_prepositions_stay() {
        for text in [
            "Beginne jede neue Zeile mit einem Strich.",
            "Bitte keine neue Zeile einfügen",
            "Das ist meine neue Zeile",
            "Das schreibt man ohne Punkt.",
            "Das schreibt man mit Komma",
            "Their new line of shoes",
            "He had colon cancer surgery.",
            "Write it without comma",
        ] {
            assert_eq!(apply(text), text);
        }
        // Where they still mean the command.
        assert_eq!(apply("Kommst du mit Fragezeichen"), "Kommst du mit?");
        assert_eq!(apply("Notes colon"), "Notes:");
    }

    #[test]
    fn quotes_on_a_command_stay_with_the_text() {
        assert_eq!(apply("„Neue Zeile hallo“"), "\n„Hallo“");
        assert_eq!(apply("Er sagte „hallo neue Zeile“ und ging"), "Er sagte „hallo“\nUnd ging");
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
