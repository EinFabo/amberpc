//! Audio in, text out. Both engines speak HTTP: a server through the OpenAI audio endpoint,
//! the local engine through whisper.cpp's own server on 127.0.0.1. The app only sees
//! [Transcriber].

use std::time::Duration;

use reqwest::blocking::{multipart, Client};
use serde::Deserialize;

#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// ISO 639-1 code, several of them comma-separated ("de,en": detect among these), or None
    /// to let the engine detect any language.
    pub language: Option<String>,
    /// Names and jargon the engine should expect.
    pub prompt: Option<String>,
}

impl TranscribeOptions {
    /// [language]: "auto", a code ("de", "de-DE") or several ("de, en-US").
    pub fn new(language: &str, vocabulary: &str) -> Self {
        let mut codes: Vec<String> = Vec::new();
        for code in language.split(',').map(|c| c.trim().split(['-', '_']).next().unwrap_or("").to_lowercase()) {
            if !code.is_empty() && code != "auto" && !codes.contains(&code) {
                codes.push(code);
            }
        }
        let language = (!codes.is_empty()).then(|| codes.join(","));
        let prompt = Some(vocabulary.trim().to_string()).filter(|p| !p.is_empty());
        TranscribeOptions { language, prompt }
    }

    /// The languages to choose among - empty for any language.
    pub fn among(&self) -> Vec<String> {
        self.language.as_deref().map(|l| l.split(',').map(str::to_string).collect()).unwrap_or_default()
    }

    /// The one language, when exactly one is set.
    pub fn fixed(&self) -> Option<&str> {
        self.language.as_deref().filter(|l| !l.contains(','))
    }
}

/// Up to this much audio, a dictation is short: whisper's language guess on a second or so is
/// weak, and a wrong guess costs much more one way than the other. Measured on whisper-server
/// with Large v3 Turbo (ACFT), German and English phrases of 0.5-1.1 s (TTS at 1.5-2x speed,
/// Fabi's failures were 0.9-1.4 s): German was taken for English with p(en) 0.29-0.47 and
/// p(de) 0.03-0.47, and run again in English it came back translated ("Okay, danke." ->
/// "Okay, thank you.", "Ich hab kein Wasser." -> "I have no water."); short English run in
/// German stayed English ("See you later.", "No problem.", "Sounds good."). From about 2 s on
/// every phrase was detected clearly (the right language at 0.87 or more, mostly 0.99).
pub const SHORT_DICTATION_MS: u64 = 3_000;

/// In a short dictation another language than the first chosen needs at least this
/// probability ("more likely than not")...
pub const CLEAR_LANGUAGE: f64 = 0.5;

/// ...and this many times the first one's. English phrases cleared both at 0.64-1.0; the
/// German ones taken for English stayed below 0.47.
pub const CLEAR_MARGIN: f64 = 2.0;

/// Of [allowed], the language to recognize [audio_ms] of speech in, by whisper.cpp's
/// [probabilities] (`{"de": 0.2, ...}`): the likeliest of them - in a short dictation the first
/// chosen unless another is clearly likelier ([SHORT_DICTATION_MS]). The first, too, when it
/// heard none of them or two equally. Short German speech is easily taken for Turkish; among
/// German and English only, it can no longer be.
pub fn pick_language(probabilities: &serde_json::Value, allowed: &[String], audio_ms: u64) -> String {
    let p = |code: &str| probabilities.get(code).and_then(|v| v.as_f64()).unwrap_or(0.0);
    let first = allowed.first().cloned().unwrap_or_default();
    let mut best = first.clone();
    for code in allowed {
        if p(code) > p(&best) {
            best = code.clone();
        }
    }
    let unclear = p(&best) < CLEAR_LANGUAGE || p(&best) < CLEAR_MARGIN * p(&first);
    let picked = if audio_ms <= SHORT_DICTATION_MS && unclear { first } else { best };
    if allowed.len() > 1 {
        note(language_note(probabilities, allowed, audio_ms, &picked));
    }
    picked
}

/// The log line for a pick: length, the chosen languages' probabilities, the pick - never the
/// text.
pub fn language_note(probabilities: &serde_json::Value, allowed: &[String], audio_ms: u64, picked: &str) -> String {
    let each: Vec<String> = allowed.iter().map(|c| format!("{c} {:.2}", probabilities.get(c).and_then(|v| v.as_f64()).unwrap_or(0.0))).collect();
    format!("language: {:.1} s of audio, among {}: {} -> {picked}", audio_ms as f64 / 1000.0, allowed.join(","), each.join(", "))
}

