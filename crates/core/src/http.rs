//! A small, defensive HTTP/1.1 server for Amber's two ports: the phone API on the home
//! network and the Stream Deck control on 127.0.0.1. Written by hand after tiny_http turned
//! out to let one packet without a key abort the app (it allocated an announced
//! Content-Length when dropping a request) and three half-sent requests stop it for good.
//!
//! Here: the peer is checked before anything is read, every socket has timeouts, the head
//! is capped, a body is read only when the handler asks for it (after its own checks), the
//! number of connections is capped, a panic in a handler stays in its connection, and
//! stopping frees the port at once.

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

/// How much the server takes from one connection. Anything beyond is refused before it
/// costs memory or a thread for long.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Request line and headers together.
    pub max_head: usize,
    pub max_body: usize,
    /// Per read and write: a client that vanished mid-upload frees its slot after this.
    pub io_timeout: Duration,
    /// For the whole upload, so a trickle of bytes cannot hold a slot forever either.
    pub body_deadline: Duration,
    /// For request line and headers, counted from the connection - the same for the head.
    pub head_deadline: Duration,
    pub max_connections: usize,
    /// From one address at a time: one device on the network cannot take every slot.
    pub max_per_peer: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_head: 16 << 10,
            max_body: 64 << 20,
            io_timeout: Duration::from_secs(15),
            body_deadline: Duration::from_secs(180),
            head_deadline: Duration::from_secs(10),
            max_connections: 8,
            max_per_peer: 3,
        }
    }
}

/// One request as the handler sees it: the head is read, the body only on demand.
pub struct Request<'a> {
    pub method: String,
    pub target: String,
    pub peer: IpAddr,
    headers: Vec<(String, String)>,
    /// Bytes read past the head: the start of the body.
    rest: Vec<u8>,
    stream: &'a mut TcpStream,
    limits: Limits,
}

impl Request<'_> {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The body announced by Content-Length - refused when chunked, missing, too large or
    /// too slow. Call it only once the request has passed its checks.
    pub fn body(&mut self) -> Result<Vec<u8>, (u16, &'static str)> {
        if self.header("Transfer-Encoding").is_some() {
            return Err((411, "Send the request with a Content-Length."));
        }
        let len = self
            .header("Content-Length")
            .and_then(|v| v.parse::<usize>().ok())
            .ok_or((411, "Send the request with a Content-Length."))?;
        if len > self.limits.max_body {
            return Err((413, "The request is too large."));
        }
        if self.header("Expect").is_some_and(|v| v.eq_ignore_ascii_case("100-continue")) {
            let _ = self.stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        }
        let started = Instant::now();
        let mut body = std::mem::take(&mut self.rest);
        body.truncate(len);
        let mut chunk = vec![0u8; 64 << 10];
        while body.len() < len {
            if started.elapsed() > self.limits.body_deadline {
                return Err((408, "The upload took too long."));
            }
            let want = (len - body.len()).min(chunk.len());
            match self.stream.read(&mut chunk[..want]) {
                Ok(0) => return Err((400, "The upload broke off.")),
                Ok(n) => body.extend_from_slice(&chunk[..n]),
                Err(_) => return Err((408, "The upload broke off.")),
            }
        }
        Ok(body)
    }
}

/// Answers a request with a status and a JSON body.
pub type Handler = Arc<dyn Fn(&mut Request) -> (u16, Value) + Send + Sync>;

/// OpenAI's error shape, which every client here knows how to show.
pub fn error_body(message: &str, kind: &str) -> Value {
    serde_json::json!({ "error": { "message": message, "type": kind } })
}

/// A running server. Stopping (or dropping) closes the port at once; requests already being
/// answered finish on their own threads, bounded by the limits.
pub struct Server {
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    pub addr: SocketAddr,
}

impl Server {
    /// Listens on [addr]; connections from peers [allow] refuses are answered 403 without
    /// reading anything. Every response closes the connection.
    pub fn start(addr: SocketAddr, limits: Limits, allow: fn(IpAddr) -> bool, handler: Handler) -> std::io::Result<Server> {
        let listener = TcpListener::bind(addr)?;
        let addr = listener.local_addr()?;
        // Non-blocking accept, polled: stopping needs no wake-up connection (which Windows
        // refuses for 0.0.0.0) and frees the port within one poll.
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared { handler, limits, allow, active: AtomicUsize::new(0), per_peer: Mutex::new(HashMap::new()) });
        let flag = stop.clone();
        let accept = std::thread::Builder::new().name("amber-http".into()).spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, peer)) => accept_one(stream, peer, &shared),
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
            }
        })?;
        Ok(Server { stop, accept: Some(accept), addr })
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.accept.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct Shared {
    handler: Handler,
    limits: Limits,
    allow: fn(IpAddr) -> bool,
    active: AtomicUsize,
    /// Open connections per address.
    per_peer: Mutex<HashMap<IpAddr, usize>>,
}

