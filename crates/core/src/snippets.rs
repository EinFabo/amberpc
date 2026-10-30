//! Snippets: a short spoken phrase ("meine Adresse", "Signatur") inserts a saved text
//! instead of the dictation. Only a dictation that is nothing but the phrase counts - a
//! sentence that merely contains it stays as spoken. Whisper's capitals and punctuation do
//! not matter, and "Snippet ..." / "Baustein ..." may lead.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Snippet {
    /// What to say.
    pub trigger: String,
    /// What gets inserted, line breaks included.
    pub text: String,
}

/// Words that may lead a trigger without changing it.
const LEADS: &[&str] = &["snippet", "baustein", "textbaustein"];

/// Lowercase words only: "Meine Adresse." -> "meine adresse".
fn normalize(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect::<Vec<_>>().join(" ")
}

/// Lowercase words without a leading "Snippet" / "Baustein" - for the dictation and the
/// trigger alike, so a trigger saved as "Snippet Signatur" matches too.
fn key(text: &str) -> String {
    let words = normalize(text);
    LEADS.iter().find_map(|lead| words.strip_prefix(lead).and_then(|r| r.strip_prefix(' ')).map(str::to_string)).unwrap_or(words)
}

/// The snippet the whole dictation names, if any.
pub fn find<'a>(snippets: &'a [Snippet], dictation: &str) -> Option<&'a Snippet> {
    let said = key(dictation);
    if said.is_empty() || LEADS.contains(&said.as_str()) {
        return None;
    }
    snippets.iter().find(|s| !s.text.trim().is_empty() && key(&s.trigger) == said)
}

/// Keeps an edited list usable: triggers with words, no duplicates (the first wins).
pub fn sanitize(snippets: Vec<Snippet>) -> Vec<Snippet> {
    let mut seen = std::collections::HashSet::new();
    snippets
        .into_iter()
        .filter(|s| {
            let key = key(&s.trigger);
            !key.is_empty() && seen.insert(key)
        })
        .map(|mut s| {
            s.text = s.text.chars().take(5_000).collect();
            s
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(trigger: &str, text: &str) -> Snippet {
        Snippet { trigger: trigger.into(), text: text.into() }
    }

    #[test]
    fn the_whole_dictation_names_the_snippet() {
        let all = vec![snip("meine Adresse", "Musterstraße 1\n12345 Berlin"), snip("Signatur", "Viele Grüße\nAnna")];
        assert_eq!(find(&all, "Meine Adresse.").unwrap().text, "Musterstraße 1\n12345 Berlin");
        assert_eq!(find(&all, "Snippet Signatur!").unwrap().trigger, "Signatur");
        assert_eq!(find(&all, "Baustein, meine Adresse").unwrap().trigger, "meine Adresse");
        // Only the phrase on its own.
        assert!(find(&all, "Schick mir bitte meine Adresse.").is_none());
        assert!(find(&all, "Snippet").is_none());
        assert!(find(&[snip("leer", "  ")], "leer").is_none(), "an empty text inserts nothing");
        // A trigger saved with the lead word matches with or without it.
        let led = vec![snip("Snippet Signatur", "Gruß")];
        assert!(find(&led, "Snippet Signatur").is_some() && find(&led, "Signatur").is_some());
    }

    #[test]
    fn edited_lists_stay_usable() {
        let list = sanitize(vec![snip("Adresse", "a"), snip("adresse.", "b"), snip("!!", "c")]);
        assert_eq!(list, vec![snip("Adresse", "a")]);
    }
}