/// The language a cloud service is sent: the one chosen, or for a short dictation the first
/// of several - it would guess freely, and a short German phrase guessed as English comes
/// back translated. A long one it detects itself.
pub fn cloud_language(options: &TranscribeOptions, audio_ms: u64) -> Option<String> {
    match options.fixed() {
        Some(one) => Some(one.to_string()),
        None if audio_ms <= SHORT_DICTATION_MS => options.among().into_iter().next(),
        None => None,
    }
}

static LANGUAGE_LOG: std::sync::OnceLock<Box<dyn Fn(String) + Send + Sync>> = std::sync::OnceLock::new();

/// Where the language picks are logged - the desktop's log, the server's.
pub fn log_languages_to(log: impl Fn(String) + Send + Sync + 'static) {
    let _ = LANGUAGE_LOG.set(Box::new(log));
}

fn note(line: String) {
    if let Some(log) = LANGUAGE_LOG.get() {
        log(line);
    }
}

/// Milliseconds of audio in a 16 kHz 16-bit mono WAV as Amber writes it (44-byte header).
pub fn wav_ms(wav: &[u8]) -> u64 {
    (wav.len().saturating_sub(44) / 2) as u64 * 1000 / 16_000
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
    pub fn new(kind: ErrorKind, message: impl Into<String>, detail: impl Into<String>) -> Self {
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
        // A list ("de,en") is ours: the services detect by themselves then - but not in a
        // short dictation.
        if let Some(lang) = cloud_language(options, wav_ms(wav)) {
            form = form.text("language", lang.to_string());
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
        let among = options.among();
        if among.len() < 2 {
            return self.run(wav, options, format, false).map(|(text, language, _)| (text, language));
        }
        // Several languages: one run with "auto", and the probabilities whisper-server works out
        // after it - on this dictation's own context (its detection before the run uses the
        // previous request's). Again in the likeliest allowed language when Whisper chose
        // another: short German speech is easily taken for Turkish or English.
        let any = TranscribeOptions { language: None, prompt: options.prompt.clone() };
        let (text, language, probabilities) = self.run(wav, &any, "verbose_json", true)?;
        let picked = pick_language(&probabilities, &among, wav_ms(wav));
        if language.as_deref().and_then(crate::chunking::language_code) == Some(picked.as_str()) {
            return Ok((text, language));
        }
        let fixed = TranscribeOptions { language: Some(picked), prompt: options.prompt.clone() };
        self.run(wav, &fixed, format, false).map(|(text, language, _)| (text, language))
    }

    /// One inference: the text, the language whisper-server names, and - with
    /// [probabilities] - its language probabilities (`{"de": 0.6, ...}`).
    fn run(&self, wav: &[u8], options: &TranscribeOptions, format: &str, probabilities: bool) -> Result<(String, Option<String>, serde_json::Value), SttError> {
        let mut form = multipart::Form::new()
            .part("file", wav_part(wav))
            .text("response_format", format.to_string())
            .text("temperature", "0.0")
            // As Amber for Android decodes (and measured its speed): no timestamp tokens.
            .text("no_timestamps", "true")
            .text("language", options.language.clone().unwrap_or_else(|| "auto".into()));
        if format == "verbose_json" && !probabilities {
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
        let json = serde_json::from_str::<serde_json::Value>(&body).unwrap_or_default();
        let language = json.get("language").and_then(|l| l.as_str()).map(str::to_string);
        Ok((text, language, json["language_probabilities"].clone()))
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

/// Whether the server at [base_url] says it is ready: `GET /health` answering 200 with
/// `{"status": "ok"}` (llama-server, whisper-server). Anything else - loading, an error, some
/// other program that took the port - is not.
pub fn health_ok(base_url: &str) -> bool {
    health(base_url).is_some_and(|v| v["status"] == "ok")
}

/// What `GET /health` at [base_url] answered with 200, if anything.
pub fn health(base_url: &str) -> Option<serde_json::Value> {
    let url = format!("{}/health", base_url.trim_end_matches('/'));
    Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()
        .and_then(|c| c.get(url).send().ok())
        .filter(|r| r.status().as_u16() == 200)
        .and_then(|r| r.json::<serde_json::Value>().ok())
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
        // Not the body itself in the log: from a server answering in plain text, it is the
        // dictated text.
        .map_err(|e| SttError::new(ErrorKind::BadResponse, "The engine sent an answer Amber does not understand.", format!("{e} ({} bytes)", body.len())))?;
    if let Some(err) = parsed.error {
        return Err(SttError::new(ErrorKind::Server, "The engine reported an error.", clip(&err.to_string())));
    }
    parsed.text.ok_or_else(|| SttError::new(ErrorKind::BadResponse, "The engine sent no text.", format!("{} bytes without a text field", body.len())))
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
    fn languages_read_as_a_choice() {
        let o = TranscribeOptions::new("de-DE, en_US", "");
        assert_eq!(o.language.as_deref(), Some("de,en"));
        assert_eq!((o.among(), o.fixed()), (vec!["de".to_string(), "en".to_string()], None));
        let one = TranscribeOptions::new("de", "");
        assert_eq!((one.among(), one.fixed()), (vec!["de".to_string()], Some("de")));
        let auto = TranscribeOptions::new("auto", "");
        assert_eq!((auto.language.clone(), auto.among()), (None, Vec::<String>::new()));
        assert_eq!(TranscribeOptions::new("de,de, ,auto", "").language.as_deref(), Some("de"));
    }

    #[test]
    fn the_likeliest_allowed_language_wins() {
        // A long dictation: the likeliest, whatever the margin.
        let long = 6_000;
        let allowed = ["de".to_string(), "en".to_string()];
        assert_eq!(pick_language(&serde_json::json!({"tr": 0.5, "en": 0.3, "de": 0.2}), &allowed, long), "en");
        // Turkish is not allowed; neither listed language heard at all: the first.
        assert_eq!(pick_language(&serde_json::json!({"tr": 0.9}), &allowed, long), "de");
        assert_eq!(pick_language(&serde_json::json!({"de": 0.4, "en": 0.4}), &allowed, long), "de", "a tie: the first");
        assert_eq!(pick_language(&serde_json::Value::Null, &allowed, long), "de");
        // A German speaker's English sentence, 7.4 s (TTS "Stefan").
        assert_eq!(pick_language(&serde_json::json!({"de": 0.216, "en": 0.587}), &allowed, 7_390), "en");
    }

    #[test]
    fn a_short_dictation_stays_in_the_first_language_unless_another_is_clear() {
        let de_en = ["de".to_string(), "en".to_string()];
        let pick = |de: f64, en: f64, ms: u64| pick_language(&serde_json::json!({"de": de, "en": en, "ja": 0.2}), &de_en, ms);
        // Measured on whisper-server, Large v3 Turbo (ACFT), TTS at 1.5-2x speed: German that
        // came back as English (in English, "Okay, danke." is "Okay, thank you.").
        assert_eq!(pick(0.025, 0.292, 650), "de", "Okay, danke.");
        assert_eq!(pick(0.177, 0.300, 740), "de", "Na gut.");
        assert_eq!(pick(0.037, 0.414, 570), "de", "Na gut.");
        assert_eq!(pick(0.466, 0.469, 880), "de", "Okay, bis morgen.");
        // German that was right stays right.
        assert_eq!(pick(0.998, 0.001, 940), "de", "Ich hab kein Wasser.");
        assert_eq!(pick(0.999, 0.000, 720), "de", "Bis gleich.");
        // Clearly English stays English.
        assert_eq!(pick(0.253, 0.701, 870), "en", "See you later.");
        assert_eq!(pick(0.000, 0.998, 830), "en", "See you later.");
        assert_eq!(pick(0.081, 0.639, 660), "en", "Thank you.");
        // Unsure English goes to German - whisper keeps short English as English there
        // (measured: "No problem.", "Sounds good.", "Thank you." unchanged).
        assert_eq!(pick(0.352, 0.378, 880), "de", "No problem.");
        assert_eq!(pick(0.407, 0.568, 850), "de", "Sounds good.");
        // Nothing hardcoded to German: the first chosen decides.
        let en_de = ["en".to_string(), "de".to_string()];
        assert_eq!(pick_language(&serde_json::json!({"de": 0.40, "en": 0.30}), &en_de, 900), "en");
        assert_eq!(pick_language(&serde_json::json!({"de": 0.95, "en": 0.03}), &en_de, 900), "de");
        // A tie and none of them: the first, short or long.
        assert_eq!(pick(0.4, 0.4, 900), "de");
        assert_eq!(pick_language(&serde_json::json!({"tr": 0.9}), &de_en, 900), "de");
        // Long: the likeliest again.
        assert_eq!(pick(0.177, 0.300, SHORT_DICTATION_MS + 1), "en");
    }

    #[test]
    fn a_cloud_service_gets_the_first_language_for_a_short_dictation() {
        let list = TranscribeOptions::new("de,en", "");
        assert_eq!(cloud_language(&list, 1_400).as_deref(), Some("de"), "short: no free detection");
        assert_eq!(cloud_language(&list, SHORT_DICTATION_MS + 1), None, "long: the service detects");
        assert_eq!(cloud_language(&TranscribeOptions::new("en", ""), 60_000).as_deref(), Some("en"));
        assert_eq!(cloud_language(&TranscribeOptions::new("auto", ""), 900), None);
    }

    #[test]
    fn the_pick_is_logged_without_the_text() {
        let line = language_note(&serde_json::json!({"de": 0.025, "en": 0.292, "ja": 0.3}), &["de".to_string(), "en".to_string()], 1377, "de");
        assert_eq!(line, "language: 1.4 s of audio, among de,en: de 0.03, en 0.29 -> de");
    }

    #[test]
    fn whisper_server_keeps_its_choice_when_it_is_allowed() {
        // One run with "auto": the probabilities come from after the run, on its own context
        // (whisper.cpp detects on the previous request's context - the review's finding).
        let (url, rx) = serve(vec![json(200, r#"{"text":"Ok, danke.","language":"german","language_probabilities":{"de":0.6,"en":0.3}}"#)]);
        let (text, language) = WhisperServerTranscriber::new(&url, Some(128))
            .transcribe_detecting(&crate::audio::encode_wav(&vec![0.1; 16_000], 16_000), &TranscribeOptions::new("de,en", ""))
            .unwrap();
        let run = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(run.contains("name=\"language\"\r\n\r\nauto") && run.contains("verbose_json") && !run.contains("no_language_probabilities"), "{run}");
        assert_eq!((text.as_str(), language.as_deref()), ("Ok, danke.", Some("german")));
        assert!(rx.try_recv().is_err(), "one request when Whisper chose an allowed language");
    }

    #[test]
    fn whisper_server_runs_again_in_the_likeliest_allowed_language() {
        // Short German speech that whisper.cpp alone took for Turkish.
        let (url, rx) = serve(vec![
            json(200, r#"{"text":"Ich hap kain vasser","language":"turkish","language_probabilities":{"tr":0.55,"de":0.3,"en":0.1}}"#),
            json(200, r#"{"text":"Ich hab kein Wasser.","language":"german"}"#),
        ]);
        let (text, language) = WhisperServerTranscriber::new(&url, Some(128))
            .transcribe_detecting(&crate::audio::encode_wav(&vec![0.1; 16_000], 16_000), &TranscribeOptions::new("de,en", ""))
            .unwrap();
        let _first = rx.recv().unwrap();
        let second = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(second.contains("name=\"language\"\r\n\r\nde"), "Turkish was never allowed: {second}");
        assert_eq!((text.as_str(), language.as_deref()), ("Ich hab kein Wasser.", Some("german")));
    }

    #[test]
    fn a_cloud_engine_gets_no_language_list() {
        // Groq or OpenAI refuse "de,en": they detect by themselves then - a long dictation.
        let (url, rx) = serve(vec![json(200, r#"{"text":"x"}"#), json(200, r#"{"text":"x"}"#)]);
        let long = crate::audio::encode_wav(&vec![0.1; 16 * (SHORT_DICTATION_MS as usize + 500)], 16_000);
        OpenAiTranscriber::new(&url, "", "m").transcribe(&long, &TranscribeOptions::new("de,en", "")).unwrap();
        let body = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(!body.contains("name=\"language\""), "{body}");
        // A short one in the first chosen language.
        let short = crate::audio::encode_wav(&vec![0.1; 16_000], 16_000);
        OpenAiTranscriber::new(&url, "", "m").transcribe(&short, &TranscribeOptions::new("de,en", "")).unwrap();
        let body = String::from_utf8_lossy(&rx.recv().unwrap().body).to_string();
        assert!(body.contains("name=\"language\"\r\n\r\nde\r\n"), "{body}");
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
    fn health_means_this_kind_of_server_said_ok() {
        let (url, _rx) = serve(vec![json(200, r#"{"status":"ok"}"#)]);
        assert!(health_ok(&url));
        let (url, _rx) = serve(vec![json(503, r#"{"error":"loading"}"#)]);
        assert!(!health_ok(&url));
        // Some other program on the port.
        let (url, _rx) = serve(vec![json(200, r#"<html>hello</html>"#)]);
        assert!(!health_ok(&url));
        assert!(!health_ok("http://127.0.0.1:9"));
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
