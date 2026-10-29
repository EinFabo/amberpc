//! The phone API: an OpenAI-compatible transcription endpoint on the home network, so Amber
//! on the phone can use this PC as its "own server" without any change to the phone app.
//!
//! The phone's own-server engine has no API key field, so the access key travels in the
//! path: the phone gets `http://<pc>:<port>/<key>/v1` as its address and appends
//! `/audio/transcriptions` itself. A Bearer header with the key works too, for other clients.
//! Requests from outside the private network are refused, whatever key they carry.

use std::collections::HashMap;
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use crate::audio;
use crate::engine::{ErrorKind, TranscribeOptions, Transcriber};
use crate::text;

pub const DEFAULT_PORT: u16 = 8765;

/// Ten minutes of 16-bit 48 kHz stereo is about 110 MB; nobody dictates that to a phone.
const MAX_BODY: usize = 64 << 20;

/// 20 characters from a 32-letter alphabet without look-alikes: 100 bits, easy to read off a
/// screen if the QR code is no option.
pub fn new_key() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 20];
    getrandom::fill(&mut bytes).expect("system random generator");
    bytes.iter().map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
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
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if method == "GET" && (segments.is_empty() || segments == ["health"]) {
        return Ok(Route::Health);
    }
    let rest: &[&str] = match segments.split_first() {
        Some((first, rest)) if same(first, key) => rest,
        _ if bearer.is_some_and(|b| same(b, key)) => &segments,
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
        let data_end = if next >= 2 && &body[next - 2..next] == b"\r\n" { next - 2 } else { next };
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
    pub log: Arc<dyn Fn(String) + Send + Sync>,
}

/// A running server; dropping it (or [Server::stop]) closes the port.
pub struct Server {
    inner: Arc<tiny_http::Server>,
    threads: Vec<JoinHandle<()>>,
    pub addr: SocketAddr,
}

impl Server {
    /// Listens on [addr] with a few worker threads. Transcriptions run one at a time - the
    /// engine does them in order anyway, and a queue is better than a pile-up.
    pub fn start(addr: SocketAddr, hooks: Hooks) -> std::io::Result<Server> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        let inner = Arc::new(tiny_http::Server::from_listener(listener, None).map_err(std::io::Error::other)?);
        let hooks = Arc::new(hooks);
        let busy = Arc::new(Mutex::new(()));
        let threads = (0..3)
            .map(|_| {
                let (server, hooks, busy) = (inner.clone(), hooks.clone(), busy.clone());
                std::thread::spawn(move || {
                    // Ends when the server is unblocked by stop().
                    while let Ok(request) = server.recv() {
                        handle(request, &hooks, &busy);
                    }
                })
            })
            .collect();
        Ok(Server { inner, threads, addr })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        for _ in 0..self.threads.len() {
            self.inner.unblock();
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn json_response(status: u16, body: serde_json::Value) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("static header");
    tiny_http::Response::from_data(body.to_string().into_bytes()).with_status_code(status).with_header(header)
}

/// OpenAI's error shape, which the phone (and every other client) knows how to show.
fn error_body(message: &str, kind: &str) -> serde_json::Value {
    serde_json::json!({ "error": { "message": message, "type": kind } })
}

fn handle(mut request: tiny_http::Request, hooks: &Hooks, busy: &Mutex<()>) {
    let peer = request.remote_addr().map(|a| a.ip());
    if !peer.is_some_and(is_local_peer) {
        let _ = request.respond(json_response(403, error_body("Only devices in your own network may use this PC.", "forbidden")));
        return;
    }
    let peer = peer.expect("checked above");
    let header = |name: &str| {
        request.headers().iter().find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name)).map(|h| h.value.as_str().to_string())
    };
    let bearer = header("Authorization").and_then(|v| v.strip_prefix("Bearer ").map(str::trim).map(str::to_string));
    let content_type = header("Content-Type").unwrap_or_default();
    let method = request.method().as_str().to_uppercase();

    let response = match route(&method, request.url(), bearer.as_deref(), &(hooks.key)()) {
        Err((status, message)) => {
            if status == 401 {
                (hooks.log)(format!("phone API: refused {peer} (wrong key)"));
            }
            json_response(status, error_body(message, if status == 401 { "invalid_api_key" } else { "not_found" }))
        }
        Ok(Route::Health) => json_response(200, serde_json::json!({ "status": "ok", "service": "amber-pc" })),
        Ok(Route::Models) => json_response(
            200,
            serde_json::json!({ "object": "list", "data": [{ "id": "amber-pc", "object": "model", "owned_by": "amber" }] }),
        ),
        Ok(Route::Transcribe) => {
            let mut body = Vec::new();
            let read = request.as_reader().take(MAX_BODY as u64 + 1).read_to_end(&mut body);
            if read.is_err() {
                json_response(400, error_body("The upload broke off.", "invalid_request_error"))
            } else if body.len() > MAX_BODY {
                json_response(413, error_body("The recording is too large.", "invalid_request_error"))
            } else {
                let _one_at_a_time = busy.lock().unwrap_or_else(|e| e.into_inner());
                transcribe(&content_type, &body, peer, hooks)
            }
        }
    };
    let _ = request.respond(response);
}

