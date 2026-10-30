//! The phone API: an OpenAI-compatible transcription endpoint on the home network, so Amber
//! on the phone can use this PC as its "own server" without any change to the phone app.
//!
//! The phone's own-server engine has no API key field, so the access key travels in the
//! path: the phone gets `http://<pc>:<port>/<key>/v1` as its address and appends
//! `/audio/transcriptions` itself. A Bearer header with the key works too, for other clients.
//! Requests from outside the private network are refused, whatever key they carry.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio;
use crate::engine::{ErrorKind, TranscribeOptions, Transcriber};
use crate::text;

pub const DEFAULT_PORT: u16 = 8765;

/// 20 characters from a 31-letter alphabet without look-alikes: 99 bits, easy to read off a
/// screen if the QR code is no option. Bytes from 248 up are thrown away, so every letter is
/// equally likely.
pub fn new_key() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let limit = (256 / ALPHABET.len() * ALPHABET.len()) as u8;
    let mut key = String::with_capacity(20);
    while key.len() < 20 {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).expect("system random generator");
        key.extend(bytes.iter().filter(|b| **b < limit).map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char));
    }
    key.truncate(20);
    key
}

/// The address the phone gets, key included.
pub fn phone_url(ip: Ipv4Addr, port: u16, key: &str) -> String {
    format!("http://{ip}:{port}/{key}/v1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Health,
    Models,
    Transcribe,
}

/// Which endpoint a request means, if it may use it. The key may lead the path or come as a
/// Bearer token; the health check needs none.
pub fn route(method: &str, path: &str, bearer: Option<&str>, key: &str) -> Result<Route, (u16, &'static str)> {
    let path = path.split('?').next().unwrap_or("");
    // No key configured means nobody gets in - never "any empty key fits".
    let key_ok = |candidate: &str| !key.is_empty() && same(candidate, key);
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if method == "GET" && (segments.is_empty() || segments == ["health"]) {
        return Ok(Route::Health);
    }
    let rest: &[&str] = match segments.split_first() {
        Some((first, rest)) if key_ok(first) => rest,
        _ if bearer.is_some_and(key_ok) => &segments,
        _ => return Err((401, "Missing or wrong access key - copy the address again from Amber on the PC.")),
    };
    // Accept the forms the phone builds from whatever was typed: ".../v1/audio/transcriptions"
    // and ".../audio/transcriptions".
    let rest = rest.strip_prefix(&["v1"][..]).unwrap_or(rest);
    match (method, rest) {
        ("POST", ["audio", "transcriptions"]) => Ok(Route::Transcribe),
        ("GET", ["models"]) => Ok(Route::Models),
        ("GET", []) => Ok(Route::Health),
        _ => Err((404, "Amber's phone API only offers /v1/audio/transcriptions and /v1/models.")),
    }
}

/// Compares without stopping at the first difference, so timing says nothing about the key.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Loopback and the private ranges (including Tailscale's 100.64/10) - the phone at home.
pub fn is_local_peer(ip: IpAddr) -> bool {
    crate::providers::is_private_host(Some(&ip.to_string()))
}

pub struct Part {
    pub name: String,
    pub filename: Option<String>,
    pub data: Vec<u8>,
}

/// Splits a multipart/form-data body into its parts.
pub fn parse_multipart(content_type: &str, body: &[u8]) -> Result<Vec<Part>, String> {
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .map(|b| b.trim_matches('"'))
        .filter(|b| !b.is_empty())
        .ok_or("expected multipart/form-data with a boundary")?;
    let delimiter = format!("--{boundary}").into_bytes();
    let mut parts = Vec::new();
    let mut pos = find(body, &delimiter, 0).ok_or("multipart body without its boundary")? + delimiter.len();
    loop {
        if body[pos..].starts_with(b"--") {
            break; // closing delimiter
        }
        pos += crlf_len(&body[pos..]);
        let head_end = find(body, b"\r\n\r\n", pos).ok_or("multipart part without headers")?;
        let head = String::from_utf8_lossy(&body[pos..head_end]).into_owned();
        let data_start = head_end + 4;
        let next = find(body, &delimiter, data_start).ok_or("multipart body not terminated")?;
        // The CRLF before the delimiter belongs to the delimiter, not the data.
        let data_end = if next >= data_start + 2 && &body[next - 2..next] == b"\r\n" { next - 2 } else { next };
        let disposition = head
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-disposition:"))
            .unwrap_or("");
        let name = param(disposition, "name").ok_or("multipart part without a name")?;
        parts.push(Part { name, filename: param(disposition, "filename"), data: body[data_start..data_end].to_vec() });
        pos = next + delimiter.len();
        if pos >= body.len() {
            break;
        }
    }
    Ok(parts)
}

fn crlf_len(s: &[u8]) -> usize {
    if s.starts_with(b"\r\n") {
        2
    } else {
        0
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

/// `name="file"` out of a Content-Disposition line.
fn param(header: &str, key: &str) -> Option<String> {
    header.split(';').map(str::trim).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().trim_matches('"').to_string())
    })
}

