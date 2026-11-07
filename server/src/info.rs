//! What the server tells about itself: the phone's address, the settings, its state, its log.

use std::net::IpAddr;
use std::path::Path;

use amber_core::api;
use amber_core::local;

use crate::config::{Config, KEYS};
use crate::paths::{cleanup_dir, llm_ready, model_ready, speech_dir};

/// [key]'s address and API key, apart, as with OpenAI: the addresses in the home network and,
/// with the tunnel on, from anywhere; the key on its own line; and the pairing link that
/// carries both, also as a QR code to scan - with the address from anywhere where there is
/// one (it works at home too).
pub fn describe(config: &Config, key: &amber_core::api::AccessKey, interfaces: &[(String, IpAddr)]) -> String {
    let addresses = api::phone_addresses(interfaces);
    let public = (config.tunnel && !config.public_host.is_empty()).then(|| api::public_url(&config.public_host));
    let to_scan = public.clone().or_else(|| addresses.first().map(|(_, ip)| api::phone_url(*ip, config.port)));
    let Some(to_scan) = to_scan else {
        return "This machine has no address in a home network right now - is it connected? (Or reach it from anywhere: amber-server tunnel)".into();
    };
    let mut out = String::from("In Amber on the phone: Engine -> Own server, Address and API key:\n\n");
    for (name, ip) in &addresses {
        out += &format!("  Address   {}   ({name})\n", api::phone_url(*ip, config.port));
    }
    if let Some(p) = &public {
        out += &format!("  Address   {p}   (from anywhere)\n");
    }
    out += &format!("  API key   {}   (key \"{}\")\n", key.secret, key.name);
    let link = api::pairing_link(&to_scan, &key.secret);
    out += &format!("\nOr both at once - scan the code, or paste this link:\n  {link}\n\n");
    use std::io::IsTerminal;
    out += &qr(&link, std::io::stdout().is_terminal());
    out
}

/// [text] as a QR code in block characters - [colored]: black on white whatever the
/// terminal's colours (blank for light modules reads inverted on a dark terminal).
pub fn qr(text: &str, colored: bool) -> String {
    let Ok(code) = qrcode::QrCode::new(text.as_bytes()) else { return String::new() };
    let plain = code.render::<qrcode::render::unicode::Dense1x2>().quiet_zone(true).build();
    let lines: Vec<String> =
        plain.lines().map(|l| if colored { format!("\x1b[30;47m{l}\x1b[0m") } else { l.to_string() }).collect();
    lines.join("\n") + "\n"
}

/// This machine's network addresses.
pub fn interfaces() -> Vec<(String, IpAddr)> {
    if_addrs::get_if_addrs().unwrap_or_default().into_iter().map(|i| (i.name.clone(), i.ip())).collect()
}

/// Every setting - the key only by its end.
pub fn listing(config: &Config, path: &Path) -> String {
    let mut out = format!("{}\n", path.display());
    if config.keys.is_empty() {
        out += "  keys: none yet - amber-server setup\n";
    }
    for k in &config.keys {
        out += &format!("  key {} = …{}  ({})\n", k.name, tail(&k.secret, 4), key_terms(k));
    }
    out += &format!("  speech = {}\n", config.speech);
    out += &format!("  cleanup = {}\n", config.cleanup.as_deref().unwrap_or("off"));
    for k in KEYS {
        out += &format!("  {k} = {}\n", config.get(k).unwrap_or_default());
    }
    out
}

/// What a key may: "every model, 2000 a day".
pub fn key_terms(k: &amber_core::api::AccessKey) -> String {
    let models = k.models.as_ref().map_or("every model".to_string(), |m| m.join(", "));
    format!("{models}, {} a day", k.per_day.map_or(2000, |d| d))
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
    if config.tunnel {
        out += &format!("tunnel   on, {} - amber-server tunnel\n", config.public_host);
    }
    let health = amber_core::engine::health(&format!("http://127.0.0.1:{}", config.port));
    out += &format!("server   {}\n", server_line(config.port, health.as_ref()));
    out
}

