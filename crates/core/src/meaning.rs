//! What a cleanup must not change about a dictation, told without a model: its language, its
//! negations ("nicht", "kein", "never") and its numbers ("um drei", "zweihundertvier", "19,99").
//! The cleanup's guards compare the dictation with the reply on these; a spoken
//! self-correction ("um drei, nee, um vier") is the one reason to drop something.

/// Little words only one of the two languages uses - not "in", "am", "an", "so", "was",
/// "will", "her", which both do.
const GERMAN: &[&str] = &[
    "ich", "du", "er", "wir", "ihr", "der", "die", "das", "den", "dem", "des", "und", "oder", "aber", "nicht", "kein", "keine", "keinen", "ist",
    "bin", "sind", "hab", "habe", "hast", "ein", "eine", "einen", "mit", "für", "auf", "von", "zu", "zum", "zur", "bei", "nach", "noch", "auch",
    "schon", "mal", "dann", "wenn", "weil", "dass", "ob", "wie", "wo", "bitte", "danke", "ja", "nein", "gleich", "heute", "morgen", "gestern",
    "bis", "kann", "kannst", "muss", "mir", "dich", "dir", "mich", "uns", "sie", "es", "sich", "doch", "nur", "sehr", "hier", "jetzt",
];
const ENGLISH: &[&str] = &[
    "i", "you", "he", "we", "they", "the", "a", "and", "or", "but", "not", "no", "is", "are", "was", "were", "be", "have", "has", "had", "do",
    "does", "don", "didn", "can", "would", "to", "of", "for", "on", "with", "at", "from", "this", "that", "it", "my", "your", "me", "him",
    "them", "what", "where", "when", "why", "how", "please", "thanks", "thank", "yes", "there", "here", "just", "also", "then", "if",
];

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase)
}

/// "de" or "en" when [text]'s little words say so clearly - two of them at least, three
/// times as many as of the other language; None for a short phrase or a mix.
pub fn guess_language(text: &str) -> Option<&'static str> {
    let (mut de, mut en) = (0, 0);
    for w in words(text) {
        de += GERMAN.contains(&w.as_str()) as usize;
        en += ENGLISH.contains(&w.as_str()) as usize;
    }
    if de >= 2 && de >= 3 * en {
        Some("de")
    } else if en >= 2 && en >= 3 * de {
        Some("en")
    } else {
        None
    }
}

const NEGATIONS: &[&str] = &[
    "nicht", "nichts", "kein", "keine", "keinen", "keinem", "keiner", "keines", "nie", "niemals", "niemand", "nirgends", "ohne", "weder",
    "nein", "not", "never", "nothing", "nobody", "none", "without", "cannot", "no",
];

/// [text]'s words with apostrophes kept ("don't"), each with whether it opens a sentence.
fn tokens(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let (mut word, mut opens, mut next_opens) = (String::new(), true, true);
    for c in text.chars().chain(std::iter::once(' ')) {
        if c.is_alphanumeric() || (c == '\'' || c == '’') && !word.is_empty() {
            if word.is_empty() {
                opens = next_opens;
            }
            word.push(if c == '’' { '\'' } else { c });
            continue;
        }
        if !word.is_empty() {
            out.push((word.to_lowercase(), opens));
            word.clear();
            next_opens = false;
        }
        if matches!(c, '.' | '!' | '?' | '\n') {
            next_opens = true;
        }
    }
    out
}

/// How many times [text] denies something: "nicht", "kein", "ohne", "never", "don't". A
/// "Nein" or "No" opening a sentence answers; it denies nothing in it.
pub fn negations(text: &str) -> usize {
    tokens(text)
        .iter()
        .filter(|(w, opens)| w.ends_with("n't") || NEGATIONS.contains(&w.as_str()) && !(*opens && (w == "nein" || w == "no")))
        .count()
}

