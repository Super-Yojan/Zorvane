//! Loopback HTTP front for the WebGPU page.
//!
//! Chrome blocks a public HTTPS origin from opening `ws://127.0.0.1`
//! (`net::ERR_BLOCKED_BY_LOCAL_NETWORK_ACCESS_CHECKS`). The gateway therefore
//! serves the simulator on loopback and proxies the published assets.

use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};
use url::Url;

const HEADER_LIMIT: usize = 8192;
const BODY_LIMIT: u64 = 512 * 1024 * 1024;

pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(180))
        .redirects(5)
        .build()
}

pub(crate) fn directory_prefix(upstream: &Url) -> String {
    let path = upstream.path();
    if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

/// `Ok(None)` redirects to the simulator directory. `Ok(Some)` is the relative
/// asset path under `prefix`. `Err` is a rejected target.
pub(crate) fn asset_relative(target: &str, prefix: &str) -> Result<Option<String>, ()> {
    if target.starts_with("//") || target.contains("://") {
        return Err(());
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !path.starts_with('/')
        || path.contains('\\')
        || path.bytes().any(|b| b.is_ascii_control())
        || query.bytes().any(|b| b.is_ascii_control() || b == b' ')
    {
        return Err(());
    }
    let mut parts = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." || decoded_unsafe(segment) {
            return Err(());
        }
        parts.push(segment);
    }
    let mut normalized = format!("/{}", parts.join("/"));
    if path.ends_with('/') && normalized != "/" {
        normalized.push('/');
    }
    let bare = prefix.trim_end_matches('/');
    if normalized == bare || normalized == "/" {
        return Ok(None);
    }
    if !normalized.starts_with(prefix) {
        return Err(());
    }
    let mut relative = normalized[prefix.len()..].to_string();
    if !query.is_empty() {
        relative.push('?');
        relative.push_str(query);
    }
    Ok(Some(relative))
}

fn decoded_unsafe(segment: &str) -> bool {
    let mut out = Vec::new();
    let bytes = segment.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
            if let Ok(value) = u8::from_str_radix(hex, 16) {
                out.push(value);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    let text = String::from_utf8_lossy(&out);
    text.contains('/') || text.contains('\\') || text == ".." || text.contains('\0')
}

pub(crate) fn serve(mut stream: TcpStream, upstream: &Url, agent: &ureq::Agent) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(180)));
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while bytes.len() < HEADER_LIMIT && !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(_) => return,
        }
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return;
    };
    let Some((head, _)) = text.split_once("\r\n\r\n") else {
        return;
    };
    let mut lines = head.split("\r\n");
    let Some(request) = lines.next() else {
        return;
    };
    let mut parts = request.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return;
    };
    if method != "GET" && method != "HEAD" {
        write_fixed(&mut stream, 405, "text/plain", b"method not allowed");
        return;
    }
    let prefix = directory_prefix(upstream);
    let relative = match asset_relative(target, &prefix) {
        Ok(Some(relative)) => relative,
        Ok(None) => {
            write_redirect(&mut stream, &prefix);
            return;
        }
        Err(()) => {
            write_fixed(&mut stream, 404, "text/plain", b"not found");
            return;
        }
    };
    let mut range = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("range") {
            let value = value.trim();
            if value.len() <= 128 && value.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
                range = Some(value.to_string());
            }
        }
    }
    let Ok(url) = upstream.join(&relative) else {
        write_fixed(&mut stream, 404, "text/plain", b"not found");
        return;
    };
    if url.host() != upstream.host()
        || url.port() != upstream.port()
        || url.scheme() != upstream.scheme()
    {
        write_fixed(&mut stream, 404, "text/plain", b"not found");
        return;
    }
    let document =
        relative.is_empty() || relative == "index.html" || relative.starts_with("index.html?");
    let mut request = if method == "HEAD" {
        agent.head(url.as_str())
    } else {
        agent.get(url.as_str())
    };
    request = request
        .set("Accept-Encoding", "identity")
        .set("User-Agent", "Zorvane/0.1");
    if let Some(range) = range.as_deref() {
        request = request.set("Range", range);
    }
    match request.call() {
        Ok(response) => forward(&mut stream, response, method == "HEAD", document, upstream),
        Err(ureq::Error::Status(_, response)) => {
            forward(&mut stream, response, method == "HEAD", document, upstream)
        }
        Err(_) if document && method != "HEAD" => {
            write_fixed(
                &mut stream,
                502,
                "text/html; charset=utf-8",
                &unavailable(upstream),
            );
        }
        Err(_) => write_fixed(
            &mut stream,
            502,
            "text/plain",
            b"simulator page unavailable",
        ),
    }
}