/// The addresses worth showing for the phone: private IPv4 of real adapters, the home
/// network (192.168.x) first. Virtual adapters (WSL, Hyper-V, VirtualBox, VMware) are left
/// out - a phone can never reach them.
pub fn phone_addresses(interfaces: &[(String, IpAddr)]) -> Vec<(String, Ipv4Addr)> {
    const VIRTUAL: [&str; 7] = ["vethernet", "wsl", "hyper-v", "virtualbox", "vmware", "loopback", "docker"];
    let mut found: Vec<(String, Ipv4Addr)> = interfaces
        .iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if v4.is_private() || is_cgnat(*v4) => Some((name.clone(), *v4)),
            _ => None,
        })
        .filter(|(name, _)| {
            let lower = name.to_lowercase();
            !VIRTUAL.iter().any(|v| lower.contains(v))
        })
        .collect();
    found.sort_by_key(|(_, ip)| match ip.octets() {
        [192, 168, ..] => 0,
        [10, ..] => 1,
        [100, ..] => 3, // Tailscale: works from anywhere, but only with Tailscale on the phone
        _ => 2,
    });
    found.dedup_by_key(|(_, ip)| *ip);
    found
}

fn is_cgnat(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..=127).contains(&b)
}

/// What the server needs from the app: the key, the engine to use right now, and a log.
pub struct Hooks {
    pub key: Arc<dyn Fn() -> String + Send + Sync>,
    pub engine: Arc<dyn Fn() -> Result<Box<dyn Transcriber>, String> + Send + Sync>,
    /// What the PC does to its own dictations before inserting them (fillers, the smart
    /// cleanup) - so the phone gets the same without an update of its app.
    pub tidy: Arc<dyn Fn(String) -> String + Send + Sync>,
    /// A finished phone dictation: the text, the audio length and how long it took - for the
    /// PC's history and statistics.
    pub record: Arc<dyn Fn(&str, u64, u64) + Send + Sync>,
    pub log: Arc<dyn Fn(String) + Send + Sync>,
}

pub use crate::http::{Limits, Server};
use crate::http::{error_body, Request};

/// Starts the phone API on [addr].
pub fn start(addr: SocketAddr, hooks: Hooks) -> std::io::Result<Server> {
    start_with(addr, hooks, Limits::default())
}

/// Transcriptions run one at a time - the engine does them in order anyway.
pub fn start_with(addr: SocketAddr, hooks: Hooks, limits: Limits) -> std::io::Result<Server> {
    let hooks = Arc::new(hooks);
    let busy = Arc::new(Mutex::new(()));
    Server::start(addr, limits, is_local_peer, Arc::new(move |req: &mut Request| handle(req, &hooks, &busy)))
}

