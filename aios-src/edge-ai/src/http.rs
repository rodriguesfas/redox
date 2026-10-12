//! Dependency-free micro HTTP/1.1: a tiny server (thread-per-connection) and
//! a tiny client, both over `std::net`. No external server/client crate, so it
//! compiles for `*-unknown-redox` and keeps Edge AI OS lean.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// Hard cap on the request body (model prompts are small).
pub const MAX_BODY: usize = 64 * 1024 * 1024;
const MAX_HEADER: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    pub query: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn json<'a, T: serde::de::Deserialize<'a>>(&'a self) -> Result<T, String> {
        serde_json::from_slice(&self.body).map_err(|e| format!("json: {e}"))
    }
}

pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// When set, the connection streams items as `data: <item>\n\n` frames
    /// (SSE) after the headers, with a terminal `data: [DONE]`. Consumes the
    /// iterator lazily — items are flushed to the socket as they are produced.
    pub stream: Option<Box<dyn Iterator<Item = String> + Send>>,
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("body_len", &self.body.len())
            .field("stream", &self.stream.as_ref().map(|_| "<stream>"))
            .finish()
    }
}

pub fn reason_phrase(code: u16) -> &'static str {
    match code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

impl Response {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Response {
            status,
            content_type: "application/json",
            body: value.to_string().into_bytes(),
            stream: None,
        }
    }

    pub fn html(status: u16, body: &str) -> Self {
        Response {
            status,
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
            stream: None,
        }
    }

    pub fn text(status: u16, body: &str) -> Self {
        Response {
            status,
            content_type: "text/plain; charset=utf-8",
            body: body.as_bytes().to_vec(),
            stream: None,
        }
    }

    pub fn error(status: u16, message: &str) -> Self {
        Response::json(status, &serde_json::json!({ "error": message }))
    }

    /// SSE (Server-Sent Events) buffered response — the whole body is
    /// computed before headers are written. For incremental delivery use
    /// [`Response::sse_stream`].
    pub fn sse(status: u16, body: String) -> Self {
        Response {
            status,
            content_type: "text/event-stream; charset=utf-8",
            body: body.into_bytes(),
            stream: None,
        }
    }

    /// SSE streaming response: headers go out first, then each item is
    /// framed as `data: <item>\n\n` and flushed immediately, ending with
    /// `data: [DONE]`. No Content-Length; the reader knows the stream is
    /// over when the connection closes or [DONE] arrives.
    pub fn sse_stream(status: u16, iter: Box<dyn Iterator<Item = String> + Send>) -> Self {
        Response {
            status,
            content_type: "text/event-stream; charset=utf-8",
            body: Vec::new(),
            stream: Some(iter),
        }
    }
}

/// Request handler signature used by [`serve`].
pub type Handler = dyn Fn(&Request) -> Response + Send + Sync + 'static;

/// Serve `handler` on `addr` (e.g. "0.0.0.0:8989"), thread-per-connection.
pub fn serve(addr: &str, handler: &'static Handler) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr)?;
    eprintln!("edge-ai: listening on http://{addr}");
    for conn in listener.incoming() {
        match conn {
            Ok(mut stream) => {
                std::thread::spawn(move || {
                    let _ = handle_connection(&mut stream, handler);
                });
            }
            Err(e) => eprintln!("edge-ai: accept: {e}"),
        }
    }
    Ok(())
}

/// Parse the response status line ("HTTP/1.1 200 OK") into the code.
fn status_code(line: &str) -> u16 {
    let mut parts = line.split_whitespace();
    if parts
        .next()
        .map(|s| s.starts_with("HTTP/"))
        .unwrap_or(false)
    {
        if let Some(code) = parts.next() {
            if let Ok(c) = code.parse() {
                return c;
            }
        }
    }
    0
}

