//! Audio in, text out. Both engines speak HTTP: a server through the OpenAI audio endpoint,
//! the local engine through whisper.cpp's own server on 127.0.0.1. The app only sees
//! [Transcriber].

use std::time::Duration;

use reqwest::blocking::{multipart, Client};
use serde::Deserialize;

#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// ISO 639-1 code, or None to let the engine detect the language.
    pub language: Option<String>,
    /// Names and jargon the engine should expect.
    pub prompt: Option<String>,
}

impl TranscribeOptions {
    pub fn new(language: &str, vocabulary: &str) -> Self {
        let language = (language != "auto" && !language.is_empty()).then(|| language.to_string());
        let prompt = Some(vocabulary.trim().to_string()).filter(|p| !p.is_empty());
        TranscribeOptions { language, prompt }
    }
}

pub trait Transcriber: Send + Sync {
    fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError>;

    /// The text and, where the engine says so, the language it detected (Whisper's name,
    /// "german") - for keeping one language across the parts of a long dictation.
    fn transcribe_detecting(&self, wav: &[u8], options: &TranscribeOptions) -> Result<(String, Option<String>), SttError> {
        self.transcribe(wav, options).map(|t| (t, None))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Unauthorized,
    RateLimited,
    TooLarge,
    Unreachable,
    Timeout,
    /// The server answered 5xx: down or overloaded - like no connection, worth a fallback.
    Unavailable,
    Server,
    BadResponse,
}

impl ErrorKind {
    /// Failures another engine could fix: no connection, a timeout, a server that is down. A
    /// refused key or a bad request needs the user, not a detour.
    pub fn service_unavailable(self) -> bool {
        matches!(self, ErrorKind::Unreachable | ErrorKind::Timeout | ErrorKind::Unavailable)
    }
}

/// [message] is for the user, [detail] for the log.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message} ({detail})")]
pub struct SttError {
    pub kind: ErrorKind,
    pub message: String,
    pub detail: String,
}

impl SttError {
    fn new(kind: ErrorKind, message: impl Into<String>, detail: impl Into<String>) -> Self {
        SttError { kind, message: message.into(), detail: detail.into() }
    }
}

fn client(timeout: Duration) -> Client {
    Client::builder()
        .timeout(timeout)
        .connect_timeout(Duration::from_secs(8))
        .user_agent(concat!("AmberPC/", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("http client")
}

/// Groq, OpenAI or an own server: `POST {base}/audio/transcriptions`.
pub struct OpenAiTranscriber {
    base_url: String,
    api_key: String,
    model: String,
    http: Client,
}

impl OpenAiTranscriber {
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        OpenAiTranscriber {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.trim().to_string(),
            model: model.to_string(),
            http: client(Duration::from_secs(90)),
        }
    }

    /// `GET {base}/models` answers without spending anything: is the address right and the
    /// key accepted?
    pub fn check(&self) -> Result<(), SttError> {
        let mut req = self.http.get(format!("{}/models", self.base_url)).timeout(Duration::from_secs(15));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send().map_err(network_error)?;
        let status = resp.status().as_u16();
        let body = resp.text().unwrap_or_default();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            Err(status_error(status, &body))
        }
    }
}

impl Transcriber for OpenAiTranscriber {
    fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
        let mut form = multipart::Form::new()
            .part("file", wav_part(wav))
            .text("model", self.model.clone())
            .text("response_format", "json")
            .text("temperature", "0");
        if let Some(lang) = &options.language {
            form = form.text("language", lang.clone());
        }
        if let Some(prompt) = &options.prompt {
            form = form.text("prompt", prompt.clone());
        }
        let mut req = self.http.post(format!("{}/audio/transcriptions", self.base_url)).multipart(form);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        read_text(req.send().map_err(network_error)?)
    }
}

/// whisper.cpp's `whisper-server`: `POST {base}/inference`.
pub struct WhisperServerTranscriber {
    base_url: String,
    /// For models tuned to a short encoder context: the slack beyond the audio.
    context_slack: Option<u32>,
    http: Client,
}

impl WhisperServerTranscriber {
    /// [context_slack]: `Some` for a short-context (ACFT) model - the encoder then stops
    /// where the speech stops instead of working through 30 s.
    pub fn new(base_url: &str, context_slack: Option<u32>) -> Self {
        // Generous: a long dictation on a CPU takes a while, and nothing is billed per second.
        WhisperServerTranscriber {
            base_url: base_url.trim_end_matches('/').to_string(),
            context_slack,
            http: client(Duration::from_secs(300)),
        }
    }
}

