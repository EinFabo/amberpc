//! The background service: a systemd user unit on Linux, a scheduled task at log-on on Windows.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The task's name on Windows.
#[cfg_attr(not(windows), allow(dead_code))]
const TASK: &str = "Amber Server";

/// A log file above this starts over (the previous one is kept as `.old`).
const MAX_LOG: u64 = 5 << 20;

/// Where the Windows task's output goes.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn log_file() -> PathBuf {
    crate::paths::data_dir().join("amber-server.log")
}

/// The systemd user unit. It restarts after a crash, not after exit 78 ("set it up first").
pub fn systemd_unit(exe: &Path) -> String {
    // systemd reads % as a specifier and \ and " as escapes inside quotes.
    let exe = exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%");
    format!(
        "[Unit]\n\
         Description=Amber Server - speech to text for Amber on the phone\n\
         \n\
         [Service]\n\
         ExecStart=\"{exe}\" run\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         RestartPreventExitStatus=78\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    )
}

/// `schtasks` arguments for a task at log-on with the user's own rights. `run --log` writes
/// to the file and lets go of its console - a console window at every log-on would end the
/// server when someone closes it.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn schtasks_args(exe: &Path, log: &Path) -> Vec<String> {
    let action = format!("\"{}\" run --log \"{}\"", exe.display(), log.display());
    ["/create", "/tn", TASK, "/tr", &action, "/sc", "onlogon", "/rl", "limited", "/f"].map(String::from).to_vec()
}

/// The log file, appended to - or started over when it got big.
pub fn open_log(path: &Path) -> std::io::Result<File> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_LOG) {
        let mut old = path.as_os_str().to_owned();
        old.push(".old");
        std::fs::rename(path, old)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::OpenOptions::new().create(true).append(true).open(path)
}

/// From here on stdout and stderr go to [path]. On Windows a console that was opened just
/// for this process (the task's) is let go.
pub fn redirect_output(path: &Path) -> Result<(), String> {
    let file = open_log(path).map_err(|e| format!("{}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: plain descriptor duplication onto stdout and stderr.
        unsafe {
            libc::dup2(file.as_raw_fd(), 1);
            libc::dup2(file.as_raw_fd(), 2);
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::IntoRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Console::{FreeConsole, GetConsoleProcessList, SetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
        // Kept open for the life of the process: it is stdout now.
        let handle = HANDLE(file.into_raw_handle());
        // SAFETY: a valid file handle; Rust's stdout and stderr look it up on every write.
        unsafe {
            SetStdHandle(STD_OUTPUT_HANDLE, handle).map_err(|e| e.to_string())?;
            SetStdHandle(STD_ERROR_HANDLE, handle).map_err(|e| e.to_string())?;
            let mut ids = [0u32; 4];
            if GetConsoleProcessList(&mut ids) <= 1 {
                let _ = FreeConsole();
            }
        }
        Ok(())
    }
}

/// Where systemd looks for the user's own units.
#[cfg(not(windows))]
fn unit_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".config")).join("systemd/user/amber-server.service")
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("{program} {}: {}", args.join(" "), err.trim()))
    }
}

/// Installs and starts the service; again after an update, it restarts with the new binary.
pub fn install() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        let args = schtasks_args(&exe, &log_file());
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        run("schtasks", &args)?;
        let _ = run("schtasks", &["/end", "/tn", TASK]);
        run("schtasks", &["/run", "/tn", TASK])?;
        Ok(format!("Amber Server runs now and starts whenever you log on. Its log: {}", log_file().display()))
    }
    #[cfg(not(windows))]
    {
        let no_systemd = |e: String| {
            format!("{e}\nsystemd does not run for this user here - start the server yourself instead: amber-server run")
        };
        run("systemctl", &["--user", "is-system-running"]).or_else(|e| if e.contains("degraded") { Ok(String::new()) } else { Err(e) }).map_err(no_systemd)?;
        let path = unit_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(&path, systemd_unit(&exe)).map_err(|e| format!("{}: {e}", path.display()))?;
        run("systemctl", &["--user", "daemon-reload"])?;
        run("systemctl", &["--user", "enable", "amber-server"])?;
        run("systemctl", &["--user", "restart", "amber-server"])?;
        let mut message = "Amber Server runs now and starts with the machine. Its log: amber-server logs".to_string();
        // Without lingering, systemd ends the user's services when they log out.
        let user = std::env::var("USER").unwrap_or_else(|_| "$USER".into());
        let lingers = run("loginctl", &["show-user", &user, "-p", "Linger"]).is_ok_and(|o| o.trim() == "Linger=yes");
        if !lingers && run("loginctl", &["enable-linger"]).is_err() {
            message += &format!("\nSo it also runs while you are logged out: sudo loginctl enable-linger {user}");
        }
        Ok(message)
    }
}