/// Whether Amber Server answers on [port] - and not something else, like the desktop app.
pub fn server_line(port: u16, health: Option<&serde_json::Value>) -> String {
    match health.map(|h| (h["service"].as_str().unwrap_or("another program"), h["version"].as_str().unwrap_or("?"))) {
        Some(("amber-server", version)) => format!("answers on port {port} ({version})"),
        Some((other, _)) => format!("not running - port {port} is taken by {other}: amber-server config set port <number>"),
        None => "not running".into(),
    }
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
        let out = std::process::Command::new("journalctl")
            .args(["--user", "-u", "amber-server", "-n", "50", "--no-pager"])
            .output()
            .map_err(|e| format!("journalctl: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || text.trim().is_empty() || text.trim() == "-- No entries --" {
            return Err("no log lines - is the service installed (amber-server service install)? \
                        Some systems keep the user's log only for members of the systemd-journal group."
                .into());
        }
        print!("{text}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_home_address_first_with_its_code() {
        let c = Config::with_key("abc");
        let out = describe(&c, &c.keys[0], &[("WSL".into(), "172.20.0.2".parse().unwrap()), ("Ethernet".into(), "192.168.1.20".parse().unwrap())]);
        // The address and the key apart, as with OpenAI; the pairing link carries both.
        assert!(out.contains("Address") && out.contains("http://192.168.1.20:8765/v1"), "{out}");
        assert!(out.lines().any(|l| l.contains("API key") && l.contains("abc")), "{out}");
        assert!(!out.contains("/abc/"), "no key in an address: {out}");
        assert!(out.contains("amber://server?url=http%3A%2F%2F192.168.1.20%3A8765%2Fv1&key=abc"), "{out}");
        assert!(!out.contains("172.20.0.2"), "virtual adapters are left out");
        assert!(out.contains('▀') || out.contains('▄') || out.contains('█'), "a QR code in block characters");
        let none = describe(&c, &c.keys[0], &[]);
        assert!(none.contains("no address"), "{none}");
    }

    #[test]
    fn the_code_is_dark_on_light_in_any_terminal() {
        // Spaces for light modules read inverted on a dark terminal, and scanners refuse that.
        let colored = qr("http://192.168.1.20:8765/abc/v1", true);
        let lines: Vec<&str> = colored.lines().filter(|l| !l.is_empty()).collect();
        assert!(lines.len() > 10);
        assert!(lines.iter().all(|l| l.starts_with("\x1b[30;47m") && l.ends_with("\x1b[0m")), "{colored:?}");
        assert!(!qr("http://192.168.1.20:8765/abc/v1", false).contains('\x1b'), "no escapes into a file or pipe");
    }

    #[test]
    fn status_tells_who_answers_on_the_port() {
        let ours = serde_json::json!({ "service": "amber-server", "status": "ok", "version": "0.5.0" });
        assert_eq!(server_line(8765, Some(&ours)), "answers on port 8765 (0.5.0)");
        let desktop = serde_json::json!({ "service": "amber-pc", "status": "ok" });
        assert_eq!(server_line(8765, Some(&desktop)), "not running - port 8765 is taken by amber-pc: amber-server config set port <number>");
        assert_eq!(server_line(8765, None), "not running");
    }

    #[test]
    fn the_address_from_anywhere_is_shown_with_the_tunnel_on() {
        let mut c = Config::with_key("abc");
        let home = [("Ethernet".into(), "192.168.1.20".parse().unwrap())];
        assert!(!describe(&c, &c.keys[0], &home).contains("https://"));
        c.tunnel = true;
        c.public_host = "amber.example.com".into();
        let out = describe(&c, &c.keys[0], &home);
        assert!(out.contains("https://amber.example.com/v1"), "{out}");
        assert!(out.contains("amber://server?url=https%3A%2F%2Famber.example.com%2Fv1&key=abc"), "the link to scan: {out}");
        // A server without a home network (a rented one): the tunnel's address all the same.
        let away = describe(&c, &c.keys[0], &[]);
        assert!(away.contains("https://amber.example.com/v1") && away.contains('▀'), "{away}");
    }

    #[test]
    fn config_never_shows_the_whole_key() {
        let c = Config::with_key("abcdefghjkmnpqrstuvw");
        let out = listing(&c, std::path::Path::new("/x/amber-server.toml"));
        assert!(out.contains("tuvw") && !out.contains("abcdefghjkmnpqrs"), "{out}");
        assert!(out.contains("rules.fillers = true") && out.contains("/x/amber-server.toml"));
    }
}