fn parse_query(q: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in q.split('&') {
        if pair.is_empty() {
            continue;
        }
        if let Some((k, v)) = pair.split_once('=') {
            map.insert(k.to_string(), percent_decode(v));
        } else {
            map.insert(pair.to_string(), String::new());
        }
    }
    map
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Find the `\r\n\r\n` terminator of the header block.
fn header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

/// True when the buffer holds the status line, headers and the full body
/// advertised by `Content-Length`. Without that header the caller keeps
/// reading until the peer closes.
fn response_ready(buf: &[u8]) -> bool {
    let Some(pos) = header_end(buf) else {
        return false;
    };
    let head = String::from_utf8_lossy(&buf[..pos]);
    let mut content_length: Option<usize> = None;
    for line in head.lines().skip(1) {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().ok();
            }
        }
    }
    match content_length {
        Some(len) => buf.len() >= pos + 4 + len,
        None => false,
    }
}

fn handle_connection(stream: &mut TcpStream, handler: &Handler) -> std::io::Result<()> {
    // Read incrementally until the header block is complete.
    let mut buf: Vec<u8> = Vec::new();
    let mut req: Option<ParsedRequest> = None;
    let mut tmp = [0u8; 8192];
    loop {
        let n = stream.read(&mut tmp)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(pos) = header_end(&buf) {
            let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
            let tail = buf[pos + 4..].to_vec();
            req = Some(parse_request(&head, tail)?);
            break;
        }
        if buf.len() > MAX_HEADER {
            return write_response(stream, Response::error(413, "header too large"));
        }
    }
    let mut parsed = match req {
        Some(p) => p,
        None => return Ok(()), // connection closed before headers
    };
    // Read any remaining body bytes (Content-Length may exceed what we got).
    let have = parsed.inner.body.len();
    let mut need = 0usize;
    if have < parsed.body_cap {
        need = parsed.body_cap - have;
    }
    if need > 0 {
        let mut more = vec![0u8; need];
        stream.read_exact(&mut more)?;
        parsed.inner.body.extend_from_slice(&more);
    }
    parsed.inner.body.truncate(parsed.body_cap);
    let resp = handler(&parsed.inner);
    write_response(stream, resp)
}

struct ParsedRequest {
    inner: Request,
    body_cap: usize,
}

fn parse_request(head: &str, tail: Vec<u8>) -> std::io::Result<ParsedRequest> {
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default();
    if method.is_empty() || target.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "bad request line",
        ));
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target.to_string(), HashMap::new()),
    };
    let mut content_length = 0usize;
    for h in lines {
        if let Some((k, v)) = h.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    let body_cap = content_length.min(MAX_BODY);
    let mut body = tail;
    body.truncate(body_cap);
    Ok(ParsedRequest {
        inner: Request {
            method,
            path,
            query,
            body,
        },
        body_cap,
    })
}