fn transcribe(content_type: &str, body: &[u8], peer: IpAddr, hooks: &Hooks) -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let started = Instant::now();
    let parts = match parse_multipart(content_type, body) {
        Ok(p) => p,
        Err(e) => return json_response(400, error_body(&e, "invalid_request_error")),
    };
    let fields: HashMap<&str, &Part> = parts.iter().map(|p| (p.name.as_str(), p)).collect();
    let Some(file) = fields.get("file") else {
        return json_response(400, error_body("The form has no 'file'.", "invalid_request_error"));
    };
    let field = |name: &str| fields.get(name).map(|p| String::from_utf8_lossy(&p.data).trim().to_string()).filter(|v| !v.is_empty());

    let (samples, rate) = match audio::decode_wav(&file.data) {
        Ok(decoded) => decoded,
        Err(e) => return json_response(400, error_body(&format!("Amber reads WAV only: {e}"), "invalid_request_error")),
    };
    let mono16k = audio::resample(&samples, rate, audio::TARGET_RATE);
    // Silence is a valid question with an empty answer - the phone's connection test sends a
    // quiet tone and counts "nothing recognized" as success.
    if !audio::has_speech(&mono16k, audio::TARGET_RATE) {
        (hooks.log)(format!("phone API: {peer}, {} ms of audio, nothing heard", audio::duration_ms(mono16k.len(), audio::TARGET_RATE)));
        return json_response(200, serde_json::json!({ "text": "" }));
    }
    let wav = audio::encode_wav(&audio::pad_for_engine(&mono16k, audio::TARGET_RATE), audio::TARGET_RATE);
    let options = TranscribeOptions::new(&field("language").unwrap_or_else(|| "auto".into()), &field("prompt").unwrap_or_default());

    let engine = match (hooks.engine)() {
        Ok(e) => e,
        Err(e) => {
            (hooks.log)(format!("phone API: engine unavailable: {e}"));
            return json_response(503, error_body(&e, "server_error"));
        }
    };
    match engine.transcribe(&wav, &options) {
        Ok(raw) => {
            let cleaned = text::clean(&raw);
            (hooks.log)(format!(
                "phone API: {peer}, {} words from {} ms of audio in {} ms",
                text::words(&cleaned),
                audio::duration_ms(mono16k.len(), audio::TARGET_RATE),
                started.elapsed().as_millis()
            ));
            json_response(200, serde_json::json!({ "text": cleaned }))
        }
        Err(e) => {
            (hooks.log)(format!("phone API: engine error: {}", e.detail));
            let status = match e.kind {
                ErrorKind::Timeout => 504,
                ErrorKind::RateLimited => 429,
                _ => 502,
            };
            json_response(status, error_body(&e.message, "server_error"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{OpenAiTranscriber, SttError};

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
        let fake = Arc::new(Fake(Mutex::new(Vec::new()), answer));
        let log = Arc::new(Mutex::new(Vec::new()));
        let (f, l) = (fake.clone(), log.clone());
        let hooks = Hooks {
            key: Arc::new(|| "testkey".to_string()),
            engine: Arc::new(move || Ok(Box::new(f.clone()) as Box<dyn Transcriber>)),
            log: Arc::new(move |m| l.lock().unwrap().push(m)),
        };
        (Server::start("127.0.0.1:0".parse().unwrap(), hooks).unwrap(), fake, log)
    }

    fn speech() -> Vec<u8> {
        let s: Vec<f32> = (0..16_000).map(|i| 0.2 * (i as f32 * 0.09).sin()).collect();
        audio::encode_wav(&s, 16_000)
    }

    /// The phone app's request, made by the same OpenAI client the PC uses for Groq: the
    /// address with the key in the path, no Authorization header.
    #[test]
    fn answers_the_phone_like_an_openai_server() {
        let (srv, fake, log) = server(Ok(" [Musik] Hallo vom Handy. ".into()));
        let base = format!("http://{}/testkey/v1", srv.addr);
        let client = OpenAiTranscriber::new(&base, "", "whisper-1");
        let text = client.transcribe(&speech(), &TranscribeOptions::new("de", "Amber")).unwrap();
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
        assert_eq!(e.kind, ErrorKind::Server);
        assert!(e.detail.contains("502") && e.detail.contains("engine down"), "{}", e.detail);
        srv.stop();
    }

    #[test]
    fn stopping_frees_the_port() {
        let (srv, _, _) = server(Ok(String::new()));
        let addr = srv.addr;
        srv.stop();
        assert!(TcpListener::bind(addr).is_ok());
    }
}