const DE_UNITS: &[(&str, u64)] = &[
    ("null", 0), ("eins", 1), ("ein", 1), ("zwei", 2), ("zwo", 2), ("drei", 3), ("vier", 4), ("fünf", 5), ("sechs", 6), ("sieben", 7),
    ("acht", 8), ("neun", 9), ("zehn", 10), ("elf", 11), ("zwölf", 12), ("dreizehn", 13), ("vierzehn", 14), ("fünfzehn", 15),
    ("sechzehn", 16), ("siebzehn", 17), ("achtzehn", 18), ("neunzehn", 19), ("zwanzig", 20), ("dreißig", 30), ("dreissig", 30),
    ("vierzig", 40), ("fünfzig", 50), ("sechzig", 60), ("siebzig", 70), ("achtzig", 80), ("neunzig", 90),
];
const EN_UNITS: &[(&str, u64)] = &[
    ("zero", 0), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("seven", 7), ("eight", 8), ("nine", 9), ("ten", 10),
    ("eleven", 11), ("twelve", 12), ("thirteen", 13), ("fourteen", 14), ("fifteen", 15), ("sixteen", 16), ("seventeen", 17),
    ("eighteen", 18), ("nineteen", 19), ("twenty", 20), ("thirty", 30), ("forty", 40), ("fifty", 50), ("sixty", 60), ("seventy", 70),
    ("eighty", 80), ("ninety", 90),
];

fn lookup(table: &[(&str, u64)], w: &str) -> Option<u64> {
    table.iter().find(|(name, _)| *name == w).map(|(_, v)| *v)
}

/// A German number word, compounds included: "zweihundertvier", "neunundneunzig".
fn german_cardinal(w: &str) -> Option<u64> {
    if let Some((left, right)) = w.split_once("tausend") {
        let l = if left.is_empty() { 1 } else { german_cardinal(left)? };
        let r = if right.is_empty() { 0 } else { german_cardinal(right)? };
        return Some(l * 1000 + r);
    }
    if let Some((left, right)) = w.split_once("hundert") {
        let l = if left.is_empty() { 1 } else { german_cardinal(left)? };
        let r = if right.is_empty() { 0 } else { german_cardinal(right)? };
        return Some(l * 100 + r);
    }
    if let Some((unit, tens)) = w.split_once("und") {
        let (u, t) = (lookup(DE_UNITS, unit)?, lookup(DE_UNITS, tens)?);
        return (u < 10 && t >= 20 && t % 10 == 0).then_some(t + u);
    }
    lookup(DE_UNITS, w)
}

/// A German number word as a value - "ein" is an article, "erst" means "only" and "erstens"
/// counts a list, so none of them is one; an ordinal ("vierzehnten") is.
fn german_value(w: &str) -> Option<u64> {
    if w == "ein" || w.ends_with("tens") {
        return None;
    }
    if let Some(v) = german_cardinal(w) {
        return Some(v);
    }
    for suffix in ["sten", "ster", "stes", "stem", "ste", "ten", "ter", "tes", "tem", "te"] {
        if let Some(stem) = w.strip_suffix(suffix) {
            let irregular = match stem {
                "ers" => Some(1),
                "drit" => Some(3),
                "sieb" => Some(7),
                _ => None,
            };
            if let Some(v) = irregular.or_else(|| german_cardinal(stem).filter(|_| stem != "ein")) {
                return Some(v);
            }
        }
    }
    None
}

/// An English number word as a value; "one" is left out ("the one", "no one").
fn english_value(w: &str) -> Option<u64> {
    let irregular = match w {
        "first" => Some(1),
        "third" => Some(3),
        "fifth" => Some(5),
        "eighth" => Some(8),
        "ninth" => Some(9),
        "twelfth" => Some(12),
        "hundred" => Some(100),
        "thousand" => Some(1000),
        _ => None,
    };
    irregular.or_else(|| lookup(EN_UNITS, w)).or_else(|| w.strip_suffix("th").and_then(|s| lookup(EN_UNITS, s)))
}

/// One number in a text: its value, and whether it was a single digit word ("null",
/// "sieben") - those, said one after another, may be one number written ("0176").
#[derive(Clone, Debug)]
struct Number {
    value: String,
    digit_word: bool,
}

