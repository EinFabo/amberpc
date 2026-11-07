//! Starting with Windows: the `HKCU\...\Run` value Amber writes, and how to read the switch
//! in Task Manager's "Startup apps", which lives next to it in `StartupApproved\Run`.

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
pub const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
pub const VALUE_NAME: &str = "Amber";
pub const ARG: &str = "--autostart";

/// The settings page a start asks for: `amber.exe --settings phone` (or `--settings=phone`)
/// opens the settings there - a second start hands it to the running Amber. A page name is
/// lowercase letters only; the settings ignore one they do not know.
pub fn settings_page(args: &[String]) -> Option<String> {
    let page = args.iter().enumerate().find_map(|(i, a)| match a.strip_prefix("--settings") {
        Some("") => args.get(i + 1).cloned(),
        Some(rest) => rest.strip_prefix('=').map(str::to_string),
        None => None,
    })?;
    (!page.is_empty() && page.chars().all(|c| c.is_ascii_lowercase())).then_some(page)
}

/// The command line Windows runs at login. Quoted, so a user folder with a space
/// ("C:\Users\Max Mustermann") is one path and not a program called "C:\Users\Max".
pub fn run_command(exe: &str) -> String {
    format!("\"{}\" {ARG}", exe.trim_matches('"'))
}

/// Task Manager stores its switch as a binary value whose first byte is even when the
/// entry is enabled (02, 06) and odd when the user turned it off (03, 07). No value means
/// enabled.
pub fn disabled_in_task_manager(approved: Option<&[u8]>) -> bool {
    approved.and_then(|b| b.first()).is_some_and(|first| first & 1 == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_can_ask_for_a_settings_page() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(settings_page(&args(&["amber.exe", "--settings", "phone"])).as_deref(), Some("phone"));
        assert_eq!(settings_page(&args(&["amber.exe", "--settings=hotkeys"])).as_deref(), Some("hotkeys"));
        assert_eq!(settings_page(&args(&["amber.exe", "--autostart"])), None);
        assert_eq!(settings_page(&args(&["amber.exe", "--settings"])), None, "no page named");
        assert_eq!(settings_page(&args(&["amber.exe", "--settings", "../x"])), None, "a page name, nothing else");
    }


    #[test]
    fn command_is_quoted_once() {
        assert_eq!(
            run_command(r"C:\Users\Max Mustermann\AppData\Local\Amber\amber.exe"),
            r#""C:\Users\Max Mustermann\AppData\Local\Amber\amber.exe" --autostart"#
        );
        assert_eq!(run_command(r#""C:\a\amber.exe""#), r#""C:\a\amber.exe" --autostart"#);
    }

    #[test]
    fn reads_task_managers_switch() {
        assert!(!disabled_in_task_manager(None));
        assert!(!disabled_in_task_manager(Some(&[])));
        assert!(!disabled_in_task_manager(Some(&[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])));
        assert!(disabled_in_task_manager(Some(&[3, 0, 0, 0, 0x10, 0x2f, 0x9a, 0x51, 0x2d, 0x49, 0xdb, 0x01])));
        assert!(!disabled_in_task_manager(Some(&[6, 0])));
        assert!(disabled_in_task_manager(Some(&[7, 0])));
    }
}