fn write_response(stream: &mut TcpStream, mut resp: Response) -> std::io::Result<()> {
    let mut out = Vec::new();
    out.extend_from_slice(
        format!(
            "HTTP/1.1 {} {}\r\n",
            resp.status,
            reason_phrase(resp.status)
        )
        .as_bytes(),
    );
    out.extend_from_slice(format!("Content-Type: {}\r\n", resp.content_type).as_bytes());
    // Streaming bodies have no known length up front; buffered bodies do.
    if resp.stream.is_none() {
        out.extend_from_slice(format!("Content-Length: {}\r\n", resp.body.len()).as_bytes());
    }
    out.extend_from_slice(b"Connection: close\r\nServer: edge-ai\r\n\r\n");
    out.extend_from_slice(&resp.body);
    stream.write_all(&out)?;
    stream.flush()?;

    // Streaming body: frame each item and flush so the client renders
    // tokens as they are produced rather than at end of generation.
    if let Some(iter) = resp.stream.as_mut() {
        for item in iter.by_ref() {
            stream.write_all(b"data: ")?;
            stream.write_all(item.as_bytes())?;
            stream.write_all(b"\n\n")?;
            stream.flush()?;
        }
        stream.write_all(b"data: [DONE]\n\n")?;
        stream.flush()?;
    }

    // The Redox netstack does not turn a dropped fd into a TCP FIN in time for
    // the peer. Shutdown the write half so the client read reaches EOF.
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// A minimal HTTP client for the `edge` CLI.
#[derive(Debug, Clone)]
pub struct Client {
    pub host: String,
    pub port: u16,
    pub cookie: Option<String>,
}

impl Client {
    /// Parse a base address like `127.0.0.1:8989` (host or `host:port`).
    pub fn from_base(base: &str) -> Result<Client, String> {
        let (host, port) = match base.rsplit_once(':') {
            Some((h, p)) => {
                if h.contains(':') {
                    // bare IPv6 address -> default port
                    return Err(format!("IPv6 bases are not supported yet: {base}"));
                }
                let port: u16 = p
                    .parse()
                    .map_err(|_| format!("bad port in base '{base}'"))?;
                (h.to_string(), port)
            }
            None => (base.to_string(), super::DEFAULT_PORT),
        };
        Ok(Client {
            host,
            port,
            cookie: None,
        })
    }

    /// Create a client with a cookie for authentication.
    pub fn with_cookie(base: &str, cookie: String) -> Result<Client, String> {
        let mut client = Self::from_base(base)?;
        client.cookie = Some(cookie);
        Ok(client)
    }

    /// Perform a request; returns `(status, body)`.
    pub fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<(u16, Vec<u8>), String> {
        let mut target = String::from(path);
        if target.is_empty() {
            target.push('/');
        }
        let mut head = format!(
            "{method} {target} HTTP/1.1\r\nHost: {}:{}\r\nUser-Agent: edge-cli\r\nAccept: application/json\r\n",
            self.host, self.port
        );
        if let Some(ref cookie) = self.cookie {
            head.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        if let Some(b) = body {
            head.push_str(&format!(
                "Content-Type: application/json\r\nContent-Length: {}\r\n",
                b.len()
            ));
        }
        head.push_str("Connection: close\r\n\r\n");

        let mut stream = TcpStream::connect((self.host.as_str(), self.port))
            .map_err(|e| format!("connect {}:{}: {e}", self.host, self.port))?;
        stream
            .write_all(head.as_bytes())
            .map_err(|e| format!("write: {e}"))?;
        if let Some(b) = body {
            stream
                .write_all(b)
                .map_err(|e| format!("write body: {e}"))?;
        }
        stream.flush().map_err(|e| format!("flush: {e}"))?;

        let mut buf: Vec<u8> = Vec::new();
        let mut tmp = [0u8; 8192];
        loop {
            let n = stream.read(&mut tmp).map_err(|e| format!("read: {e}"))?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            if response_ready(&buf) {
                break;
            }
        }
        let pos =
            header_end(&buf).ok_or_else(|| "malformed response (no header end)".to_string())?;
        let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
        let mut lines = head.lines();
        let status = status_code(lines.next().unwrap_or_default());
        let mut content_length = 0usize;
        for h in lines {
            if let Some((k, v)) = h.split_once(':') {
                if k.trim().eq_ignore_ascii_case("content-length") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
        }
        let mut body = buf[pos + 4..].to_vec();
        body.truncate(content_length);
        Ok((status, body))
    }

    pub fn get(&self, path: &str) -> Result<(u16, Vec<u8>), String> {
        self.request("GET", path, None)
    }

    pub fn post_json(
        &self,
        path: &str,
        value: &serde_json::Value,
    ) -> Result<(u16, Vec<u8>), String> {
        self.request("POST", path, Some(value.to_string().as_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_query_and_decode() {
        let q = parse_query("model=my%20model&tokens=32&flag");
        assert_eq!(q.get("model").map(|s| s.as_str()), Some("my model"));
        assert_eq!(q.get("tokens").map(|s| s.as_str()), Some("32"));
        assert_eq!(q.get("flag").map(|s| s.as_str()), Some(""));
    }

    #[test]
    fn status_line_parsing() {
        assert_eq!(status_code("HTTP/1.1 200 OK"), 200);
        assert_eq!(status_code("HTTP/1.1 503 Service Unavailable"), 503);
        assert_eq!(status_code("garbage"), 0);
    }

    #[test]
    fn response_ready_waits_for_content_length() {
        let partial = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nab";
        assert!(!response_ready(partial));
        let full = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabcde";
        assert!(response_ready(full));
        assert!(!response_ready(b"HTTP/1.1 200 OK\r\n\r\nabcde"));
    }

    #[test]
    fn reason_phrases_cover_used_codes() {
        for code in [200, 201, 400, 404, 405, 413, 500, 503] {
            assert!(!reason_phrase(code).is_empty());
        }
    }
}
