//! Serves clips and track audio to the window over plain HTTP on this PC only.
//!
//! The webview's own file protocol hands every piece of a video to the app's main thread,
//! which slows big files down and can make the whole window stall while one loads. A small
//! server on 127.0.0.1 lets the webview's network stack fetch byte ranges directly, on other
//! threads. It answers only with a random secret in the address, only for files the asset
//! scope allows (the clip folder and Umbra's caches), and only reads.

use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use tauri::{AppHandle, Manager};

struct Server {
    port: u16,
    token: String,
}

static SERVER: OnceLock<Server> = OnceLock::new();

/// Pages allowed to read media cross-origin (the mixer uses Web Audio, which needs CORS).
const ORIGINS: &[&str] = &[
    "http://tauri.localhost",
    "https://tauri.localhost",
    "http://localhost:1420",
];

/// Starts the server; afterwards `url` gives addresses on it.
pub fn start(app: &AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(l) => l,
        Err(err) => {
            log::warn!("cannot start the media server, using the file protocol: {err}");
            return;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let token = random_token();
    if SERVER
        .set(Server {
            port,
            token: token.clone(),
        })
        .is_err()
    {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("umbra-media".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let app = app.clone();
                let token = token.clone();
                let _ = std::thread::Builder::new()
                    .name("umbra-media-conn".into())
                    .spawn(move || serve(stream, &app, &token));
            }
        });
    if spawned.is_err() {
        log::warn!("cannot start the media server thread");
    }
}

/// The address for a file on the media server, if it runs.
pub fn url(path: &Path) -> Option<String> {
    let server = SERVER.get()?;
    Some(format!(
        "http://127.0.0.1:{}/{}/{}",
        server.port,
        server.token,
        crate::clips::encode_uri_component(&path.to_string_lossy())
    ))
}

fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for _ in 0..2 {
        let mut h = RandomState::new().build_hasher();
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

struct Request {
    method: String,
    target: String,
    range: Option<String>,
    origin: Option<String>,
    keep_alive: bool,
}

fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut request = Request {
        method,
        target,
        range: None,
        origin: None,
        keep_alive: true,
    };
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim().to_string();
            match name.trim().to_ascii_lowercase().as_str() {
                "range" => request.range = Some(value),
                "origin" => request.origin = Some(value),
                "connection" => request.keep_alive = !value.eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
    }
    Some(request)
}

fn serve(stream: TcpStream, app: &AppHandle, token: &str) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(60)));
    let _ = stream.set_nodelay(true);
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut writer = std::io::BufWriter::with_capacity(256 * 1024, write_half);
    let mut reader = BufReader::new(stream);
    while let Some(request) = read_request(&mut reader) {
        let keep = request.keep_alive;
        if respond(&mut writer, &request, app, token).is_err() || !keep {
            break;
        }
    }
}

fn respond(
    out: &mut impl Write,
    request: &Request,
    app: &AppHandle,
    token: &str,
) -> std::io::Result<()> {
    let cors = request
        .origin
        .as_deref()
        .filter(|o| ORIGINS.contains(o))
        .map(|o| format!("Access-Control-Allow-Origin: {o}\r\nVary: Origin\r\n"))
        .unwrap_or_default();
    let fail = |out: &mut dyn Write, status: &str| -> std::io::Result<()> {
        write!(out, "HTTP/1.1 {status}\r\nContent-Length: 0\r\n{cors}\r\n")?;
        out.flush()
    };
    if request.method == "OPTIONS" {
        write!(
            out,
            "HTTP/1.1 204 No Content\r\n{cors}Access-Control-Allow-Methods: GET, HEAD\r\nAccess-Control-Allow-Headers: Range\r\nContent-Length: 0\r\n\r\n"
        )?;
        return out.flush();
    }
    if request.method != "GET" && request.method != "HEAD" {
        return fail(out, "405 Method Not Allowed");
    }
    let Some(path) = request
        .target
        .strip_prefix('/')
        .and_then(|t| t.split_once('/'))
        .filter(|(t, _)| *t == token)
        .and_then(|(_, rest)| percent_decode(rest.split('?').next().unwrap_or(rest)))
        .map(PathBuf::from)
    else {
        return fail(out, "404 Not Found");
    };
    // Only what the window may load anyway: the clip folder and Umbra's caches.
    if !path.is_absolute() || !app.asset_protocol_scope().is_allowed(&path) {
        return fail(out, "403 Forbidden");
    }
    let Ok(mut file) = std::fs::File::open(&path) else {
        return fail(out, "404 Not Found");
    };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (start, end) = match request.range.as_deref().and_then(|r| parse_range(r, size)) {
        Some(range) => range,
        None if request.range.is_some() => {
            write!(
                out,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{size}\r\nContent-Length: 0\r\n{cors}\r\n"
            )?;
            return out.flush();
        }
        None => (0, size.saturating_sub(1)),
    };
    let length = if size == 0 { 0 } else { end - start + 1 };
    let status = if request.range.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\n{}Cache-Control: no-store\r\n{cors}\r\n",
        content_type(&path),
        if request.range.is_some() {
            format!("Content-Range: bytes {start}-{end}/{size}\r\n")
        } else {
            String::new()
        },
    )?;
    if request.method == "GET" && length > 0 {
        file.seek(SeekFrom::Start(start))?;
        std::io::copy(&mut file.take(length), out)?;
    }
    out.flush()
}

/// `%xx` escapes back to bytes, read as UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `bytes=a-b`, `bytes=a-` or `bytes=-n` within a file of `size` bytes.
fn parse_range(header: &str, size: u64) -> Option<(u64, u64)> {
    let spec = header
        .trim()
        .strip_prefix("bytes=")?
        .split(',')
        .next()?
        .trim();
    let (a, b) = spec.split_once('-')?;
    if size == 0 {
        return None;
    }
    let (start, end) = match (a.trim(), b.trim()) {
        ("", n) => {
            let n: u64 = n.parse().ok()?;
            (size.saturating_sub(n), size - 1)
        }
        (a, "") => (a.parse().ok()?, size - 1),
        (a, b) => (a.parse().ok()?, b.parse::<u64>().ok()?.min(size - 1)),
    };
    (start <= end && start < size).then_some((start, end))
}

fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4",
        Some("m4a") => "audio/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("wav") => "audio/wav",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("opus" | "ogg") => "audio/ogg",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn ranges_are_read() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=900-5000", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }
}