fn handle(req: &mut Request, hooks: &Hooks, busy: &Mutex<()>) -> (u16, serde_json::Value) {
    let peer = req.peer;
    let bearer = req.header("Authorization").and_then(|v| v.strip_prefix("Bearer ")).map(|v| v.trim().to_string());
    match route(&req.method, &req.target, bearer.as_deref(), &(hooks.key)()) {
        Err((status, message)) => {
            if status == 401 {
                (hooks.log)(format!("phone API: refused {peer} (wrong key)"));
            }
            (status, error_body(message, if status == 401 { "invalid_api_key" } else { "not_found" }))
        }
        Ok(Route::Health) => (200, serde_json::json!({ "status": "ok", "service": "amber-pc" })),
        Ok(Route::Models) => {
            (200, serde_json::json!({ "object": "list", "data": [{ "id": "amber-pc", "object": "model", "owned_by": "amber" }] }))
        }
        Ok(Route::Transcribe) => {
            // Only now, with the key checked, is the upload read.
            let body = match req.body() {
                Ok(b) => b,
                Err((status, message)) => return (status, error_body(message, "invalid_request_error")),
            };
            let content_type = req.header("Content-Type").unwrap_or("").to_string();
            let _one_at_a_time = busy.lock().unwrap_or_else(|e| e.into_inner());
            transcribe(&content_type, &body, peer, hooks)
        }
    }
}

