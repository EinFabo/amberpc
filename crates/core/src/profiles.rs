//! Different programs, different wording: the same dictation reads casual in Discord, formal
//! in Outlook, plain in an editor. A profile per program (by its exe) picks the writing style
//! the smart cleanup uses, an own instruction, and whether the smart cleanup runs there at
//! all. Port of Amber for Android's `AppProfiles` (there per app package).

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WritingStyle {
    /// Only tidied: punctuation, false starts, self-corrections.
    #[default]
    AsSpoken,
    /// A chat message: relaxed, the way the speaker would type it.
    Casual,
    /// An email: complete sentences, polite and professional.
    Formal,
    /// A note: short, every fact kept.
    Concise,
}

/// Whether the smart cleanup runs in a program.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum CleanupChoice {
    /// As the switch under Cleanup says.
    #[default]
    Default,
    Always,
    Never,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    /// The program's file name, lowercase: "discord.exe".
    pub exe: String,
    /// What the settings show: "Discord".
    pub label: String,
    pub style: WritingStyle,
    /// The user's own words for this program ("always address people formally").
    pub instruction: String,
    pub cleanup: CleanupChoice,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { exe: String::new(), label: String::new(), style: WritingStyle::AsSpoken, instruction: String::new(), cleanup: CleanupChoice::Default }
    }
}

/// How long an own instruction may be - it goes into every cleanup request.
pub const INSTRUCTION_CHARS: usize = 300;

fn profile(exe: &str, label: &str, style: WritingStyle) -> Profile {
    Profile { exe: exe.into(), label: label.into(), style, ..Profile::default() }
}

/// What a fresh install starts with. Code editors and terminals stay as spoken - the default
/// for every program without a profile anyway.
pub fn defaults() -> Vec<Profile> {
    use WritingStyle::*;
    vec![
        profile("discord.exe", "Discord", Casual),
        profile("whatsapp.exe", "WhatsApp", Casual),
        profile("telegram.exe", "Telegram", Casual),
        profile("signal.exe", "Signal", Casual),
        profile("slack.exe", "Slack", Casual),
        profile("ms-teams.exe", "Microsoft Teams", Casual),
        profile("outlook.exe", "Outlook (classic)", Formal),
        profile("olk.exe", "Outlook (new)", Formal),
        profile("thunderbird.exe", "Thunderbird", Formal),
        profile("obsidian.exe", "Obsidian", Concise),
        profile("onenote.exe", "OneNote", Concise),
    ]
}

/// The profile for [exe] ("Discord.exe" or a full path), if there is one.
pub fn find<'a>(profiles: &'a [Profile], exe: &str) -> Option<&'a Profile> {
    let name = exe_name(exe);
    profiles.iter().find(|p| p.exe == name)
}

/// "C:\\Program Files\\Discord\\Discord.exe" -> "discord.exe".
pub fn exe_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).trim().to_lowercase()
}

/// Whether the smart cleanup runs for a dictation into [profile]'s program.
pub fn smart_cleanup(global: bool, profile: Option<&Profile>) -> bool {
    match profile.map(|p| p.cleanup) {
        Some(CleanupChoice::Always) => true,
        Some(CleanupChoice::Never) => false,
        _ => global,
    }
}

/// Keeps a list the user edited usable: names normalized, duplicates and empty entries
/// dropped (the first one wins), instructions bounded.
pub fn sanitize(profiles: Vec<Profile>) -> Vec<Profile> {
    let mut seen = std::collections::HashSet::new();
    profiles
        .into_iter()
        .filter_map(|mut p| {
            p.exe = exe_name(&p.exe);
            if p.exe.is_empty() || !seen.insert(p.exe.clone()) {
                return None;
            }
            if p.label.trim().is_empty() {
                p.label = p.exe.trim_end_matches(".exe").to_string();
            }
            p.instruction = p.instruction.chars().take(INSTRUCTION_CHARS).collect();
            Some(p)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_program_by_its_exe() {
        let all = defaults();
        assert_eq!(find(&all, r"C:\Users\fabia\AppData\Local\Discord\app-1.0\Discord.exe").unwrap().style, WritingStyle::Casual);
        assert_eq!(find(&all, "OUTLOOK.EXE").unwrap().style, WritingStyle::Formal);
        assert!(find(&all, "code.exe").is_none(), "no profile: as spoken");
    }

    #[test]
    fn the_choice_per_program_wins_over_the_switch() {
        let mut p = profile("x.exe", "X", WritingStyle::Casual);
        assert!(smart_cleanup(true, Some(&p)));
        assert!(!smart_cleanup(false, Some(&p)));
        p.cleanup = CleanupChoice::Always;
        assert!(smart_cleanup(false, Some(&p)));
        p.cleanup = CleanupChoice::Never;
        assert!(!smart_cleanup(true, Some(&p)));
        assert!(smart_cleanup(true, None));
    }

    #[test]
    fn edited_lists_stay_usable() {
        let mut long = profile(" Notepad.EXE ", "", WritingStyle::Concise);
        long.instruction = "x".repeat(1000);
        let list = sanitize(vec![long, profile("notepad.exe", "Second", WritingStyle::Formal), profile("", "Empty", WritingStyle::Casual)]);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].exe, "notepad.exe");
        assert_eq!(list[0].label, "notepad");
        assert_eq!(list[0].style, WritingStyle::Concise, "the first one wins");
        assert_eq!(list[0].instruction.len(), INSTRUCTION_CHARS);
        assert_eq!(sanitize(defaults()), defaults(), "the defaults are already clean");
    }
}
