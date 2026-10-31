//! The background service: a systemd user unit on Linux, a scheduled task at log-on on Windows.

use std::process::Command;

/// The task's name on Windows.
#[cfg(windows)]
const TASK: &str = "Amber Server";

/// Where the Windows task's output goes.
#[cfg(windows)]
pub fn log_file() -> std::path::PathBuf {
    crate::paths::data_dir().join("amber-server.log")
}

/// "active", "inactive", "not installed" ...
pub fn state() -> String {
    #[cfg(windows)]
    let out = Command::new("schtasks").args(["/query", "/tn", TASK, "/fo", "list"]).output();
    #[cfg(not(windows))]
    let out = Command::new("systemctl").args(["--user", "is-active", "amber-server"]).output();
    match out {
        Ok(o) if cfg!(windows) && o.status.success() => "installed (starts at log-on)".into(),
        Ok(o) if !cfg!(windows) => match String::from_utf8_lossy(&o.stdout).trim() {
            "" | "unknown" => "not installed - amber-server service install".into(),
            s => s.to_string(),
        },
        _ => "not installed - amber-server service install".into(),
    }
}
