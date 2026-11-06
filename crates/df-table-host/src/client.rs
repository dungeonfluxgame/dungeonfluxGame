//! A minimal HTTP client for the developer API on the loopback address.
//!
//! It owns one request at a time over a plain socket, with a deadline on the whole exchange and a
//! cap on the answer. It speaks only to `127.0.0.1`, so it needs no TLS and no proxy handling, and
//! it never retries. An answer that is cut short or too large is an error, never a shorter body.

use std::fmt;
use std::io::ErrorKind;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::limits::{
    MAXIMUM_RESPONSE_BYTES, REQUEST_TIMEOUT, STREAM_ANSWER_TIMEOUT, STREAM_STEP_TIMEOUT,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientError {
    /// Nothing is listening on the port.
    NotServing {
        port: u16,
    },
    TimedOut,
    /// The connection failed for a reason other than nothing listening.
    Connection(ErrorKind),
    TooLarge,
    /// The connection closed before the whole body arrived.
    Truncated,
    /// The answer is not an HTTP answer this client reads (no head, no status, a chunked body).
    InvalidResponse,
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotServing { port } => write!(
                formatter,
                "nothing is serving on 127.0.0.1:{port} \
                 (start it with: df-table serve --port {port})"
            ),
            Self::TimedOut => formatter.write_str("the canvas did not answer in time"),
            Self::Connection(kind) => {
                write!(formatter, "the connection to the canvas failed: {kind}")
            }
            Self::TooLarge => write!(
                formatter,
                "the answer is larger than {MAXIMUM_RESPONSE_BYTES} bytes"
            ),
            Self::Truncated => {
                formatter.write_str("the connection closed before the answer was complete")
            }
            Self::InvalidResponse => {
                formatter.write_str("the canvas sent an answer that is not HTTP")
            }
        }
    }
}

impl std::error::Error for ClientError {}

fn connection_error(error: &std::io::Error) -> ClientError {
    ClientError::Connection(error.kind())
}

async fn connect(port: u16) -> Result<TcpStream, ClientError> {
    TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| match error.kind() {
            ErrorKind::ConnectionRefused => ClientError::NotServing { port },
            kind => ClientError::Connection(kind),
        })
}

/// Reads `source` to its end, refusing more than `limit` bytes. One byte past the limit is read
/// so an answer cut off by the cap is told from one that ends exactly at it.
async fn read_capped(source: impl AsyncRead + Unpin, limit: usize) -> Result<Vec<u8>, ClientError> {
    let mut answer = Vec::new();
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    source
        .take(cap)
        .read_to_end(&mut answer)
        .await
        .map_err(|error| connection_error(&error))?;
    if answer.len() > limit {
        return Err(ClientError::TooLarge);
    }
    Ok(answer)
}

async fn exchange(port: u16, request: &[u8], limit: usize) -> Result<Vec<u8>, ClientError> {
    let mut stream = connect(port).await?;
    stream
        .write_all(request)
        .await
        .map_err(|error| connection_error(&error))?;
    read_capped(stream, limit).await
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

fn is_chunked(head: &str) -> bool {
    head.lines().any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding")
                && value.to_ascii_lowercase().contains("chunked")
        })
    })
}

/// Splits an answer into its head and body. The length is checked on the bytes as they came, so
/// a body that is not valid UTF-8 cannot pass for a longer one once it is read as text.
fn parse_response(answer: &[u8]) -> Result<Response, ClientError> {
    const HEAD_END: &[u8] = b"\r\n\r\n";
    let head_length = answer
        .windows(HEAD_END.len())
        .position(|window| window == HEAD_END)
        .ok_or(ClientError::InvalidResponse)?;
    let head = String::from_utf8_lossy(&answer[..head_length]);
    let body = &answer[head_length + HEAD_END.len()..];
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(ClientError::InvalidResponse)?;
    if is_chunked(&head) {
        return Err(ClientError::InvalidResponse);
    }
    if content_length(&head).is_some_and(|expected| body.len() < expected) {
        return Err(ClientError::Truncated);
    }
    Ok(Response {
        status,
        body: String::from_utf8_lossy(body).into_owned(),
    })
}

async fn request(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
    limit: usize,
) -> Result<Response, ClientError> {
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let answer = tokio::time::timeout(REQUEST_TIMEOUT, exchange(port, request.as_bytes(), limit))
        .await
        .map_err(|_| ClientError::TimedOut)??;
    parse_response(&answer)
}

/// GET `path` and read the whole answer.
pub async fn get(port: u16, path: &str) -> Result<Response, ClientError> {
    request(port, "GET", path, "", MAXIMUM_RESPONSE_BYTES).await
}

/// POST `body` (JSON, possibly empty) to `path` and read the whole answer.
pub async fn post_json(port: u16, path: &str, body: &str) -> Result<Response, ClientError> {
    request(port, "POST", path, body, MAXIMUM_RESPONSE_BYTES).await
}

/// `get` with a smaller cap, so a test can reach the cap without a huge answer.
#[cfg(test)]
pub(crate) async fn get_with_limit(
    port: u16,
    path: &str,
    limit: usize,
) -> Result<Response, ClientError> {
    request(port, "GET", path, "", limit).await
}

/// A POST whose body is sent in pieces, for a body too large to hold: the caller says the length
/// up front, sends exactly that many bytes, then reads the answer. Each sending step has its own
/// deadline, so a long upload is not cut off while it keeps moving; the answer has one fixed
/// deadline of its own.
pub struct StreamedPost {
    stream: TcpStream,
}

impl StreamedPost {
    /// Connects and sends the request head for a JSON body of `content_length` bytes.
    pub async fn begin(port: u16, path: &str, content_length: u64) -> Result<Self, ClientError> {
        let mut stream = tokio::time::timeout(STREAM_STEP_TIMEOUT, connect(port))
            .await
            .map_err(|_| ClientError::TimedOut)??;
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
             Content-Length: {content_length}\r\nConnection: close\r\n\r\n"
        );
        send_piece(&mut stream, head.as_bytes()).await?;
        Ok(Self { stream })
    }

    /// Sends the next piece of the body.
    pub async fn send(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        send_piece(&mut self.stream, bytes).await
    }

    /// Reads the answer once the whole body is sent (or the host stopped listening), waiting at
    /// most `STREAM_ANSWER_TIMEOUT` while the host stores the file.
    pub async fn finish(self) -> Result<Response, ClientError> {
        let answer = tokio::time::timeout(
            STREAM_ANSWER_TIMEOUT,
            read_capped(self.stream, MAXIMUM_RESPONSE_BYTES),
        )
        .await
        .map_err(|_| ClientError::TimedOut)??;
        parse_response(&answer)
    }
}

async fn send_piece(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), ClientError> {
    tokio::time::timeout(STREAM_STEP_TIMEOUT, stream.write_all(bytes))
        .await
        .map_err(|_| ClientError::TimedOut)?
        .map_err(|error| connection_error(&error))
}