impl Transcriber for WhisperServerTranscriber {
    fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String, SttError> {
        self.request(wav, options, "json").map(|(text, _)| text)
    }

    fn transcribe_detecting(&self, wav: &[u8], options: &TranscribeOptions) -> Result<(String, Option<String>), SttError> {
        self.request(wav, options, "verbose_json")
    }
}

impl WhisperServerTranscriber {
    fn request(&self, wav: &[u8], options: &TranscribeOptions, format: &str) -> Result<(String, Option<String>), SttError> {
        let mut form = multipart::Form::new()
            .part("file", wav_part(wav))
            .text("response_format", format.to_string())
            .text("temperature", "0.0")
            // As Amber for Android decodes (and measured its speed): no timestamp tokens.
            .text("no_timestamps", "true")
            .text("language", options.language.clone().unwrap_or_else(|| "auto".into()));
        if format == "verbose_json" {
            // Only the detected language is wanted; the probabilities for every language cost
            // another encoder pass.
            form = form.text("no_language_probabilities", "true");
        }
        if let Some(prompt) = &options.prompt {
            form = form.text("prompt", prompt.clone());
        }
        let mut full_window = false;
        if let Some(slack) = self.context_slack {
            // 16-bit mono after the 44-byte header Amber writes.
            let ctx = crate::local::audio_ctx_for(wav.len().saturating_sub(44) / 2, slack);
            if ctx > 0 {
                form = form.text("audio_ctx", ctx.to_string());
            }
            full_window = ctx == 0;
        }
        let resp = self.http.post(format!("{}/inference", self.base_url)).multipart(form).send().map_err(network_error)?;
        let status = resp.status().as_u16();
        let body = resp.text().map_err(network_error)?;
        if full_window {
            // whisper.cpp detects the language with the previous request's context; after a
            // full window that would cost the next short dictation a 30 s encoder pass.
            let (http, base) = (self.http.clone(), self.base_url.clone());
            std::thread::spawn(move || reset_context(&http, &base));
        }
        if !(200..300).contains(&status) {
            return Err(status_error(status, &body));
        }
        let text = parse_text(&body)?;
        let language = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v.get("language").and_then(|l| l.as_str()).map(str::to_string));
        Ok((text, language))
    }
}

impl WhisperServerTranscriber {
    /// Leaves the server's state at a short context: after loading a tuned model, and after a
    /// dictation that needed the full window.
    pub fn reset_context(&self) {
        reset_context(&self.http, &self.base_url);
    }
}

/// A second of silence with a fixed language and the smallest context - whisper.cpp keeps
/// that context for its next language detection. Also warms the GPU up.
fn reset_context(http: &Client, base_url: &str) {
    let silence = crate::audio::encode_wav(&vec![0.0; 19_200], crate::audio::TARGET_RATE);
    let form = multipart::Form::new()
        .part("file", wav_part(&silence))
        .text("response_format", "json")
        .text("no_timestamps", "true")
        .text("language", "en")
        .text("audio_ctx", "256");
    let _ = http.post(format!("{base_url}/inference")).multipart(form).send();
}

/// Whether a local server (whisper.cpp, llama.cpp) is up and done loading. Both answer 503
/// while their model loads - whisper.cpp even opens its port before that, so an open port
/// alone says nothing. Any other answer, even a 404 for a missing /health, means ready.
pub fn local_server_ready(base_url: &str) -> bool {
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
        .and_then(|c| c.get(url).send().ok())
        .is_some_and(|r| r.status().as_u16() != 503)
}

fn wav_part(wav: &[u8]) -> multipart::Part {
    multipart::Part::bytes(wav.to_vec()).file_name("dictation.wav").mime_str("audio/wav").expect("static mime")
}

#[derive(Deserialize)]
struct TextResponse {
    text: Option<String>,
    error: Option<serde_json::Value>,
}

fn read_text(resp: reqwest::blocking::Response) -> Result<String, SttError> {
    let status = resp.status().as_u16();
    let body = resp.text().map_err(network_error)?;
    if !(200..300).contains(&status) {
        return Err(status_error(status, &body));
    }
    parse_text(&body)
}

pub fn parse_text(body: &str) -> Result<String, SttError> {
    let parsed: TextResponse = serde_json::from_str(body)
        .map_err(|e| SttError::new(ErrorKind::BadResponse, "The engine sent an answer Amber does not understand.", format!("{e}: {}", clip(body))))?;
    if let Some(err) = parsed.error {
        return Err(SttError::new(ErrorKind::Server, "The engine reported an error.", clip(&err.to_string())));
    }
    parsed.text.ok_or_else(|| SttError::new(ErrorKind::BadResponse, "The engine sent no text.", clip(body)))
}