fn number_tokens(text: &str) -> Vec<Number> {
    let mut out: Vec<Number> = Vec::new();
    // English "twenty five", "two hundred": merged with the number before.
    let mut last_english: Option<u64> = None;
    for w in text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()) {
        if w.chars().all(|c| c.is_ascii_digit()) {
            out.push(Number { value: w.to_string(), digit_word: false });
            last_english = None;
            continue;
        }
        let lower = w.to_lowercase();
        if let Some(v) = english_value(&lower) {
            match (last_english, v) {
                (Some(prev), 100 | 1000) => {
                    out.pop();
                    out.push(Number { value: (prev * v).to_string(), digit_word: false });
                    last_english = Some(prev * v);
                }
                (Some(prev), 1..=9) if prev >= 20 && prev % 10 == 0 || prev >= 100 && prev % 100 == 0 => {
                    out.pop();
                    out.push(Number { value: (prev + v).to_string(), digit_word: false });
                    last_english = None;
                }
                _ => {
                    out.push(Number { value: v.to_string(), digit_word: v < 10 });
                    last_english = Some(v);
                }
            }
            continue;
        }
        last_english = None;
        if let Some(v) = german_value(&lower) {
            out.push(Number { value: v.to_string(), digit_word: v < 10 });
        }
    }
    out
}

/// The numbers in [text], as values: "vierzehnten" and "14" are both "14".
pub fn numbers(text: &str) -> Vec<String> {
    number_tokens(text).into_iter().map(|n| n.value).collect()
}

fn same_value(a: &str, b: &str) -> bool {
    a == b || matches!((a.parse::<u64>(), b.parse::<u64>()), (Ok(x), Ok(y)) if x == y)
}

/// How [reply]'s numbers differ from [dictation]'s - in digits or in words, a phone number's
/// digits said one by one written together: (one added or changed, one dropped).
pub fn number_changes(dictation: &str, reply: &str) -> (bool, bool) {
    let said = number_tokens(dictation);
    let written = numbers(reply);
    // Runs of single digits said one after another ("null eins sieben sechs"), as written.
    let mut runs: Vec<(usize, usize, String)> = Vec::new();
    for start in 0..said.len() {
        let mut joined = String::new();
        for (end, n) in said.iter().enumerate().skip(start) {
            if !n.digit_word {
                break;
            }
            joined.push_str(&n.value);
            if end > start {
                runs.push((start, end, joined.clone()));
            }
        }
    }
    let mut covered = vec![false; said.len()];
    let mut added = false;
    for w in &written {
        if let Some(i) = said.iter().position(|n| same_value(&n.value, w)) {
            covered[i] = true;
            continue;
        }
        match runs.iter().find(|(_, _, joined)| joined == w) {
            Some((start, end, _)) => covered[*start..=*end].iter_mut().for_each(|c| *c = true),
            None => added = true,
        }
    }
    // A number said and not written: dropped - unless another occurrence covers it.
    let dropped = !said.iter().enumerate().all(|(i, n)| covered[i] || said.iter().enumerate().any(|(j, m)| covered[j] && same_value(&m.value, &n.value)));
    (added, dropped)
}

/// Whether [reply] says the numbers [dictation] said, no more, none fewer.
pub fn same_numbers(dictation: &str, reply: &str) -> bool {
    number_changes(dictation, reply) == (false, false)
}

