//! The commas German requires and speech recognition often leaves out: before a subordinate
//! clause ("..., dass", "..., weil", "..., wenn"), before an infinitive group ("..., um ...
//! zu"), before "sondern" and a clause-joining "aber". Ported from Amber for Android
//! (`GermanCommas`), tests included.
//!
//! Rules, not a model: no download, no wait, and nothing it can invent. Each rule fires only
//! where the comma is nearly certain - checked on Android against 4,200 sentences of written
//! German (FLEURS, commas removed and restored): 97 % of the commas it sets were in the
//! original, most of the rest were commas the original itself had missed. Commas that need
//! grammar ("Wenn du kommst, bring Kuchen mit") are left alone.

use std::sync::LazyLock;

use regex::Regex;

/// Conjunctions that always open a subordinate clause.
const SUBORDINATORS: &[&str] = &[
    "dass", "weil", "obwohl", "obgleich", "obschon", "falls", "sofern", "sodass", "nachdem", "sobald", "solange", "indem", "wobei",
    "wodurch", "weshalb", "weswegen", "wohingegen", "wenngleich", "ob", "wenn", "bevor",
];

/// Words that belong to the conjunction after them, so the comma goes before them ("..., auch wenn").
const LEADS: &[&str] = &[
    "so", "ohne", "anstatt", "statt", "außer", "als", "bis", "nur", "kaum", "auch", "selbst", "erst", "gerade", "kurz", "lange", "wie",
    "je", "besonders", "insbesondere", "aber", "doch", "sondern",
];

/// After these a comma is wrong: "und dass", "oder weil" - and "aber wenn" takes it before "aber".
const JOINERS: &[&str] = &["und", "oder", "sowie", "bzw", "beziehungsweise", "weder", "noch", "entweder", "aber", "doch", "denn", "sondern"];

/// "rund um die Stadt", "gegen um acht": "um" as a preposition, whatever follows.
const BEFORE_PREPOSITIONAL_UM: &[&str] = &["rund", "etwa", "ungefähr", "gegen", "genau", "bis"];

const PRONOUNS: &[&str] = &["ich", "du", "er", "sie", "es", "wir", "ihr", "man", "mir", "mich", "dir", "dich", "uns", "euch", "ihm", "ihn", "ihnen", "sich"];

const ARTICLES: &[&str] = &[
    "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer", "eines", "kein", "keine", "mein", "meine", "dein",
    "deine", "sein", "seine", "unser", "unsere", "euer", "eure", "dieser", "diese", "dieses", "jeder", "jede", "alle",
];

/// Besides pronouns and articles: "aber" joins two clauses when one of these follows it;
/// otherwise it is a particle ("ist aber schön").
const CLAUSE_AFTER_ABER_EXTRA: &[&str] = &["nicht", "auch", "dann", "jetzt", "heute", "noch"];

/// Besides pronouns: before these, "aber" is a particle or inside a relative clause.
const NO_CLAUSE_BEFORE_ABER_EXTRA: &[&str] = &["was", "wer", "der", "die", "das", "diese", "dieser", "dieses"];

/// Already punctuated on this side: no comma added.
const CLOSING: &str = ",.;:!?–-)\"“»";
const OPENING: &str = "(\"„«»“'‚";
const SENTENCE_END: &str = ".!?";

static INFINITIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{Ll}+zu\p{Ll}+en$").unwrap());
static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:\d.*|uhr)$").unwrap());

/// Small words frequent in German and rare in English, to tell the language of a transcript.
const GERMAN_MARKERS: &[&str] = &[
    "und", "ich", "nicht", "ist", "das", "die", "der", "es", "wir", "mit", "auf", "für", "dass", "ein", "eine", "zu", "den", "auch", "sich",
    "von", "bin", "du", "haben", "wie", "noch", "aber", "wenn", "schon", "ja", "nein", "bitte", "danke", "mal", "heute", "morgen", "kann",
    "muss", "habe", "hab", "sind", "war",
];

/// And the English ones, so a sentence is weighed rather than just counted.
const ENGLISH_MARKERS: &[&str] = &[
    "the", "and", "i", "you", "is", "to", "of", "it", "that", "we", "this", "what", "are", "was", "have", "with", "for", "not", "be", "do",
    "my", "your", "he", "she", "they", "should", "will", "can",
];

fn is_in(set: &[&str], word: &str) -> bool {
    set.contains(&word)
}

/// The bare word: no quotes, brackets or punctuation around it.
fn core(token: &str) -> &str {
    token.trim_matches(|c: char| "\"„“«»‚‘'(),.;:!?".contains(c))
}

fn last_char(s: &str) -> char {
    s.chars().last().unwrap_or(' ')
}

fn first_char(s: &str) -> char {
    s.chars().next().unwrap_or(' ')
}