pub fn status_error(status: u16, body: &str) -> SttError {
    // OpenAI-style errors carry their reason in error.message.
    let reason = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| clip(body));
    let detail = format!("HTTP {status}: {reason}");
    match status {
        401 | 403 => SttError::new(ErrorKind::Unauthorized, "The API key was rejected - check it in the settings.", detail),
        404 => SttError::new(ErrorKind::Server, "The server does not know this address or model.", detail),
        413 => SttError::new(ErrorKind::TooLarge, "The recording is too long for this server.", detail),
        429 => SttError::new(ErrorKind::RateLimited, "Rate limit reached - try again in a moment.", detail),
        500..=599 => SttError::new(ErrorKind::Unavailable, format!("The server had a problem (HTTP {status})."), detail),
        _ => SttError::new(ErrorKind::Server, format!("The server had a problem (HTTP {status})."), detail),
    }
}

pub(crate) fn network_error(e: reqwest::Error) -> SttError {
    if e.is_timeout() {
        SttError::new(ErrorKind::Timeout, "The engine took too long to answer.", e.to_string())
    } else if e.is_connect() {
        SttError::new(ErrorKind::Unreachable, "The engine is not reachable - check the connection.", e.to_string())
    } else {
        SttError::new(ErrorKind::Unreachable, "The connection to the engine failed.", e.to_string())
    }
}

fn clip(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 300 {
        format!("{}…", s.chars().take(300).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
pub(crate) mod testserver {
    //! A one-shot HTTP server on 127.0.0.1 for tests: answers each request with the next
    //! canned response and hands back what it received.

    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    pub struct Received {
        pub head: String,
        pub body: Vec<u8>,
    }

    pub fn serve(responses: Vec<(u16, &'static str, Vec<u8>)>) -> (String, mpsc::Receiver<Received>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for (status, content_type, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut head = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                        break;
                    }
                    head.push_str(&line);
                }
                let len = head
                    .lines()
                    .find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                    .unwrap_or(0);
                let mut req_body = vec![0; len];
                reader.read_exact(&mut req_body).unwrap();
                let mut stream = reader.into_inner();
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
                tx.send(Received { head, body: req_body }).unwrap();
            }
        });
        (url, rx)
    }
}

#[cfg(test)]
mod tests {
    use super::testserver::serve;
    use super::*;

    fn json(status: u16, body: &str) -> (u16, &'static str, Vec<u8>) {
        (status, "application/json", body.as_bytes().to_vec())
    }