fn transcribe(content_type: &str, body: &[u8], peer: IpAddr, hooks: &Hooks) -> (u16, serde_json::Value) {
    let started = Instant::now();
    let parts = match parse_multipart(content_type, body) {
        Ok(p) => p,
        Err(e) => return (400, error_body(&e, "invalid_request_error")),
    };
    let fields: HashMap<&str, &Part> = parts.iter().map(|p| (p.name.as_str(), p)).collect();
    let Some(file) = fields.get("file") else {
        return (400, error_body("The form has no 'file'.", "invalid_request_error"));
    };
    let field = |name: &str| fields.get(name).map(|p| String::from_utf8_lossy(&p.data).trim().to_string()).filter(|v| !v.is_empty());

    let (samples, rate) = match audio::decode_wav(&file.data) {
        Ok(decoded) => decoded,
        Err(e) => return (400, error_body(&format!("Amber reads WAV only: {e}"), "invalid_request_error")),
    };
    let mono16k = audio::resample(&samples, rate, audio::TARGET_RATE);
    // Silence is a valid question with an empty answer - the phone's connection test sends a
    // quiet tone and counts "nothing recognized" as success.
    if !audio::has_speech(&mono16k, audio::TARGET_RATE) {
        (hooks.log)(format!("phone API: {peer}, {} ms of audio, nothing heard", audio::duration_ms(mono16k.len(), audio::TARGET_RATE)));
        return (200, serde_json::json!({ "text": "" }));
    }
    let wav = audio::encode_wav(&audio::pad_for_engine(&mono16k, audio::TARGET_RATE), audio::TARGET_RATE);
    let options = TranscribeOptions::new(&field("language").unwrap_or_else(|| "auto".into()), &field("prompt").unwrap_or_default());

    let engine = match (hooks.engine)() {
        Ok(e) => e,
        Err(e) => {
            (hooks.log)(format!("phone API: engine unavailable: {e}"));
            return (503, error_body(&e, "server_error"));
        }
    };
    match engine.transcribe(&wav, &options) {
        Ok(raw) => {
            let cleaned = (hooks.tidy)(text::clean(&raw));
            (hooks.record)(&cleaned, audio::duration_ms(mono16k.len(), audio::TARGET_RATE), started.elapsed().as_millis() as u64);
            (hooks.log)(format!(
                "phone API: {peer}, {} words from {} ms of audio in {} ms",
                text::words(&cleaned),
                audio::duration_ms(mono16k.len(), audio::TARGET_RATE),
                started.elapsed().as_millis()
            ));
            (200, serde_json::json!({ "text": cleaned }))
        }
        Err(e) => {
            (hooks.log)(format!("phone API: engine error: {}", e.detail));
            let status = match e.kind {
                ErrorKind::Timeout => 504,
                ErrorKind::RateLimited => 429,
                _ => 502,
            };
            (status, error_body(&e.message, "server_error"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{OpenAiTranscriber, SttError};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    #[test]
    fn keys_are_long_random_and_readable() {
        let (a, b) = (new_key(), new_key());
        assert_eq!(a.len(), 20);
        assert_ne!(a, b);
        assert!(a.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()));
        assert!(!a.contains(['l', 'o', '0', '1', 'i']));
        assert_eq!(phone_url(Ipv4Addr::new(192, 168, 1, 20), 8765, "k"), "http://192.168.1.20:8765/k/v1");
    }

    #[test]
    fn routes_with_the_key_in_the_path_or_as_bearer() {
        let k = "secretkey";
        assert_eq!(route("POST", "/secretkey/v1/audio/transcriptions", None, k), Ok(Route::Transcribe));
        assert_eq!(route("POST", "/secretkey/audio/transcriptions", None, k), Ok(Route::Transcribe));
        assert_eq!(route("GET", "/secretkey/v1/models?x=1", None, k), Ok(Route::Models));
        assert_eq!(route("POST", "/v1/audio/transcriptions", Some("secretkey"), k), Ok(Route::Transcribe));
        assert_eq!(route("GET", "/", None, k), Ok(Route::Health));
        assert_eq!(route("POST", "/wrongkey/v1/audio/transcriptions", None, k).unwrap_err().0, 401);
        assert_eq!(route("POST", "/v1/audio/transcriptions", Some("secretkez"), k).unwrap_err().0, 401);
        assert_eq!(route("POST", "/v1/audio/transcriptions", None, k).unwrap_err().0, 401);
        assert_eq!(route("POST", "/secretkey/v1/chat/completions", None, k).unwrap_err().0, 404);
        assert_eq!(route("GET", "/secretkey/v1/audio/transcriptions", None, k).unwrap_err().0, 404);
    }

    #[test]
    fn only_the_home_network() {
        for ok in ["127.0.0.1", "192.168.178.30", "10.0.0.7", "172.20.1.1", "100.101.1.2", "fd00::5", "::1"] {
            assert!(is_local_peer(ok.parse().unwrap()), "{ok}");
        }
        for bad in ["8.8.8.8", "2a02:8070::1", "100.200.1.1"] {
            assert!(!is_local_peer(bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn multipart_parts() {
        let body = b"--XyZ\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-1\r\n--XyZ\r\nContent-Disposition: form-data; name=\"file\"; filename=\"dictation.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFF\r\n--data\r\n--XyZ--\r\n";
        let parts = parse_multipart("multipart/form-data; boundary=XyZ", body).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "model");
        assert_eq!(parts[0].data, b"whisper-1");
        assert_eq!(parts[1].filename.as_deref(), Some("dictation.wav"));
        assert_eq!(parts[1].data, b"RIFF\r\n--data", "binary data with CRLF and dashes stays whole");
        assert!(parse_multipart("application/json", body).is_err());
        assert!(parse_multipart("multipart/form-data; boundary=XyZ", b"--XyZ\r\nbroken").is_err());
    }

    #[test]
    fn addresses_for_the_phone() {
        let ifs = vec![
            ("vEthernet (WSL)".to_string(), "172.27.240.1".parse().unwrap()),
            ("Ethernet".to_string(), "192.168.178.30".parse().unwrap()),
            ("Tailscale".to_string(), "100.80.1.2".parse().unwrap()),
            ("WLAN".to_string(), "10.0.0.7".parse().unwrap()),
            ("Loopback Pseudo-Interface 1".to_string(), "127.0.0.1".parse().unwrap()),
            ("Ethernet".to_string(), "fe80::1".parse().unwrap()),
            ("Public".to_string(), "85.1.2.3".parse().unwrap()),
        ];
        let got: Vec<String> = phone_addresses(&ifs).iter().map(|(n, ip)| format!("{n} {ip}")).collect();
        assert_eq!(got, ["Ethernet 192.168.178.30", "WLAN 10.0.0.7", "Tailscale 100.80.1.2"]);
    }

    /// Stands in for the engine; remembers what it was given.
    struct Fake(Mutex<Vec<(usize, TranscribeOptions)>>, Result<String, ErrorKind>);
    impl Transcriber for Arc<Fake> {
        fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
            self.0.lock().unwrap().push((wav.len(), options.clone()));
            self.1.clone().map_err(|kind| SttError { kind, message: "engine down".into(), detail: "test".into() })
        }
    }

    fn server(answer: Result<String, ErrorKind>) -> (Server, Arc<Fake>, Arc<Mutex<Vec<String>>>) {
        server_with(answer, Limits::default())
    }

    fn server_with(answer: Result<String, ErrorKind>, limits: Limits) -> (Server, Arc<Fake>, Arc<Mutex<Vec<String>>>) {
        server_at("127.0.0.1:0", answer, limits).unwrap()
    }

    /// For checking that a port is free again: one outside the range the system hands out,
    /// which every other test's servers and connections share (in WSL only 4096 ports) - one
    /// of them may take the freed port before it is checked. Each such test gets its own
    /// [from], or they take each other's freed port.
    fn server_on_quiet_port(from: u16) -> Server {
        (from..from + 500)
            .find_map(|port| server_at(&format!("127.0.0.1:{port}"), Ok(String::new()), Limits::default()).ok())
            .expect("a free port")
            .0
    }

    fn server_at(addr: &str, answer: Result<String, ErrorKind>, limits: Limits) -> std::io::Result<(Server, Arc<Fake>, Arc<Mutex<Vec<String>>>)> {
        let fake = Arc::new(Fake(Mutex::new(Vec::new()), answer));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (f, l) = (fake.clone(), log.clone());
        let hooks = Hooks {
            key: Arc::new(|| "testkey".to_string()),
            engine: Arc::new(move || Ok(Box::new(f.clone()) as Box<dyn Transcriber>)),
            // Stands in for the PC's cleanup: drops the fillers the fake engine heard.
            tidy: Arc::new(|t| text::strip_fillers(&t)),
            record: Arc::new(|_, _, _| {}),
            log: Arc::new(move |m| l.lock().unwrap().push(m)),
        };
        Ok((start_with(addr.parse().unwrap(), hooks, limits)?, fake, log))
    }

    fn speech() -> Vec<u8> {
        let s: Vec<f32> = (0..16_000).map(|i| 0.2 * (i as f32 * 0.09).sin()).collect();
        audio::encode_wav(&s, 16_000)
    }

    /// The phone app's request, made by the same OpenAI client the PC uses for Groq: the
    /// address with the key in the path, no Authorization header.
    #[test]
    fn answers_the_phone_like_an_openai_server() {
        let (srv, fake, log) = server(Ok(" [Musik] Ähm, hallo vom Handy. ".into()));
        let base = format!("http://{}/testkey/v1", srv.addr);
        let client = OpenAiTranscriber::new(&base, "", "whisper-1");
        let text = client.transcribe(&speech(), &TranscribeOptions::new("de", "Amber")).unwrap();
        // Annotation gone, and the PC's cleanup applied before the phone sees the text.
        assert_eq!(text, "Hallo vom Handy.");
        let calls = fake.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1.language.as_deref(), Some("de"));
        assert_eq!(calls[0].1.prompt.as_deref(), Some("Amber"));
        assert!(log.lock().unwrap()[0].contains("3 words"));
        // "Test connection" on the phone: GET models works too.
        assert!(client.check().is_ok());
    }

    #[test]
    fn the_phones_connection_test_gets_an_empty_answer() {
        // A second of a very quiet 220 Hz tone, as ConnectionTester sends it.
        let tone: Vec<f32> = (0..16_000).map(|i| 0.018 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin()).collect();
        let (srv, fake, _) = server(Ok("should not be asked".into()));
        let client = OpenAiTranscriber::new(&format!("http://{}/testkey/v1", srv.addr), "", "m");
        assert_eq!(client.transcribe(&audio::encode_wav(&tone, 16_000), &TranscribeOptions::default()).unwrap(), "");
        assert!(fake.0.lock().unwrap().is_empty());
    }

    #[test]
    fn wrong_key_and_engine_errors() {
        let (srv, fake, log) = server(Err(ErrorKind::Unreachable));
        let wrong = OpenAiTranscriber::new(&format!("http://{}/nope/v1", srv.addr), "", "m");
        assert_eq!(wrong.transcribe(&speech(), &TranscribeOptions::default()).unwrap_err().kind, ErrorKind::Unauthorized);
        assert!(fake.0.lock().unwrap().is_empty());
        assert!(log.lock().unwrap()[0].contains("wrong key"));

        let bearer = OpenAiTranscriber::new(&format!("http://{}/v1", srv.addr), "testkey", "m");
        let e = bearer.transcribe(&speech(), &TranscribeOptions::default()).unwrap_err();
        // A 5xx: the phone takes it as "server unavailable" and may fall back to its own model.
        assert_eq!(e.kind, ErrorKind::Unavailable);
        assert!(e.detail.contains("502") && e.detail.contains("engine down"), "{}", e.detail);
        srv.stop();
    }

    /// Sends raw bytes, returns the status line of the answer (or what went wrong).
    fn raw(addr: SocketAddr, request: &[u8]) -> String {
        let mut s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        let _ = s.write_all(request);
        let mut answer = String::new();
        let _ = s.read_to_string(&mut answer);
        answer.lines().next().unwrap_or("").to_string()
    }

    fn quick() -> Limits {
        // Every test client is 127.0.0.1: the limit per address would be the one tested.
        Limits { io_timeout: Duration::from_millis(300), body_deadline: Duration::from_secs(2), max_per_peer: 8, ..Limits::default() }
    }

    #[test]
    fn an_absurd_content_length_costs_nothing() {
        // Review finding: tiny_http allocated the announced length when dropping a request -
        // one packet without a key aborted Amber.
        let (srv, _, _) = server(Ok(String::new()));
        let huge = b"GET /health HTTP/1.1\r\nContent-Length: 1000000000000000\r\n\r\n";
        assert_eq!(raw(srv.addr, huge), "HTTP/1.1 200 OK");
        let post = b"POST /testkey/v1/audio/transcriptions HTTP/1.1\r\nContent-Length: 1000000000000000\r\n\r\n";
        assert_eq!(raw(srv.addr, post), "HTTP/1.1 413 Payload Too Large");
        assert_eq!(raw(srv.addr, b"GET / HTTP/1.1\r\n\r\n"), "HTTP/1.1 200 OK", "still alive");
    }

    #[test]
    fn half_sent_requests_cannot_block_the_server() {
        // Review finding: three connections announcing a body they never send stopped all
        // workers for good. Now they time out, and others are answered meanwhile.
        let (srv, _, _) = server_with(Ok(String::new()), quick());
        let hanging: Vec<TcpStream> = (0..5)
            .map(|_| {
                let mut s = TcpStream::connect(srv.addr).unwrap();
                s.write_all(b"POST /testkey/v1/audio/transcriptions HTTP/1.1\r\nContent-Length: 5000\r\n\r\n").unwrap();
                s
            })
            .collect();
        assert_eq!(raw(srv.addr, b"GET /health HTTP/1.1\r\n\r\n"), "HTTP/1.1 200 OK");
        for mut s in hanging {
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut answer = String::new();
            let _ = s.read_to_string(&mut answer);
            assert!(answer.starts_with("HTTP/1.1 408"), "{answer}");
        }
        // Wrong key: answered at once, the announced body is never waited for.
        let t0 = Instant::now();
        let wrong = b"POST /nope/v1/audio/transcriptions HTTP/1.1\r\nContent-Length: 5000\r\n\r\n";
        assert_eq!(raw(srv.addr, wrong), "HTTP/1.1 401 Unauthorized");
        assert!(t0.elapsed() < Duration::from_millis(250));
    }

    #[test]
    fn oversized_heads_and_garbage_are_refused() {
        let (srv, _, _) = server(Ok(String::new()));
        let mut big = b"GET /health HTTP/1.1\r\nX: ".to_vec();
        big.extend(std::iter::repeat_n(b'a', 40_000));
        big.extend_from_slice(b"\r\n\r\n");
        assert_eq!(raw(srv.addr, &big), "HTTP/1.1 431 Request Header Fields Too Large");
        assert_eq!(raw(srv.addr, b"\x16\x03\x01 TLS hello\r\n\r\n"), "HTTP/1.1 400 Bad Request");
        let chunked = b"POST /testkey/v1/audio/transcriptions HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert_eq!(raw(srv.addr, chunked), "HTTP/1.1 411 Length Required");
    }

    #[test]
    fn a_part_without_data_does_not_panic() {
        // Review finding: a delimiter right after the part headers sliced backwards.
        let body = b"--B\r\nContent-Disposition: form-data; name=\"file\"\r\n\r\n--B--\r\n";
        let parts = parse_multipart("multipart/form-data; boundary=B", body).unwrap();
        assert_eq!(parts[0].data, b"");
        // And small bodies of every shape: errors, never panics.
        let alphabet: [&[u8]; 7] = [b"--B", b"\r\n", b"name=\"f\"", b"Content-Disposition: form-data; ", b"x", b"--", b"\r\n\r\n"];
        for a in &alphabet {
            for b in &alphabet {
                for c in &alphabet {
                    for d in &alphabet {
                        let body = [*a, *b, *c, *d].concat();
                        let _ = parse_multipart("multipart/form-data; boundary=B", &body);
                    }
                }
            }
        }
    }

    #[test]
    fn a_crashing_request_leaves_the_server_running() {
        let (srv, _, _) = server(Ok(String::new()));
        let mut wav = audio::encode_wav(&[0.1; 4], 16_000);
        wav[24..28].copy_from_slice(&1u32.to_le_bytes()); // 1 Hz: refused, not resampled 16 000-fold
        let form = [
            &b"--B\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\n\r\n"[..],
            &wav,
            b"\r\n--B--\r\n",
        ]
        .concat();
        let mut req = format!(
            "POST /testkey/v1/audio/transcriptions HTTP/1.1\r\nContent-Type: multipart/form-data; boundary=B\r\nContent-Length: {}\r\n\r\n",
            form.len()
        )
        .into_bytes();
        req.extend_from_slice(&form);
        assert_eq!(raw(srv.addr, &req), "HTTP/1.1 400 Bad Request");
        assert_eq!(raw(srv.addr, b"GET / HTTP/1.1\r\n\r\n"), "HTTP/1.1 200 OK");
    }

    #[test]
    fn stopping_frees_the_port_even_with_a_hanging_client() {
        // Review finding: on Windows the old server kept the port after "off".
        let srv = server_on_quiet_port(23_000);
        let addr = srv.addr;
        let mut hanging = TcpStream::connect(addr).unwrap();
        hanging.write_all(b"POST /testkey/v1/audio/transcriptions HTTP/1.1\r\nContent-Length: 99\r\n\r\n").unwrap();
        std::thread::sleep(Duration::from_millis(150));
        let t0 = Instant::now();
        srv.stop();
        assert!(t0.elapsed() < Duration::from_secs(1), "stop waited {:?}", t0.elapsed());
        assert!(TcpListener::bind(addr).is_ok());
    }

    #[test]
    fn a_trickling_head_cannot_hold_a_slot() {
        // Review finding: one header byte now and then kept a connection open for days.
        let limits = Limits { io_timeout: Duration::from_millis(300), head_deadline: Duration::from_millis(800), ..Limits::default() };
        let (srv, _, _) = server_with(Ok(String::new()), limits);
        let mut slow = TcpStream::connect(srv.addr).unwrap();
        slow.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let t0 = Instant::now();
        for b in b"POST /testkey/v1/audio" {
            if slow.write_all(&[*b]).is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut answer = String::new();
        let _ = slow.read_to_string(&mut answer);
        assert!(answer.starts_with("HTTP/1.1 408"), "{answer}");
        assert!(t0.elapsed() < Duration::from_millis(2_500), "closed after {:?}", t0.elapsed());
    }

    #[test]
    fn one_address_cannot_take_every_slot() {
        let limits = Limits { max_per_peer: 2, ..Limits::default() };
        let (srv, _, _) = server_with(Ok(String::new()), limits);
        let _a = TcpStream::connect(srv.addr).unwrap();
        let _b = TcpStream::connect(srv.addr).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut third = TcpStream::connect(srv.addr).unwrap();
        third.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut answer = String::new();
        let _ = third.read_to_string(&mut answer);
        assert!(answer.starts_with("HTTP/1.1 503"), "{answer}");
    }

    #[test]
    fn no_key_means_no_entry() {
        assert_eq!(route("POST", "/v1/audio/transcriptions", Some(""), "").unwrap_err().0, 401);
        assert_eq!(route("POST", "//v1/audio/transcriptions", None, "").unwrap_err().0, 401);
    }

    #[test]
    fn stopping_frees_the_port() {
        let srv = server_on_quiet_port(23_500);
        let addr = srv.addr;
        srv.stop();
        assert!(TcpListener::bind(addr).is_ok());
    }
}