pub fn uninstall() -> Result<String, String> {
    #[cfg(windows)]
    {
        let _ = run("schtasks", &["/end", "/tn", TASK]);
        run("schtasks", &["/delete", "/tn", TASK, "/f"]).map_err(|_| "the service is not installed".to_string())?;
        Ok("Amber Server stopped and no longer starts at log-on. Models and settings are kept.".into())
    }
    #[cfg(not(windows))]
    {
        let path = unit_path();
        if !path.exists() {
            return Err("the service is not installed".into());
        }
        let _ = run("systemctl", &["--user", "disable", "--now", "amber-server"]);
        std::fs::remove_file(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = run("systemctl", &["--user", "daemon-reload"]);
        Ok("Amber Server stopped and no longer starts with the machine. Models and settings are kept.".into())
    }
}

/// "active", "failed", "not installed" ...
pub fn state() -> String {
    const NOT: &str = "not installed - amber-server service install";
    #[cfg(windows)]
    {
        match run("schtasks", &["/query", "/tn", TASK]) {
            Ok(_) => "installed (starts at log-on)".into(),
            Err(_) => NOT.into(),
        }
    }
    #[cfg(not(windows))]
    {
        if !unit_path().exists() {
            return NOT.into();
        }
        // is-active fails for anything but "active" and still prints the state.
        let out = Command::new("systemctl").args(["--user", "is-active", "amber-server"]).output();
        match out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()) {
            Ok(s) if s == "failed" => "failed - amber-server logs".into(),
            Ok(s) if !s.is_empty() => s,
            _ => "installed".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_unit_restarts_on_failure_but_not_without_setup() {
        let unit = systemd_unit(Path::new("/home/u/.local/bin/amber-server"));
        assert!(unit.contains("ExecStart=\"/home/u/.local/bin/amber-server\" run\n"), "{unit}");
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("RestartPreventExitStatus=78\n"));
        assert!(unit.contains("WantedBy=default.target\n"));
        // systemd reads % as a specifier and \ as an escape.
        let odd = systemd_unit(Path::new("/opt/my apps/50%/amber-server"));
        assert!(odd.contains("ExecStart=\"/opt/my apps/50%%/amber-server\" run\n"), "{odd}");
    }

    #[test]
    fn the_task_starts_at_logon_without_admin_rights() {
        let args = schtasks_args(Path::new(r"C:\Users\A B\AppData\Local\Programs\amber-server\amber-server.exe"), Path::new(r"C:\Users\A B\AppData\Local\amber-server\amber-server.log"));
        let has = |pair: [&str; 2]| args.windows(2).any(|w| w == pair);
        assert!(has(["/sc", "onlogon"]) && has(["/rl", "limited"]) && has(["/tn", "Amber Server"]), "{args:?}");
        assert!(args.contains(&"/f".to_string()));
        assert!(has([
            "/tr",
            r#""C:\Users\A B\AppData\Local\Programs\amber-server\amber-server.exe" run --log "C:\Users\A B\AppData\Local\amber-server\amber-server.log""#
        ]), "the output goes to the log file, without a console window: {args:?}");
    }

    #[test]
    fn a_big_log_starts_over() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("amber-server.log");
        std::fs::write(&log, vec![b'x'; (MAX_LOG + 1) as usize]).unwrap();
        drop(open_log(&log).unwrap());
        assert_eq!(std::fs::metadata(&log).unwrap().len(), 0);
        assert!(dir.path().join("amber-server.log.old").exists(), "the previous one kept once");
        std::fs::write(&log, b"short\n").unwrap();
        use std::io::Write;
        writeln!(open_log(&log).unwrap(), "more").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "short\nmore\n", "appended");
    }
}
