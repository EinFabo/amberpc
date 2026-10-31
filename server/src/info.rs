//! What the server tells about itself: the phone's address, the settings, its state, its log.

use std::net::IpAddr;
use std::path::Path;

use amber_core::api;
use amber_core::local;

use crate::config::{Config, KEYS};
use crate::paths::{cleanup_dir, llm_ready, model_ready, speech_dir};

/// The phone's addresses in the home network, the first as a QR code to scan.
pub fn describe(config: &Config, interfaces: &[(String, IpAddr)]) -> String {
    let addresses = api::phone_addresses(interfaces);
    let Some((_, first)) = addresses.first() else {
        return "This machine has no address in a home network right now - is it connected?".into();
    };
    let mut out = String::from("In Amber on the phone: Engine -> Own server, this address:\n\n");
    for (name, ip) in &addresses {
        out += &format!("  {name:<12} {}\n", api::phone_url(*ip, config.port, &config.key));
    }
    let url = api::phone_url(*first, config.port, &config.key);
    if let Ok(code) = qrcode::QrCode::new(url.as_bytes()) {
        out += "\n";
        out += &code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build();
        out += "\n";
    }
    out
}

/// This machine's network addresses.
pub fn interfaces() -> Vec<(String, IpAddr)> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i.ip())).collect()
}

/// Every setting - the key only by its end.
pub fn listing(config: &Config, path: &Path) -> String {
    let mut out = format!("{}\n", path.display());
    let key = if config.key.is_empty() { "(none yet - amber-server setup)".to_string() } else { format!("…{}", tail(&config.key, 4)) };
    out += &format!("  key = {key}\n");
    out += &format!("  speech = {}\n", config.speech);
    out += &format!("  cleanup = {}\n", config.cleanup.as_deref().unwrap_or("off"));
    for k in KEYS {
        out += &format!("  {k} = {}\n", config.get(k).unwrap_or_default());
    }
    out
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

/// Config, models, service, and whether the server answers.
pub fn status(path: &Path) -> String {
    let mut out = String::new();
    let config = if path.exists() { Config::load(path) } else { Err("not set up - run: amber-server setup".into()) };
    match &config {
        Ok(_) => out += &format!("config   {}\n", path.display()),
        Err(e) => out += &format!("config   {e}\n"),
    }
    let config = config.unwrap_or_default();
    let ready = local::model(&config.speech).is_some_and(|m| model_ready(&speech_dir(), m));
    out += &format!("speech   {} ({})\n", config.speech, if ready { "downloaded" } else { "not downloaded" });
    match config.cleanup.as_deref() {
        Some(id) => {
            let ready = local::llm_model(id).is_some_and(|m| llm_ready(&cleanup_dir(), m));
            out += &format!("cleanup  {id} ({})\n", if ready { "downloaded" } else { "not downloaded" });
        }
        None => out += "cleanup  off\n",
    }
    out += &format!("service  {}\n", crate::service::state());
    let answers = amber_core::engine::health_ok(&format!("http://127.0.0.1:{}", config.port));
    out += &format!("server   {}\n", if answers { format!("answers on port {}", config.port) } else { "not running".into() });
    out
}

/// The service's last lines.
pub fn logs() -> Result<(), String> {
    #[cfg(windows)]
    {
        let file = crate::service::log_file();
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let lines: Vec<&str> = text.lines().collect();
        for l in &lines[lines.len().saturating_sub(50)..] {
            println!("{l}");
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let status = std::process::Command::new("journalctl")
            .args(["--user", "-u", "amber-server", "-n", "50", "--no-pager"])
            .status()
            .map_err(|e| format!("journalctl: {e}"))?;
        if status.success() { Ok(()) } else { Err("journalctl found no log - is the service installed? amber-server service install".into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_home_address_first_with_its_code() {
        let c = Config { key: "abc".into(), ..Config::default() };
        let out = describe(&c, &[("WSL".into(), "172.20.0.2".parse().unwrap()), ("Ethernet".into(), "192.168.1.20".parse().unwrap())]);
        assert!(out.contains("http://192.168.1.20:8765/abc/v1"), "{out}");
        assert!(!out.contains("172.20.0.2"), "virtual adapters are left out");
        assert!(out.contains('▀') || out.contains('▄') || out.contains('█'), "a QR code in block characters");
        let none = describe(&c, &[]);
        assert!(none.contains("no address"), "{none}");
    }

    #[test]
    fn config_never_shows_the_whole_key() {
        let c = Config { key: "abcdefghjkmnpqrstuvw".into(), ..Config::default() };
        let out = listing(&c, std::path::Path::new("/x/amber-server.toml"));
        assert!(out.contains("tuvw") && !out.contains("abcdefghjkmnpqrs"), "{out}");
        assert!(out.contains("rules.fillers = true") && out.contains("/x/amber-server.toml"));
    }
}
