//! The control port for the Stream Deck plugin: press, release and status, on 127.0.0.1 and
//! nowhere else. Port and token reach the plugin through a handshake file the app writes
//! (`streamdeck.json` next to the settings); every request carries the token, so another
//! program on this PC cannot start a dictation by guessing the port.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::http::{error_body, Limits, Request, Server};

/// Not `Authorization`: nothing here is a standard scheme, and a name of our own says
/// plainly that this is Amber's.
pub const TOKEN_HEADER: &str = "x-amber-token";

/// The handshake file's name, next to `settings.json`.
pub const HANDSHAKE_FILE: &str = "streamdeck.json";

pub struct Hooks {
    /// The key went down / came up - the gesture machine decides what that means.
    pub press: Arc<dyn Fn() + Send + Sync>,
    pub release: Arc<dyn Fn() + Send + Sync>,
    pub toggle: Arc<dyn Fn() + Send + Sync>,
    /// `{"phase": "idle" | "recording" | ..., "version": ...}`
    pub status: Arc<dyn Fn() -> Value + Send + Sync>,
}

pub fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

/// The handshake file's content.
pub fn handshake(port: u16, token: &str) -> Value {
    json!({ "port": port, "token": token, "version": env!("CARGO_PKG_VERSION") })
}

/// Listens on a free port on 127.0.0.1. Nothing here takes a body, and the plugin asks
/// several times a second, so the limits are tight.
pub fn start(token: String, hooks: Hooks) -> std::io::Result<Server> {
    let limits = Limits { max_head: 8 << 10, max_body: 0, io_timeout: Duration::from_secs(5), max_connections: 16, ..Limits::default() };
    let hooks = Arc::new(hooks);
    let token: Arc<str> = token.into();
    Server::start(SocketAddr::from(([127, 0, 0, 1], 0)), limits, is_loopback, Arc::new(move |req: &mut Request| handle(req, &token, &hooks)))
}

fn handle(req: &mut Request, token: &str, hooks: &Hooks) -> (u16, Value) {
    let given = req.header(TOKEN_HEADER).unwrap_or("");
    if token.is_empty() || !same(given, token) {
        return (401, error_body("Wrong or missing token - the plugin reads it from streamdeck.json.", "invalid_token"));
    }
    let path = req.target.split('?').next().unwrap_or("");
    let ok = || (200, json!({ "ok": true }));
    match (req.method.as_str(), path) {
        ("GET", "/v1/status") => (200, (hooks.status)()),
        ("POST", "/v1/press") => {
            (hooks.press)();
            ok()
        }
        ("POST", "/v1/release") => {
            (hooks.release)();
            ok()
        }
        ("POST", "/v1/toggle") => {
            (hooks.toggle)();
            ok()
        }
        _ => (404, error_body("Unknown control command.", "not_found")),
    }
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn send(addr: SocketAddr, request: &str) -> (String, String) {
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        s.read_to_string(&mut answer).unwrap();
        let status = answer.lines().next().unwrap_or("").to_string();
        let body = answer.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
        (status, body)
    }

    fn server() -> (Server, Arc<[AtomicUsize; 3]>) {
        let counts: Arc<[AtomicUsize; 3]> = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)]);
        let (a, b, c) = (counts.clone(), counts.clone(), counts.clone());
        let hooks = Hooks {
            press: Arc::new(move || {
                a[0].fetch_add(1, Ordering::SeqCst);
            }),
            release: Arc::new(move || {
                b[1].fetch_add(1, Ordering::SeqCst);
            }),
            toggle: Arc::new(move || {
                c[2].fetch_add(1, Ordering::SeqCst);
            }),
            status: Arc::new(|| json!({ "phase": "recording" })),
        };
        (start("tok123".into(), hooks).unwrap(), counts)
    }

    #[test]
    fn press_release_toggle_and_status_with_the_token() {
        let (srv, counts) = server();
        assert_eq!(srv.addr.ip(), IpAddr::from([127, 0, 0, 1]));
        let req = |m: &str, p: &str| format!("{m} {p} HTTP/1.1\r\nx-amber-token: tok123\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(send(srv.addr, &req("POST", "/v1/press")).0, "HTTP/1.1 200 OK");
        assert_eq!(send(srv.addr, &req("POST", "/v1/release")).0, "HTTP/1.1 200 OK");
        assert_eq!(send(srv.addr, &req("POST", "/v1/toggle")).0, "HTTP/1.1 200 OK");
        let (status, body) = send(srv.addr, &req("GET", "/v1/status"));
        assert_eq!(status, "HTTP/1.1 200 OK");
        assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["phase"], "recording");
        assert_eq!(counts.iter().map(|c| c.load(Ordering::SeqCst)).collect::<Vec<_>>(), [1, 1, 1]);
        assert_eq!(send(srv.addr, &req("POST", "/v1/explode")).0, "HTTP/1.1 404 Not Found");
    }

    #[test]
    fn nothing_without_the_token() {
        let (srv, counts) = server();
        for headers in ["", "x-amber-token: wrong\r\n", "x-amber-token: tok12\r\n", "Authorization: Bearer tok123\r\n"] {
            let (status, _) = send(srv.addr, &format!("POST /v1/press HTTP/1.1\r\n{headers}\r\n"));
            assert_eq!(status, "HTTP/1.1 401 Unauthorized", "{headers:?}");
        }
        assert_eq!(counts[0].load(Ordering::SeqCst), 0);
    }

    #[test]
    fn handshake_has_port_and_token() {
        let h = handshake(51234, "abc");
        assert_eq!(h["port"], 51234);
        assert_eq!(h["token"], "abc");
        assert!(!is_loopback("192.168.1.2".parse().unwrap()));
        assert!(is_loopback("127.0.0.1".parse().unwrap()));
    }
}