fn forward(
    stream: &mut TcpStream,
    response: ureq::Response,
    head: bool,
    document: bool,
    upstream: &Url,
) {
    let status = response.status();
    if document && status == 404 && !head {
        write_fixed(
            stream,
            502,
            "text/html; charset=utf-8",
            &unavailable(upstream),
        );
        return;
    }
    let mut headers = String::new();
    for name in [
        "content-type",
        "cache-control",
        "etag",
        "last-modified",
        "accept-ranges",
        "content-range",
    ] {
        if let Some(value) = response.header(name) {
            if value
                .bytes()
                .all(|b| b.is_ascii() && b != b'\r' && b != b'\n')
            {
                headers.push_str(name);
                headers.push_str(": ");
                headers.push_str(value);
                headers.push_str("\r\n");
            }
        }
    }
    let length = response
        .header("content-length")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|length| *length <= BODY_LIMIT);
    let status_text = reason(status);
    if head || matches!(status, 204 | 304) {
        let length = length.unwrap_or(0);
        let _ = write!(
            stream,
            "HTTP/1.1 {status} {status_text}\r\n{headers}Content-Length: {length}\r\nConnection: close\r\n\r\n"
        );
        return;
    }
    if let Some(length) = length {
        if write!(
            stream,
            "HTTP/1.1 {status} {status_text}\r\n{headers}Content-Length: {length}\r\nConnection: close\r\n\r\n"
        )
        .is_err()
        {
            return;
        }
        let _ = copy_limited(stream, &mut response.into_reader(), length);
        return;
    }
    // ureq has already decoded the body. Buffer only when the upstream omitted a length.
    let mut body = Vec::new();
    if response
        .into_reader()
        .take(BODY_LIMIT + 1)
        .read_to_end(&mut body)
        .is_err()
        || body.len() as u64 > BODY_LIMIT
    {
        return;
    }
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(&body);
}

fn copy_limited(
    stream: &mut TcpStream,
    reader: &mut impl Read,
    mut remaining: u64,
) -> std::io::Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        let want = buffer.len().min(remaining as usize);
        let count = reader.read(&mut buffer[..want])?;
        if count == 0 {
            break;
        }
        stream.write_all(&buffer[..count])?;
        remaining -= count as u64;
    }
    Ok(())
}

fn unavailable(upstream: &Url) -> Vec<u8> {
    let location = upstream.as_str().replace('&', "&amp;").replace('<', "&lt;");
    format!(
        "<!DOCTYPE html><html lang=\"en\"><meta charset=\"utf-8\"><title>Zorvane</title>\
         <p>The WebGPU simulator could not be loaded from {location}.</p>\
         <p>This client is running. The published WebGPU build is not available yet.</p></html>"
    )
    .into_bytes()
}

fn write_redirect(stream: &mut TcpStream, location: &str) {
    let body = b"redirect";
    let _ = write!(
        stream,
        "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(body);
}

fn write_fixed(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(body);
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        204 => "No Content",
        206 => "Partial Content",
        302 => "Found",
        304 => "Not Modified",
        404 => "Not Found",
        405 => "Method Not Allowed",
        502 => "Bad Gateway",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_paths_stay_under_the_play_prefix() {
        let prefix = "/Zorvane/play/webgpu/";
        assert_eq!(
            asset_relative("/Zorvane/play/webgpu/", prefix).unwrap(),
            Some(String::new())
        );
        assert_eq!(
            asset_relative("/Zorvane/play/webgpu", prefix).unwrap(),
            None
        );
        assert_eq!(asset_relative("/", prefix).unwrap(), None);
        assert_eq!(
            asset_relative("/Zorvane/play/webgpu/index.html", prefix).unwrap(),
            Some("index.html".into())
        );
        assert_eq!(
            asset_relative("/Zorvane/play/webgpu/assets/rover.glb?x=1", prefix).unwrap(),
            Some("assets/rover.glb?x=1".into())
        );
        assert!(asset_relative("/Zorvane/play/webgpu/../../secret", prefix).is_err());
        assert!(asset_relative("/Zorvane/play/webgpu/%2e%2e/secret", prefix).is_err());
        assert!(asset_relative("/etc/passwd", prefix).is_err());
        assert!(asset_relative("http://evil.test/", prefix).is_err());
    }
}
