//! `amber-server setup`: quick (no questions - what the installer runs) or asking.

use std::io::{BufRead, Write};
use std::path::Path;

use amber_core::local;

use crate::catalog::{self, fit, recommended_speech, Machine};
use crate::config::Config;

/// The downloads setup starts - stubbed in the tests.
pub trait Fetch {
    fn speech(&self, id: &str) -> Result<(), String>;
    fn cleanup(&self, id: &str) -> Result<(), String>;
}

/// The real downloads.
pub struct Downloads {
    pub gpu: bool,
}

impl Fetch for Downloads {
    fn speech(&self, id: &str) -> Result<(), String> {
        catalog::download_speech(id)
    }
    fn cleanup(&self, id: &str) -> Result<(), String> {
        catalog::download_cleanup(id, self.gpu)
    }
}

pub fn quick(path: &Path) -> Result<Config, String> {
    let config = existing(path)?;
    let machine = catalog::machine(&config);
    quick_with(path, machine, &Downloads { gpu: machine.gpu })
}

pub fn ask(path: &Path, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Config, String> {
    let config = existing(path)?;
    let machine = catalog::machine(&config);
    ask_with(path, machine, &Downloads { gpu: machine.gpu }, input, out)
}

fn existing(path: &Path) -> Result<Config, String> {
    if path.exists() {
        Config::load(path)
    } else {
        Ok(Config::default())
    }
}

/// The recommended speech model, a key, no new cleanup. Run again (the installer does on
/// every update) it keeps the model chosen before.
pub fn quick_with(path: &Path, machine: Machine, fetch: &dyn Fetch) -> Result<Config, String> {
    let mut config = existing(path)?;
    if !set_up(&config) {
        config.speech = recommended_speech(machine.gpu, machine.threads, machine.ram_mb).to_string();
        config.port = free_from(config.port);
    }
    finish(path, config, fetch)
}

/// [port], or the next free one when something holds it - Amber for Windows on the same PC
/// has 8765. Only for a first setup: once set up, the server itself holds its port.
fn free_from(port: u16) -> u16 {
    let free = |p: u16| std::net::TcpListener::bind(("0.0.0.0", p)).is_ok();
    if free(port) { port } else { (port.saturating_add(1)..=u16::MAX).find(|p| free(*p)).unwrap_or(port) }
}

/// Whether setup ran before: only it makes the key (`config set` before it leaves it empty).
fn set_up(config: &Config) -> bool {
    !config.key.trim().is_empty()
}

/// Asks for the speech model, the cleanup, the language and the port; Enter takes the default.
pub fn ask_with(path: &Path, machine: Machine, fetch: &dyn Fetch, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<Config, String> {
    let mut config = existing(path)?;
    let w = |e: std::io::Error| e.to_string();

    let speech: Vec<_> = local::MODELS.iter().filter(|m| !m.replaced).collect();
    let recommended = recommended_speech(machine.gpu, machine.threads, machine.ram_mb);
    // Enter keeps what was chosen before.
    let current = if set_up(&config) { config.speech.as_str() } else { recommended };
    let default = speech.iter().position(|m| m.id == current).unwrap_or(0);
    writeln!(out, "Speech model:").map_err(w)?;
    for (i, m) in speech.iter().enumerate() {
        writeln!(out, "  {}  {:<24} {:>5} MB  {}", i + 1, m.label, m.size_mb, fit(m, machine.gpu, machine.threads, machine.ram_mb)).map_err(w)?;
    }
    config.speech = speech[pick(input, out, "Number", default + 1, 1, speech.len())? - 1].id.to_string();

    writeln!(out, "\nCleanup (tidies punctuation and slips with a language model - large download):").map_err(w)?;
    writeln!(out, "  0  none").map_err(w)?;
    for (i, m) in local::LLM_MODELS.iter().enumerate() {
        writeln!(out, "  {}  {:<24} {:>5} MB", i + 1, m.label, m.size_mb).map_err(w)?;
    }
    let current = config.cleanup.as_deref().and_then(|id| local::LLM_MODELS.iter().position(|m| m.id == id)).map_or(0, |i| i + 1);
    config.cleanup = match pick(input, out, "Number", current, 0, local::LLM_MODELS.len())? {
        0 => None,
        n => Some(local::LLM_MODELS[n - 1].id.to_string()),
    };

    loop {
        let answer = line(input, out, &format!("\nLanguage (auto, de, en ...) [{}]", config.language))?;
        if answer.is_empty() {
            break;
        }
        match config.set("language", &answer) {
            Ok(()) => break,
            Err(e) => writeln!(out, "  {e}").map_err(w)?,
        }
    }
    if !set_up(&config) {
        config.port = free_from(config.port);
    }
    loop {
        let answer = line(input, out, &format!("Port [{}]", config.port))?;
        if answer.is_empty() {
            break;
        }
        match config.set("port", &answer) {
            Ok(()) => break,
            Err(e) => writeln!(out, "  {e}").map_err(w)?,
        }
    }
    finish(path, config, fetch)
}

/// Downloads what the config names, makes a key if there is none, saves.
fn finish(path: &Path, mut config: Config, fetch: &dyn Fetch) -> Result<Config, String> {
    fetch.speech(&config.speech)?;
    if let Some(id) = &config.cleanup {
        // The speech works without it; the choice stays for a later `cleanup add`.
        if let Err(e) = fetch.cleanup(id) {
            eprintln!("The cleanup model {id} could not be downloaded ({e}) - try again later: amber-server cleanup add {id}");
        }
    }
    // A config written by `config set` before setup has an empty key.
    if config.key.trim().is_empty() {
        config.key = amber_core::api::new_key();
    }
    config.save(path)?;
    Ok(config)
}

/// A number from [min] to [max]; Enter takes [default]. Asks again on anything else.
fn pick(input: &mut dyn BufRead, out: &mut dyn Write, prompt: &str, default: usize, min: usize, max: usize) -> Result<usize, String> {
    loop {
        let answer = line(input, out, &format!("{prompt} [{default}]"))?;
        if answer.is_empty() {
            return Ok(default);
        }
        match answer.parse::<usize>() {
            Ok(n) if (min..=max).contains(&n) => return Ok(n),
            _ => writeln!(out, "  a number from the list").map_err(|e| e.to_string())?,
        }
    }
}

fn line(input: &mut dyn BufRead, out: &mut dyn Write, prompt: &str) -> Result<String, String> {
    write!(out, "{prompt}: ").map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    let mut answer = String::new();
    input.read_line(&mut answer).map_err(|e| e.to_string())?;
    Ok(answer.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct Stub {
        speech: RefCell<Vec<String>>,
        cleanup: RefCell<Vec<String>>,
    }

    struct FailingCleanup;
    impl Fetch for FailingCleanup {
        fn speech(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn cleanup(&self, _: &str) -> Result<(), String> {
            Err("network down".into())
        }
    }

    impl Fetch for Stub {
        fn speech(&self, id: &str) -> Result<(), String> {
            self.speech.borrow_mut().push(id.into());
            Ok(())
        }
        fn cleanup(&self, id: &str) -> Result<(), String> {
            self.cleanup.borrow_mut().push(id.into());
            Ok(())
        }
    }

    const OFFICE_PC: Machine = Machine { gpu: false, threads: 4, ram_mb: 4_000 };

    fn path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("amber-server.toml");
        (dir, p)
    }

    #[test]
    fn asking_with_enter_everywhere_equals_quick() {
        let (_d, p) = path();
        let stub = Stub::default();
        let mut out = Vec::new();
        let asked = ask_with(&p, OFFICE_PC, &stub, &mut "\n\n\n\n".as_bytes(), &mut out).unwrap();
        let (_d2, p2) = path();
        let quick = quick_with(&p2, OFFICE_PC, &Stub::default()).unwrap();
        assert_eq!(asked.speech, quick.speech);
        assert_eq!(asked.speech, "futo-multilingual-74", "the office PC's recommendation");
        // 8765, or the next free port where Amber for Windows holds it.
        assert_eq!((asked.cleanup.as_deref(), asked.port), (None, quick.port));
        assert!(asked.port >= 8765);
        assert_eq!(*stub.speech.borrow(), ["futo-multilingual-74"]);
        assert!(stub.cleanup.borrow().is_empty());
        assert_eq!(asked.key.len(), 20, "a key was made");
        let shown = String::from_utf8(out).unwrap();
        assert!(shown.contains("recommended") && shown.contains("Qwen3"), "{shown}");
    }

    #[test]
    fn asking_takes_the_numbers_given() {
        let (_d, p) = path();
        let stub = Stub::default();
        let c = ask_with(&p, OFFICE_PC, &stub, &mut "2\n1\nde\n9000\n".as_bytes(), &mut Vec::new()).unwrap();
        assert_eq!(c.speech, "futo-multilingual-244");
        assert_eq!(c.cleanup.as_deref(), Some("qwen3-4b-instruct-2507-q4km"));
        assert_eq!((c.language.as_str(), c.port), ("de", 9000));
        assert_eq!(*stub.cleanup.borrow(), ["qwen3-4b-instruct-2507-q4km"]);
        assert_eq!(Config::load(&p).unwrap(), c, "saved");
    }

    #[test]
    fn a_wrong_answer_is_asked_again() {
        let (_d, p) = path();
        let mut out = Vec::new();
        // 0 and 9 are no speech model, "x" no number, "xx" no language.
        let c = ask_with(&p, OFFICE_PC, &Stub::default(), &mut "0\n9\nx\n1\n\nxx\nen\n\n".as_bytes(), &mut out).unwrap();
        assert_eq!((c.speech.as_str(), c.language.as_str()), ("futo-multilingual-74", "en"));
        assert!(String::from_utf8(out).unwrap().contains("a number from the list"));
    }

    #[test]
    fn running_setup_again_keeps_the_chosen_model() {
        // The installer runs setup --quick on every update.
        let (_d, p) = path();
        let big = Machine { gpu: true, threads: 16, ram_mb: 32_000 };
        Config { key: "k".repeat(20), speech: "futo-multilingual-74".into(), ..Config::default() }.save(&p).unwrap();
        assert_eq!(quick_with(&p, big, &Stub::default()).unwrap().speech, "futo-multilingual-74");
        let asked = ask_with(&p, big, &Stub::default(), &mut "\n\n\n\n".as_bytes(), &mut Vec::new()).unwrap();
        assert_eq!(asked.speech, "futo-multilingual-74", "Enter keeps it");
        // Settings made before the first setup do not count as a choice.
        let (_d2, p2) = path();
        Config { port: 9000, ..Config::default() }.save(&p2).unwrap();
        assert_eq!(quick_with(&p2, big, &Stub::default()).unwrap().speech, recommended_speech(true, 16, 32_000));
    }

    #[test]
    fn a_failed_cleanup_download_does_not_stop_setup() {
        let (_d, p) = path();
        Config { key: "k".repeat(20), cleanup: Some("qwen3-4b-instruct-2507-q4km".into()), ..Config::default() }.save(&p).unwrap();
        let c = quick_with(&p, OFFICE_PC, &FailingCleanup).unwrap();
        assert_eq!(c.cleanup.as_deref(), Some("qwen3-4b-instruct-2507-q4km"), "kept, to retry with cleanup add");
        assert!(Config::load(&p).is_ok());
    }

    #[test]
    fn the_first_setup_takes_a_free_port() {
        // Amber for Windows on the same PC has 8765 already.
        let taken = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let (_d, p) = path();
        Config { port, ..Config::default() }.save(&p).unwrap();
        let c = quick_with(&p, OFFICE_PC, &Stub::default()).unwrap();
        assert_ne!(c.port, port);
        assert!(std::net::TcpListener::bind(("0.0.0.0", c.port)).is_ok(), "{} is free", c.port);
        // Set up before: the port is the user's choice, even while something holds it.
        Config { key: "k".repeat(20), port, ..Config::default() }.save(&p).unwrap();
        assert_eq!(quick_with(&p, OFFICE_PC, &Stub::default()).unwrap().port, port);
    }

    #[test]
    fn asking_offers_a_free_port() {
        let taken = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let (_d, p) = path();
        Config { port, ..Config::default() }.save(&p).unwrap();
        let c = ask_with(&p, OFFICE_PC, &Stub::default(), &mut "\n\n\n\n".as_bytes(), &mut Vec::new()).unwrap();
        assert_ne!(c.port, port, "Enter must not take a port the server cannot get");
    }

    #[test]
    fn setup_keeps_the_key() {
        let (_d, p) = path();
        Config { key: "k".repeat(20), ..Config::default() }.save(&p).unwrap();
        assert_eq!(quick_with(&p, OFFICE_PC, &Stub::default()).unwrap().key, "k".repeat(20));
    }
}