/// Whether [text] is German: by the language the engine was set to ([language], e.g. "de",
/// "de-DE", "german", "de,en"), and when that does not settle it, by its words.
pub fn is_german(language: Option<&str>, text: &str) -> bool {
    let tags: Vec<String> =
        language.map(|l| l.to_lowercase().split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect()).unwrap_or_default();
    let german_tag = |t: &String| t.starts_with("de") || t == "german";
    if tags.len() == 1 {
        return german_tag(&tags[0]);
    }
    if !tags.is_empty() && !tags.iter().any(german_tag) {
        return false;
    }
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split(' ').map(core).filter(|w| !w.is_empty()).collect();
    if words.len() < 2 {
        return false;
    }
    let german = words.iter().filter(|w| is_in(GERMAN_MARKERS, w) || w.chars().any(|c| "äöüß".contains(c))).count();
    let english = words.iter().filter(|w| is_in(ENGLISH_MARKERS, w)).count();
    german >= 1 && german > english
}

/// [text] with single spaces between words, commas added where they are nearly certain.
pub fn apply(text: &str) -> String {
    let tokens: Vec<&str> = text.split(' ').collect();
    if tokens.len() < 3 || tokens.iter().any(|t| t.is_empty()) {
        return text.to_string();
    }
    let words: Vec<&str> = tokens.iter().map(|t| core(t)).collect();
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let last = tokens.len() - 1;
    let mut before = vec![false; tokens.len()];

    let sentence_start = |k: usize| k == 0 || SENTENCE_END.contains(last_char(tokens[k - 1]));
    let sentence_end = |k: usize| k == last || SENTENCE_END.contains(last_char(tokens[k]));
    let clause_after_aber = |w: &str| is_in(PRONOUNS, w) || is_in(ARTICLES, w) || is_in(CLAUSE_AFTER_ABER_EXTRA, w);
    let no_clause_before_aber = |w: &str| is_in(PRONOUNS, w) || is_in(NO_CLAUSE_BEFORE_ABER_EXTRA, w);

    for i in 1..tokens.len() {
        let word = lower[i].as_str();
        // Capitalised mid-sentence: a name ("Victoria Falls").
        if word.is_empty() || words[i] != word {
            continue;
        }
        if CLOSING.contains(last_char(tokens[i - 1])) || OPENING.contains(first_char(tokens[i])) {
            continue;
        }
        // Nothing follows to make a clause.
        if sentence_end(i) {
            continue;
        }
        let next = lower[i + 1].as_str();

        let leadable = if is_in(SUBORDINATORS, word) {
            !(word == "nachdem" && lower[i - 1] == "je")
        } else if word == "während" {
            is_in(PRONOUNS, next)
        } else if word == "sondern" {
            true
        } else if word == "aber" {
            !no_clause_before_aber(&lower[i - 1]) && clause_after_aber(next)
        } else if (word == "um" && !is_in(BEFORE_PREPOSITIONAL_UM, &lower[i - 1]))
            // "ohne Zucker", "ohne einen Passagier": a capitalised noun or an article after it
            // makes "ohne" a preposition.
            || ((word == "ohne" || word == "anstatt") && !is_in(ARTICLES, next) && words[i + 1] == next)
        {
            !TIME.is_match(next) && opens_infinitive(i, &tokens, &words)
        } else {
            continue;
        };
        if !leadable {
            continue;
        }

        let mut at = i;
        let leads = is_in(SUBORDINATORS, word) || word == "um" || word == "ohne" || word == "anstatt";
        if leads && is_in(LEADS, &lower[i - 1]) {
            // "Auch wenn ..." opens the sentence.
            if sentence_start(i - 1) {
                continue;
            }
            if CLOSING.contains(last_char(tokens[i - 2])) || OPENING.contains(first_char(tokens[i - 1])) {
                continue;
            }
            at = i - 1;
        }
        if is_in(JOINERS, &lower[at - 1]) {
            continue;
        }
        before[at] = true;
    }

    tokens
        .iter()
        .enumerate()
        .map(|(k, t)| if k + 1 < tokens.len() && before[k + 1] { format!("{t},") } else { t.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "um ... zu gehen" or "um ... einzukaufen" within the same sentence, a few words on.
fn opens_infinitive(i: usize, tokens: &[&str], words: &[&str]) -> bool {
    let end = (i + 8).min(tokens.len() - 1);
    for k in i + 1..=end {
        let last = k == tokens.len() - 1 || SENTENCE_END.contains(last_char(tokens[k]));
        if words[k] == "zu" && !last {
            return true;
        }
        // Lower case only: nouns are capitalised, so "Bezugsquellen" is no infinitive.
        if INFINITIVE.is_match(words[k]) {
            return true;
        }
        if last {
            return false;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subordinate_clause_gets_its_comma() {
        assert_eq!(apply("Ich glaube dass er morgen kommt."), "Ich glaube, dass er morgen kommt.");
        assert_eq!(apply("Wir bleiben drinnen weil es regnet."), "Wir bleiben drinnen, weil es regnet.");
        assert_eq!(apply("Sag Bescheid wenn du da bist."), "Sag Bescheid, wenn du da bist.");
        assert_eq!(apply("Ich weiß nicht ob das klappt."), "Ich weiß nicht, ob das klappt.");
        assert_eq!(apply("Er kam obwohl er krank war."), "Er kam, obwohl er krank war.");
        assert_eq!(apply("Ruf an bevor du losfährst."), "Ruf an, bevor du losfährst.");
    }

    #[test]
    fn the_words_that_belong_to_the_conjunction_go_behind_the_comma() {
        assert_eq!(apply("Ich komme auch wenn es spät wird."), "Ich komme, auch wenn es spät wird.");
        assert_eq!(apply("Er ging ohne dass jemand es merkte."), "Er ging, ohne dass jemand es merkte.");
        assert_eq!(apply("Es regnete so dass wir blieben."), "Es regnete, so dass wir blieben.");
        assert_eq!(apply("Ich mache das nur um dich zu ärgern."), "Ich mache das, nur um dich zu ärgern.");
        assert_eq!(apply("Pass auf besonders wenn es glatt ist."), "Pass auf, besonders wenn es glatt ist.");
        // "aber wenn": the comma goes before "aber", the one that joins.
        assert_eq!(
            apply("Man kann es trinken aber wenn du krank bist, lieber nicht."),
            "Man kann es trinken, aber wenn du krank bist, lieber nicht."
        );
    }

    #[test]
    fn an_infinitive_group_with_um_ohne_or_anstatt_gets_its_comma() {
        assert_eq!(apply("Ich gehe raus um einzukaufen."), "Ich gehe raus, um einzukaufen.");
        assert_eq!(apply("Sie kam um uns zu helfen."), "Sie kam, um uns zu helfen.");
        assert_eq!(apply("Er ging ohne zu fragen."), "Er ging, ohne zu fragen.");
    }

    #[test]
    fn um_and_ohne_as_prepositions_stay_as_they_are() {
        for text in [
            "Wir treffen uns um 10 Uhr am Bahnhof.",
            "Wir treffen uns um acht Uhr.",
            "Der Laden liegt gleich um die Ecke.",
            "Gepäck ohne einen Passagier zu versenden ist verboten.",
            "Ich trinke Kaffee ohne Zucker.",
            "Rund um die alte Stadt gibt es viel zu sehen.",
            "Er machte Werbung ohne Genehmigung anzufragen.",
        ] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn sondern_a_joining_aber_and_waehrend_with_a_subject() {
        assert_eq!(apply("Nicht heute sondern morgen."), "Nicht heute, sondern morgen.");
        assert_eq!(apply("Ich wollte kommen aber ich war krank."), "Ich wollte kommen, aber ich war krank.");
        assert_eq!(apply("Er las während sie kochte."), "Er las, während sie kochte.");
    }

    #[test]
    fn particles_and_prepositions_that_only_look_like_conjunctions_are_left_alone() {
        for text in [
            "Das ist aber schön.",
            "Nach und nach setzte sich aber die Erkenntnis durch.",
            "Während der Fahrt schlief sie.",
            "Er schlief während der Fahrt.",
            "Je nachdem wie das Wetter wird.",
            "Wir fahren zu den Victoria Falls im Sommer.",
        ] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn existing_punctuation_sentence_starts_and_joiners_are_respected() {
        for text in [
            "Ich glaube, dass er kommt.",
            "Wenn du kommst bring Kuchen mit.",
            "Auch wenn es regnet gehen wir.",
            "Er sagt, dass es regnet und dass es kalt ist.",
            "Er sagt: \"Obwohl es regnet, gehen wir.\"",
            "Das war es. Weil es regnet, bleiben wir.",
            "Sie fragt (falls du kommst) nach dir.",
            "Es hat ein Gehirn (selbst wenn es klein ist) wie wir.",
        ] {
            assert_eq!(apply(text), text);
        }
    }

    #[test]
    fn several_clauses_in_one_sentence_each_get_theirs() {
        assert_eq!(apply("Ich hoffe dass du kommst weil ich dich vermisse."), "Ich hoffe, dass du kommst, weil ich dich vermisse.");
    }

    #[test]
    fn a_conjunction_at_the_very_end_is_no_clause_yet() {
        assert_eq!(apply("Ich weiß nicht ob"), "Ich weiß nicht ob");
    }

    #[test]
    fn german_is_recognised_by_the_language_or_by_its_words() {
        let text = "Ich glaube dass er kommt";
        assert!(is_german(Some("de"), text));
        assert!(is_german(Some("de-DE"), text));
        assert!(is_german(Some("german"), text));
        assert!(!is_german(Some("en"), text), "a set language wins over the words");
        assert!(is_german(Some("de,en"), text));
        assert!(!is_german(Some("de,en"), "I think that he is coming tomorrow"));
        assert!(is_german(None, text));
        assert!(!is_german(None, "I think that he is coming tomorrow"));
        assert!(!is_german(None, "Okay"));
    }
}