/// Whether the speaker takes something back in [text]: "nee", "nein, warte", "stopp",
/// "ich meine", "besser gesagt", "no wait", "I mean", "scratch that", "sorry". Then
/// something said may rightly be missing from the cleanup; it relaxes the checks for what
/// was dropped, never for what was added.
pub fn corrects_itself(text: &str) -> bool {
    let t = text.to_lowercase();
    let marker = regex::Regex::new(
        r"\b(nee|nö|stopp?|korrektur|ich meine|besser gesagt|sorry|no wait|i mean|scratch that|or rather|actually)\b|\bnein\b[\s,.]*(warte|stopp?|sorry|doch)\b|[^.!?\s][^.!?]*\bnein\s*,",
    )
    .expect("static regex");
    marker.is_match(&t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_language_is_told_by_its_little_words() {
        assert_eq!(guess_language("Ich hab kein Wasser."), Some("de"));
        assert_eq!(guess_language("I have no water."), Some("en"));
        assert_eq!(guess_language("Ich hab das Deployment gestern gefixt, aber der Build ist noch broken."), Some("de"));
        assert_eq!(guess_language("Can you send me the file before the meeting tomorrow?"), Some("en"));
        // Too little to tell.
        assert_eq!(guess_language("Okay, danke."), None);
        assert_eq!(guess_language("Passt."), None);
        assert_eq!(guess_language("Tinfoil"), None);
        // Half and half: not confident either way.
        assert_eq!(guess_language("Hey, kannst du mir die Datei schicken, I need it for the meeting."), None);
    }

    #[test]
    fn negations_are_counted_but_not_an_answering_no() {
        assert_eq!(negations("Ich komme heute nicht, aber morgen."), 1);
        assert_eq!(negations("ohne Zucker bitte, keine Milch"), 2);
        assert_eq!(negations("I don't think so, it never works."), 2);
        assert_eq!(negations("We can't and won't."), 2);
        // "Nein" and "No" opening a sentence answer; they deny nothing in it.
        assert_eq!(negations("Nein, ich habe keine Zeit."), 1);
        assert_eq!(negations("No. That is fine."), 0);
        assert_eq!(negations("There is no water."), 1);
        assert_eq!(negations("Das ist ein Problem."), 0);
    }

    #[test]
    fn numbers_are_values_however_they_are_written() {
        let has = |text: &str, value: &str| numbers(text).iter().any(|n| n == value);
        assert!(has("am vierzehnten um halb drei in raum zweihundertvier", "14"));
        assert!(has("am vierzehnten um halb drei in raum zweihundertvier", "3"));
        assert!(has("am vierzehnten um halb drei in raum zweihundertvier", "204"));
        assert!(has("Es kostet neunzehn Euro neunundneunzig.", "99"));
        assert!(has("twenty-five people", "25"));
        assert!(has("Raum 204", "204"));
        // "ein" is an article, "erst" means "only", "erstens" counts a list.
        assert!(numbers("Ich habe erst ein Buch gelesen, erstens weil").is_empty());
        assert!(numbers("Das ist ein Problem.").is_empty());
    }

    #[test]
    fn a_reply_keeps_the_dictations_numbers() {
        // Digits for words is a tidy-up, not a change.
        assert!(same_numbers("um sieben uhr fünfzehn", "um 7:15 Uhr"));
        assert!(same_numbers("zwei komma fünf prozent", "2,5 %"));
        assert!(same_numbers("neunzehn Euro neunundneunzig", "19,99 Euro"));
        assert!(same_numbers("Ruf mich unter null eins sieben sechs an.", "Ruf mich unter 0176 an."));
        assert!(same_numbers("am vierzehnten in raum zweihundertvier", "am 14. in Raum 204"));
        assert!(same_numbers("twenty-five people", "25 people"));
        // A number changed, added or dropped is not.
        assert!(!same_numbers("um drei", "um vier"));
        assert!(!same_numbers("in Raum zwölf", "in Raum 12 um 3"));
        assert!(!same_numbers("fünf Stühle und zwei Tische", "fünf Stühle und Tische"));
    }

    #[test]
    fn a_self_correction_is_heard() {
        for t in ["um drei nee stopp um vier", "um drei, nein, warte, um vier", "ohne Prüfung, nein stopp, mit Prüfung", "am Montag, ich meine am Dienstag",
                  "an Anna, sorry, an Maria", "im Raum zwölf, besser gesagt vierzehn", "at three, no wait, at four", "the bus, I mean the train", "scratch that"] {
            assert!(corrects_itself(t), "{t}");
        }
        for t in ["nein ich habe keine zeit", "Ich komme heute nicht.", "No, I don't think so.", "Das ist kein Problem."] {
            assert!(!corrects_itself(t), "{t}");
        }
    }
}
