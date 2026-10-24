//! The gateway's HTTP: reading one request off a connection, writing one reply. Only what the app uses.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const MAX_HEADER: usize = 8192;
pub(super) const MAX_BODY: usize = 64 * 1024;
/// A request, once its first byte has arrived, must be whole within this long: a client that trickles bytes cannot hold a
/// connection slot (there are only a few) by resetting a per-read timeout.
pub(super) const REQUEST_WITHIN: Duration = Duration::from_secs(10);

/// What the gateway needs of a connection, so the same request handling serves a TCP socket (Wi-Fi) and a Bluetooth stream
/// channel (ble.rs).
pub trait Conn: Read + Write + Send {
    fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()>;
    /// Who is on the other end, for the logs and the lockout: an IP address, or `ble`.
    fn peer(&self) -> String;
    fn set_nodelay(&self) {}
}

impl Conn for TcpStream {
    fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        TcpStream::set_read_timeout(self, d)
    }

    fn peer(&self) -> String {
        self.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default()
    }

    fn set_nodelay(&self) {
        let _ = TcpStream::set_nodelay(self, true);
    }
}

pub(super) struct Request {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) bearer: String,
    pub(super) body: Vec<u8>,
    pub(super) close: bool, // the client asked for the connection to be closed after this request
    pub(super) deflate: bool, // the client reads raw-deflate replies (X-Compress: raw-deflate)
}

pub(super) enum Next {
    Request(Request),
    /// Nothing arrived: the client left, or a kept-alive connection was idle too long.
    Closed,
    /// Malformed, too big (headers over MAX_HEADER, a body over MAX_BODY) or too slow (REQUEST_WITHIN).
    Bad,
}

/// Read more of a request into `buf`: false when it ran out of time (`Next::Bad` or, with nothing read yet, `Closed`) or the
/// peer left. `wait` applies until the first byte; after that the request has until `deadline`.
fn fill(s: &mut dyn Conn, buf: &mut Vec<u8>, deadline: &mut Option<Instant>, wait: Duration) -> bool {
    let timeout = deadline.map_or(wait, |d| d.saturating_duration_since(Instant::now()));
    if timeout.is_zero() || s.set_read_timeout(Some(timeout)).is_err() {
        return false;
    }
    let mut chunk = [0u8; 1024];
    match s.read(&mut chunk) {
        Ok(n) if n > 0 => {
            buf.extend_from_slice(&chunk[..n]);
            deadline.get_or_insert_with(|| Instant::now() + REQUEST_WITHIN);
            true
        }
        _ => false,
    }
}

/// The next request on a connection, waiting up to `wait` for its first byte. `pending` holds bytes read past the end of the
/// previous request (a client that sends the next one early); what this one leaves over goes back into it.
pub(super) fn read_request(s: &mut dyn Conn, wait: Duration, pending: &mut Vec<u8>) -> Next {
    let mut buf = std::mem::take(pending);
    let mut deadline = (!buf.is_empty()).then(|| Instant::now() + REQUEST_WITHIN);
    let end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > MAX_HEADER {
            return Next::Bad;
        }
        if !fill(s, &mut buf, &mut deadline, wait) {
            return if buf.is_empty() { Next::Closed } else { Next::Bad };
        }
    };
    if end > MAX_HEADER {
        return Next::Bad;
    }
    let Ok(head) = String::from_utf8(buf[..end].to_vec()) else { return Next::Bad };
    let mut body = buf[end + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(path)) = (first.next(), first.next()) else { return Next::Bad };
    let (mut bearer, mut length, mut close, mut deflate) = (String::new(), 0usize, first.next() == Some("HTTP/1.0"), false);
    for (k, v) in lines.filter_map(|l| l.split_once(':')) {
        let v = v.trim();
        if k.eq_ignore_ascii_case("authorization") {
            bearer = v.strip_prefix("Bearer ").map(|t| t.trim().to_string()).unwrap_or_default();
        } else if k.eq_ignore_ascii_case("content-length") {
            let Ok(n) = v.parse() else { return Next::Bad };
            length = n;
        } else if k.eq_ignore_ascii_case("connection") {
            close = v.eq_ignore_ascii_case("close");
        } else if k.eq_ignore_ascii_case("x-compress") {
            deflate = v.eq_ignore_ascii_case("raw-deflate");
        }
    }
    if length > MAX_BODY {
        return Next::Bad;
    }
    while body.len() < length {
        if !fill(s, &mut body, &mut deadline, wait) {
            return Next::Bad;
        }
    }
    if body.len() > length {
        *pending = body.split_off(length);
    }
    Next::Request(Request { method: method.into(), path: path.into(), bearer, body, close, deflate })
}

/// The whole reply in one write (one segment where it fits). `keep` leaves the connection open for the next request: the
/// phone polls several times a second, and a new connection each time costs a handshake and, on the phone, noticeable stalls
/// (measured 2026-10-23: the poll after every log upload took about 820 ms).
pub(super) fn respond(w: &mut dyn Write, status: &str, kind: &str, body: &[u8], keep: bool, encoding: Option<&str>) -> std::io::Result<()> {
    let connection = if keep { "keep-alive\r\nKeep-Alive: timeout=15" } else { "close" };
    let encoded = encoding.map_or(String::new(), |e| format!("X-Compressed: {e}\r\n"));
    let mut out = format!("HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{encoded}Cache-Control: no-store\r\nConnection: {connection}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    w.write_all(&out)
}

/// Replies larger than this are compressed for a client that asks (`X-Compress: raw-deflate`): over Bluetooth, which moves
/// about 15-20 KB a second (measured 2026-10-23: a 10 KB snapshot took 500-670 ms, the 193 KB signal list never finished),
/// the live snapshot shrinks 3 times (10,306 to 3,458 bytes, median of 876) and the signal list 6.6 times (193,649 to 29,159).
/// A custom header, not `Accept-Encoding`: a phone's HTTP stack sends that on its own and expects zlib-wrapped data, not the
/// raw DEFLATE the app's Bluetooth link decodes.
pub(super) const COMPRESS_OVER: usize = 512;

pub(super) fn compress(body: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    let _ = enc.write_all(body);
    enc.finish().unwrap_or_default()
}