    #[test]
    fn openai_request_and_answer() {
        let (url, rx) = serve(vec![json(200, r#"{"text":" Hallo Welt."}"#)]);
        let t = OpenAiTranscriber::new(&format!("{url}/v1/"), " sk-test ", "whisper-large-v3-turbo");
        let text = t.transcribe(b"RIFFdata", &TranscribeOptions::new("de", "Tauri")).unwrap();
        assert_eq!(text, " Hallo Welt.");

        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /v1/audio/transcriptions "));
        assert!(req.head.to_lowercase().contains("authorization: bearer sk-test\r\n"));
        let body = String::from_utf8_lossy(&req.body);
        for needle in ["name=\"model\"\r\n\r\nwhisper-large-v3-turbo", "name=\"language\"\r\n\r\nde", "name=\"prompt\"\r\n\r\nTauri",
                       "filename=\"dictation.wav\"", "RIFFdata"] {
            assert!(body.contains(needle), "missing {needle}");
        }
    }

    #[test]
    fn auto_language_and_empty_vocabulary_are_left_out() {
        let (url, rx) = serve(vec![json(200, r#"{"text":"x"}"#)]);
        OpenAiTranscriber::new(&url, "", "m").transcribe(b"", &TranscribeOptions::new("auto", "  ")).unwrap();
        let req = rx.recv().unwrap();
        let body = String::from_utf8_lossy(&req.body);
        assert!(!body.contains("name=\"language\""));
        assert!(!body.contains("name=\"prompt\""));
        assert!(!req.head.to_lowercase().contains("authorization"));
    }

    #[test]
    fn whisper_server_request() {
        let (url, rx) = serve(vec![json(200, r#"{"text":" Test.\n"}"#)]);
        let text = WhisperServerTranscriber::new(&url, None).transcribe(b"wav", &TranscribeOptions::new("auto", "")).unwrap();
        assert_eq!(text, " Test.\n");
        let req = rx.recv().unwrap();
        assert!(req.head.starts_with("POST /inference "));
        let body = String::from_utf8_lossy(&req.body).to_string();
        assert!(body.contains("name=\"language\"\r\n\r\nauto"));
        assert!(!body.contains("audio_ctx"), "a stock model gets the full window");
    }

    #[test]
    fn a_tuned_model_gets_a_short_encoder_context() {
        let (url, rx) = serve(vec![json(200, r#"{"text":"x"}"#)]);
        // 5 s of 16 kHz audio.
        let wav = crate::audio::encode_wav(&vec![0.1; 80_000], 16_000);
        WhisperServerTranscriber::new(&url, Some(64)).transcribe(&wav, &TranscribeOptions::default()).unwrap();
        let body = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(body.contains("name=\"audio_ctx\"\r\n\r\n314"), "250 positions of speech + 64");
    }

    #[test]
    fn errors_become_plain_words() {
        let (url, _rx) = serve(vec![
            json(401, r#"{"error":{"message":"Invalid API Key","type":"invalid_request_error"}}"#),
            json(429, "slow down"),
            json(500, "boom"),
            json(200, "<html>"),
            json(200, r#"{"error":"model not loaded"}"#),
        ]);
        let t = OpenAiTranscriber::new(&url, "k", "m");
        let o = TranscribeOptions::default();
        let e = t.transcribe(b"", &o).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unauthorized);
        assert!(e.detail.contains("Invalid API Key"));
        assert_eq!(t.transcribe(b"", &o).unwrap_err().kind, ErrorKind::RateLimited);
        let down = t.transcribe(b"", &o).unwrap_err().kind;
        assert_eq!(down, ErrorKind::Unavailable);
        assert!(down.service_unavailable() && !ErrorKind::Unauthorized.service_unavailable());
        assert_eq!(t.transcribe(b"", &o).unwrap_err().kind, ErrorKind::BadResponse);
        assert_eq!(t.transcribe(b"", &o).unwrap_err().kind, ErrorKind::Server);
    }

    #[test]
    fn unreachable_server() {
        // Bind and drop: the port is closed.
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let e = WhisperServerTranscriber::new(&format!("http://127.0.0.1:{port}"), None).transcribe(b"", &TranscribeOptions::default()).unwrap_err();
        assert_eq!(e.kind, ErrorKind::Unreachable);
    }

    #[test]
    fn a_full_window_is_followed_by_a_short_reset() {
        let (url, rx) = serve(vec![
            (200, "application/json", br#"{"text":"long"}"#.to_vec()),
            (200, "application/json", br#"{"text":""}"#.to_vec()),
        ]);
        // 30 s: past the short-context range.
        let wav = crate::audio::encode_wav(&vec![0.1; 16_000 * 30], 16_000);
        WhisperServerTranscriber::new(&url, Some(128)).transcribe(&wav, &TranscribeOptions::default()).unwrap();
        let first = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(!first.contains("audio_ctx") && first.contains("name=\"no_timestamps\"\r\n\r\ntrue"));
        let reset = String::from_utf8_lossy(&rx.recv_timeout(Duration::from_secs(5)).unwrap().body).to_string();
        assert!(reset.contains("name=\"audio_ctx\"\r\n\r\n256") && reset.contains("name=\"language\"\r\n\r\nen"));
    }

    #[test]
    fn the_detected_language_comes_along() {
        let (url, rx) = serve(vec![json(200, r#"{"task":"transcribe","language":"german","text":" Hallo.","segments":[]}"#)]);
        let (text, lang) = WhisperServerTranscriber::new(&url, None).transcribe_detecting(b"wav", &TranscribeOptions::default()).unwrap();
        assert_eq!((text.as_str(), lang.as_deref()), (" Hallo.", Some("german")));
        let body = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(body.contains("verbose_json") && body.contains("name=\"no_language_probabilities\"\r\n\r\ntrue"));
    }

    #[test]
    fn local_server_readiness() {
        let (url, _rx) = serve(vec![json(503, r#"{"status":"loading model"}"#), json(200, "{}"), json(404, "{}")]);
        assert!(!local_server_ready(&url), "503 is still loading");
        assert!(local_server_ready(&url));
        assert!(local_server_ready(&url), "no /health at all, but answering");
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(!local_server_ready(&format!("http://127.0.0.1:{port}")), "not listening");
    }

    #[test]
    fn key_check() {
        let (url, rx) = serve(vec![json(200, r#"{"data":[]}"#), json(401, "{}")]);
        let t = OpenAiTranscriber::new(&url, "k", "m");
        assert!(t.check().is_ok());
        assert!(rx.recv().unwrap().head.starts_with("GET /models "));
        assert_eq!(t.check().unwrap_err().kind, ErrorKind::Unauthorized);
    }
}