/// Counts a connection, in all and for its address, while its thread runs.
struct Slot(Arc<Shared>, IpAddr);

impl Slot {
    /// None if the server, or this address, has no room for another connection.
    fn take(shared: &Arc<Shared>, peer: IpAddr) -> Option<Slot> {
        let mut per_peer = shared.per_peer.lock().unwrap();
        let mine = per_peer.entry(peer).or_insert(0);
        if *mine >= shared.limits.max_per_peer {
            return None;
        }
        if shared.active.fetch_add(1, Ordering::SeqCst) >= shared.limits.max_connections {
            shared.active.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        *mine += 1;
        Some(Slot(shared.clone(), peer))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
        let mut per_peer = self.0.per_peer.lock().unwrap();
        if let Some(n) = per_peer.get_mut(&self.1) {
            *n -= 1;
            if *n == 0 {
                per_peer.remove(&self.1);
            }
        }
    }
}

fn accept_one(mut stream: TcpStream, peer: SocketAddr, shared: &Arc<Shared>) {
    let limits = shared.limits;
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(limits.io_timeout));
    let _ = stream.set_write_timeout(Some(limits.io_timeout));
    // Strangers get their answer before anything of theirs is read.
    if !(shared.allow)(peer.ip()) {
        respond(&mut stream, 403, &error_body("Not allowed from this address.", "forbidden"));
        return;
    }
    let Some(slot) = Slot::take(shared, peer.ip()) else {
        respond(&mut stream, 503, &error_body("Busy - try again in a moment.", "server_error"));
        return;
    };
    let accepted = Instant::now();
    // A failed spawn drops the closure, and with it the slot and the connection.
    let _ = std::thread::Builder::new().name("amber-http-conn".into()).spawn(move || {
        let shared = slot.0.clone();
        // A bug in one request must not take the server (or Amber) down with it.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| serve(&mut stream, peer.ip(), &shared, accepted)));
        if outcome.is_err() {
            respond(&mut stream, 500, &error_body("Internal error.", "server_error"));
        }
        drop(slot);
    });
}

fn serve(stream: &mut TcpStream, peer: IpAddr, shared: &Shared, accepted: Instant) {
    let limits = shared.limits;
    let (method, target, headers, rest) = match read_head(stream, limits.max_head, accepted + limits.head_deadline, limits.io_timeout) {
        Ok(h) => h,
        Err((status, message)) => return respond(stream, status, &error_body(message, "invalid_request_error")),
    };
    let _ = stream.set_read_timeout(Some(limits.io_timeout));
    let mut request = Request { method, target, peer, headers, rest, stream, limits: shared.limits };
    let (status, body) = (shared.handler)(&mut request);
    respond(request.stream, status, &body);
}

type Head = (String, String, Vec<(String, String)>, Vec<u8>);

/// Reads up to the blank line after the headers, never more than [max] bytes of head, and
/// not past [until] - a byte now and then must not hold the connection for days.
fn read_head(stream: &mut TcpStream, max: usize, until: Instant, io_timeout: Duration) -> Result<Head, (u16, &'static str)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(i) = find(&buf, b"\r\n\r\n") {
            break i;
        }
        if buf.len() > max {
            return Err((431, "Request headers too large."));
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err((408, "Request timed out."));
        }
        let _ = stream.set_read_timeout(Some(left.min(io_timeout)));
        match stream.read(&mut chunk) {
            Ok(0) => return Err((400, "Connection closed mid-request.")),
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => return Err((408, "Request timed out.")),
        }
    };
    if end > max {
        return Err((431, "Request headers too large."));
    }
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| (400, "Malformed request."))?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target, version) = (first.next().unwrap_or(""), first.next().unwrap_or(""), first.next().unwrap_or(""));
    if method.is_empty() || !target.starts_with('/') || !version.starts_with("HTTP/1.") {
        return Err((400, "Malformed request."));
    }
    let headers = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    Ok((method.to_ascii_uppercase(), target.to_string(), headers, buf[end + 4..].to_vec()))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn respond(stream: &mut TcpStream, status: u16, body: &Value) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let body = body.to_string();
    let response =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}
