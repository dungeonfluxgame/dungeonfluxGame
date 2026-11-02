//! A minimal HTTP client for the developer API on the loopback address.
//!
//! It owns one request at a time over a plain socket, with a deadline on the whole exchange and a
//! cap on the answer. It speaks only to `127.0.0.1`, so it needs no TLS and no proxy handling, and
//! it never retries. An answer that is cut short or too large is an error, never a shorter body.

use std::fmt;
use std::io::ErrorKind;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::limits::{MAXIMUM_RESPONSE_BYTES, REQUEST_TIMEOUT};

/// Deadline for each step of a streamed request: connecting, sending one piece, reading the answer.
const STREAM_STEP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError {
    /// Nothing is listening on the port.
    NotServing {
        port: u16,
    },
    TimedOut,
    /// The connection failed for a reason other than nothing listening.
    Connection(String),
    TooLarge,
    /// The connection closed before the whole body arrived.
    Truncated,
    BadResponse,
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotServing { port } => write!(
                formatter,
                "nothing is serving on 127.0.0.1:{port} (start it with: df-table serve --port {port})"
            ),
            Self::TimedOut => formatter.write_str("the canvas did not answer in time"),
            Self::Connection(message) => write!(formatter, "connection problem: {message}"),
            Self::TooLarge => write!(
                formatter,
                "the answer is larger than {MAXIMUM_RESPONSE_BYTES} bytes"
            ),
            Self::Truncated => {
                formatter.write_str("the connection closed before the answer was complete")
            }
            Self::BadResponse => formatter.write_str("the canvas sent an answer that is not HTTP"),
        }
    }
}

async fn exchange(port: u16, request: &[u8], limit: usize) -> Result<Vec<u8>, ClientError> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| match error.kind() {
            ErrorKind::ConnectionRefused => ClientError::NotServing { port },
            _ => ClientError::Connection(error.to_string()),
        })?;
    stream
        .write_all(request)
        .await
        .map_err(|error| ClientError::Connection(error.to_string()))?;
    let mut answer = Vec::new();
    // One byte past the limit shows that the answer was cut off by the cap.
    let cap = u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(1);
    stream
        .take(cap)
        .read_to_end(&mut answer)
        .await
        .map_err(|error| ClientError::Connection(error.to_string()))?;
    if answer.len() > limit {
        return Err(ClientError::TooLarge);
    }
    Ok(answer)
}

fn content_length(head: &str) -> Option<usize> {
    head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse().ok())
            .flatten()
    })
}

fn parse_response(answer: &[u8]) -> Result<Response, ClientError> {
    let text = String::from_utf8_lossy(answer);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or(ClientError::BadResponse)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(ClientError::BadResponse)?;
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return Err(ClientError::BadResponse);
    }
    if content_length(head).is_some_and(|expected| body.len() < expected) {
        return Err(ClientError::Truncated);
    }
    Ok(Response {
        status,
        body: body.to_owned(),
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
/// up front, sends exactly that many bytes, then reads the answer. Each step has its own deadline,
/// so a long upload is not cut off while it keeps moving.
pub struct StreamedPost {
    stream: TcpStream,
}

impl StreamedPost {
    /// Connects and sends the request head for a JSON body of `content_length` bytes.
    pub async fn begin(port: u16, path: &str, content_length: u64) -> Result<Self, ClientError> {
        let mut stream =
            tokio::time::timeout(STREAM_STEP_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
                .await
                .map_err(|_| ClientError::TimedOut)?
                .map_err(|error| match error.kind() {
                    ErrorKind::ConnectionRefused => ClientError::NotServing { port },
                    _ => ClientError::Connection(error.to_string()),
                })?;
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

    /// Reads the answer once the whole body is sent (or the host stopped listening).
    pub async fn finish(mut self) -> Result<Response, ClientError> {
        let mut answer = Vec::new();
        let cap = u64::try_from(MAXIMUM_RESPONSE_BYTES)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        tokio::time::timeout(
            STREAM_STEP_TIMEOUT,
            (&mut self.stream).take(cap).read_to_end(&mut answer),
        )
        .await
        .map_err(|_| ClientError::TimedOut)?
        .map_err(|error| ClientError::Connection(error.to_string()))?;
        if answer.len() > MAXIMUM_RESPONSE_BYTES {
            return Err(ClientError::TooLarge);
        }
        parse_response(&answer)
    }
}

async fn send_piece(stream: &mut TcpStream, bytes: &[u8]) -> Result<(), ClientError> {
    tokio::time::timeout(STREAM_STEP_TIMEOUT, stream.write_all(bytes))
        .await
        .map_err(|_| ClientError::TimedOut)?
        .map_err(|error| ClientError::Connection(error.to_string()))
}
