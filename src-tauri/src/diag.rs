//! The diagnostics log: the last few hundred events in memory for the settings window, and
//! a small file next to the app data for when something went wrong yesterday.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

const KEEP: usize = 300;
const MAX_FILE: u64 = 1 << 20;

#[derive(Serialize, Clone)]
pub struct Entry {
    pub time: String,
    pub level: &'static str,
    pub message: String,
}

struct Log {
    entries: VecDeque<Entry>,
    file: Option<File>,
}

static LOG: OnceLock<Mutex<Log>> = OnceLock::new();

fn log() -> &'static Mutex<Log> {
    LOG.get_or_init(|| Mutex::new(Log { entries: VecDeque::new(), file: None }))
}

/// Opens `amber.log` in [dir]; a file over 1 MB is moved to `amber.old.log` first.
pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("amber.log");
    if std::fs::metadata(&path).map(|m| m.len() > MAX_FILE).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("amber.old.log"));
    }
    let file = OpenOptions::new().create(true).append(true).open(path).ok();
    log().lock().unwrap().file = file;
}

fn push(level: &'static str, message: String) {
    let entry = Entry { time: now(), level, message };
    let mut log = log().lock().unwrap();
    if let Some(f) = log.file.as_mut() {
        let _ = writeln!(f, "{} {:5} {}", entry.time, entry.level, entry.message);
    }
    #[cfg(debug_assertions)]
    eprintln!("{} {:5} {}", entry.time, entry.level, entry.message);
    if log.entries.len() >= KEEP {
        log.entries.pop_front();
    }
    log.entries.push_back(entry);
}

pub fn info(message: impl Into<String>) {
    push("info", message.into());
}

pub fn warn(message: impl Into<String>) {
    push("warn", message.into());
}

pub fn error(message: impl Into<String>) {
    push("error", message.into());
}

pub fn entries() -> Vec<Entry> {
    log().lock().unwrap().entries.iter().cloned().collect()
}

#[cfg(windows)]
fn now() -> String {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds)
}

#[cfg(not(windows))]
fn now() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("{:02}:{:02}:{:02} UTC", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}
