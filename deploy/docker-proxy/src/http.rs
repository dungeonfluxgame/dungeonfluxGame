//! Just enough HTTP/1.1 for one request per connection: parse the request head, refuse anything that could smuggle a second
//! request past the policy, and rebuild the head that is forwarded to the Docker daemon.

/// A parsed request head. The body is read separately, up to `content_length` bytes.
#[derive(Debug, PartialEq, Eq)]
pub struct Head {
    pub method: String,
    pub target: String,
    pub content_length: usize,
}

/// Largest request head and body the proxy accepts. A create body is a few hundred bytes.
pub const MAX_HEAD_BYTES: usize = 16 * 1024;
pub const MAX_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct BadRequest(pub &'static str);

/// Parse a head that ends at the first blank line. `raw` includes the final CRLF CRLF.
pub fn parse_head(raw: &[u8]) -> Result<Head, BadRequest> {
    let text = std::str::from_utf8(raw).map_err(|_| BadRequest("the request head is not UTF-8"))?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(BadRequest("empty request"))?;
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if parts.next().is_some()
        || method.is_empty()
        || !target.starts_with('/')
        || version != "HTTP/1.1"
    {
        return Err(BadRequest("malformed request line"));
    }
    // The target goes into the forwarded request line verbatim: printable ASCII only, in the path AND the query (no control
    // character, DEL, space or non-ASCII byte).
    if !target.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(BadRequest(
            "the request target has a control, space or non-ASCII byte",
        ));
    }
    if !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(BadRequest("malformed method"));
    }
    let mut content_length = 0usize;
    let mut seen_length = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':').ok_or(BadRequest("malformed header"))?;
        if name.is_empty() || name.contains(' ') || name.contains('\t') {
            return Err(BadRequest("malformed header name"));
        }
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            // A second way to frame the body would let a second request ride behind the first.
            "transfer-encoding" | "upgrade" | "trailer" => {
                return Err(BadRequest(
                    "transfer-encoding, upgrade and trailer are not allowed",
                ));
            }
            "content-length" => {
                if seen_length {
                    return Err(BadRequest("duplicate content-length"));
                }
                seen_length = true;
                content_length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| BadRequest("bad content-length"))?;
                if content_length > MAX_BODY_BYTES {
                    return Err(BadRequest("request body too large"));
                }
            }
            _ => {}
        }
    }
    Ok(Head {
        method: method.to_string(),
        target: target.to_string(),
        content_length,
    })
}

/// The head that is forwarded: the same request line, a fixed set of headers, and `Connection: close` so the daemon never
/// keeps the connection open for a second request.
pub fn forward_head(head: &Head, content_length: usize) -> Vec<u8> {
    let mut out = format!(
        "{} {} HTTP/1.1\r\nHost: docker\r\nConnection: close\r\n",
        head.method, head.target
    );
    if content_length > 0 {
        out.push_str("Content-Type: application/json\r\n");
    }
    out.push_str(&format!("Content-Length: {content_length}\r\n\r\n"));
    out.into_bytes()
}

/// The refusal sent to the client.
pub fn refusal_response(reason: &str) -> Vec<u8> {
    let escaped: String = reason
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
        .collect();
    let body = format!("{{\"message\":\"refused by df-docker-proxy: {escaped}\"}}");
    format!(
        "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}
