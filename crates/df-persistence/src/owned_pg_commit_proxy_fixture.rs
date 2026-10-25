//! Fixture-only lost COMMIT acknowledgement on ROOT-registered, already connected sockets.
//! ROOT owns listener/ports/runtime/task joins. This module binds or connects no socket.
//! NoTls fixture startup only: production TLS/auth/hosting is not represented by this proxy.
use std::future::{Future, poll_fn};
use std::io;
use std::pin::pin;
use std::task::Poll;
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout_at};

#[derive(Debug, Eq, PartialEq)]
pub enum ProxyError {
    Capacity,
    Protocol,
    Io,
    Deadline,
    ClosedBeforeCommit,
}
#[derive(Debug, Eq, PartialEq)]
pub struct CommitAckObservation {
    pub frontend_commit_forwarded: bool,
    pub postgres_commit_complete_observed: bool,
    pub postgres_ready_idle_observed: bool,
    pub suppressed_response_bytes: usize,
}
#[derive(Clone, Copy)]
pub struct ProxyBounds {
    pub maximum_frame_bytes: usize,
    pub maximum_suppressed_response_bytes: usize,
    pub deadline: Instant,
}
struct Frames {
    buffer: Vec<u8>,
    startup: bool,
    maximum_frame_bytes: usize,
}
impl Frames {
    fn new(startup: bool, maximum_frame_bytes: usize) -> Result<Self, ProxyError> {
        if !(8..=4 * 1024 * 1024).contains(&maximum_frame_bytes) {
            return Err(ProxyError::Capacity);
        }
        Ok(Self {
            buffer: Vec::new(),
            startup,
            maximum_frame_bytes,
        })
    }
    fn append(&mut self, bytes: &[u8]) -> Result<(), ProxyError> {
        let next = self
            .buffer
            .len()
            .checked_add(bytes.len())
            .ok_or(ProxyError::Capacity)?;
        let maximum_buffer_bytes = self
            .maximum_frame_bytes
            .checked_add(8192)
            .ok_or(ProxyError::Capacity)?;
        if next > maximum_buffer_bytes {
            return Err(ProxyError::Capacity);
        }
        self.buffer
            .try_reserve_exact(bytes.len())
            .map_err(|_| ProxyError::Capacity)?;
        if self.buffer.capacity() > maximum_buffer_bytes {
            return Err(ProxyError::Capacity);
        }
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }
    fn consume(
        &mut self,
        mut observe: impl FnMut(u8, &[u8]) -> Result<(), ProxyError>,
    ) -> Result<(), ProxyError> {
        let mut consumed = 0;
        loop {
            let current = &self.buffer[consumed..];
            let header = if self.startup { 4 } else { 5 };
            if current.len() < header {
                break;
            }
            let offset = usize::from(!self.startup);
            let raw: [u8; 4] = current[offset..offset + 4]
                .try_into()
                .map_err(|_| ProxyError::Protocol)?;
            let length =
                usize::try_from(i32::from_be_bytes(raw)).map_err(|_| ProxyError::Protocol)?;
            let total = length.checked_add(offset).ok_or(ProxyError::Capacity)?;
            if length < 4 || total > self.maximum_frame_bytes {
                return Err(ProxyError::Protocol);
            }
            if current.len() < total {
                break;
            }
            if self.startup {
                // Reject SSL/GSS negotiation; ROOT config must disable TLS for this NoTls fixture.
                if total < 8 || current[4..8] != 196608_i32.to_be_bytes() {
                    return Err(ProxyError::Protocol);
                }
                self.startup = false;
            } else {
                observe(current[0], &current[5..total])?;
            }
            consumed = consumed.checked_add(total).ok_or(ProxyError::Capacity)?;
        }
        if consumed != 0 {
            self.buffer.drain(..consumed).for_each(drop);
        }
        Ok(())
    }
}
#[derive(Clone, Copy)]
enum ReadSide {
    Frontend,
    Postgres,
}
async fn next_readable(frontend: &TcpStream, postgres: &TcpStream) -> Result<ReadSide, ProxyError> {
    let mut front = pin!(frontend.readable());
    let mut back = pin!(postgres.readable());
    poll_fn(|context| {
        match front.as_mut().poll(context) {
            Poll::Ready(Ok(())) => return Poll::Ready(Ok(ReadSide::Frontend)),
            Poll::Ready(Err(_)) => return Poll::Ready(Err(ProxyError::Io)),
            Poll::Pending => {}
        }
        match back.as_mut().poll(context) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(ReadSide::Postgres)),
            Poll::Ready(Err(_)) => Poll::Ready(Err(ProxyError::Io)),
            Poll::Pending => Poll::Pending,
        }
    })
    .await
}
async fn forward(stream: &TcpStream, mut bytes: &[u8]) -> Result<(), ProxyError> {
    while !bytes.is_empty() {
        stream.writable().await.map_err(|_| ProxyError::Io)?;
        match stream.try_write(bytes) {
            Ok(0) => return Err(ProxyError::Io),
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(ProxyError::Io),
        }
    }
    Ok(())
}
/// Completion means COMMIT CommandComplete AND idle ReadyForQuery were read from real
/// PostgreSQL, yet their bytes never reached the adapter. Both owned sockets then drop.
/// An error/EOF/deadline never claims that the underlying decision committed.
pub async fn drop_commit_acknowledgement(
    frontend: TcpStream,
    postgres: TcpStream,
    bounds: ProxyBounds,
) -> Result<CommitAckObservation, ProxyError> {
    if bounds.deadline <= Instant::now()
        || bounds.maximum_suppressed_response_bytes == 0
        || bounds.maximum_suppressed_response_bytes > 1024 * 1024
    {
        return Err(ProxyError::Capacity);
    }
    let mut front_frames = Frames::new(true, bounds.maximum_frame_bytes)?;
    let mut back_frames = Frames::new(false, bounds.maximum_frame_bytes)?;
    let mut observation = CommitAckObservation {
        frontend_commit_forwarded: false,
        postgres_commit_complete_observed: false,
        postgres_ready_idle_observed: false,
        suppressed_response_bytes: 0,
    };
    timeout_at(bounds.deadline, async {
        let mut buffer = [0_u8; 8192];
        loop {
            let side = next_readable(&frontend, &postgres).await?;
            let source = match side {
                ReadSide::Frontend => &frontend,
                ReadSide::Postgres => &postgres,
            };
            let count = match source.try_read(&mut buffer) {
                Ok(0) => return Err(ProxyError::ClosedBeforeCommit),
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(_) => return Err(ProxyError::Io),
            };
            let bytes = &buffer[..count];
            match side {
                ReadSide::Frontend => {
                    front_frames.append(bytes)?;
                    let mut commit = false;
                    front_frames.consume(|kind, body| {
                        if kind == b'Q' && body == b"COMMIT\0" {
                            commit = true;
                        }
                        Ok(())
                    })?;
                    // Forward every COMMIT byte before claiming it reached the database.
                    forward(&postgres, bytes).await?;
                    if commit {
                        observation.frontend_commit_forwarded = true;
                    }
                }
                ReadSide::Postgres => {
                    back_frames.append(bytes)?;
                    back_frames.consume(|kind, body| {
                        if observation.frontend_commit_forwarded
                            && kind == b'C'
                            && body == b"COMMIT\0"
                        {
                            observation.postgres_commit_complete_observed = true;
                        }
                        if observation.postgres_commit_complete_observed
                            && kind == b'Z'
                            && body == b"I"
                        {
                            observation.postgres_ready_idle_observed = true;
                        }
                        Ok(())
                    })?;
                    if observation.frontend_commit_forwarded {
                        observation.suppressed_response_bytes = observation
                            .suppressed_response_bytes
                            .checked_add(count)
                            .ok_or(ProxyError::Capacity)?;
                        if observation.suppressed_response_bytes
                            > bounds.maximum_suppressed_response_bytes
                        {
                            return Err(ProxyError::Capacity);
                        }
                        if observation.postgres_ready_idle_observed {
                            return Ok(observation);
                        }
                    } else {
                        forward(&frontend, bytes).await?;
                    }
                }
            }
        }
    })
    .await
    .map_err(|_| ProxyError::Deadline)?
}

#[cfg(test)]
mod parser_tests {
    use super::*;
    fn startup() -> Vec<u8> {
        let mut value = Vec::new();
        value.extend_from_slice(&8_i32.to_be_bytes());
        value.extend_from_slice(&196608_i32.to_be_bytes());
        value
    }
    fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
        let mut value = vec![kind];
        value.extend_from_slice(&(i32::try_from(body.len()).unwrap() + 4).to_be_bytes());
        value.extend_from_slice(body);
        value
    }
    #[test]
    fn fragmented_startup_and_commit_are_detected_only_after_complete_request() {
        let mut parser = Frames::new(true, 1024).unwrap();
        for byte in startup() {
            parser.append(&[byte]).unwrap();
            parser
                .consume(|_, _| panic!("startup is not a typed frame"))
                .unwrap();
        }
        let request = frame(b'Q', b"COMMIT\0");
        let mut commits = 0;
        for (index, byte) in request.iter().enumerate() {
            parser.append(&[*byte]).unwrap();
            parser
                .consume(|kind, body| {
                    if kind == b'Q' && body == b"COMMIT\0" {
                        commits += 1;
                    }
                    Ok(())
                })
                .unwrap();
            assert_eq!(commits, usize::from(index == request.len() - 1));
        }
        assert!(parser.buffer.is_empty());
    }
    #[test]
    fn rollback_and_error_frames_do_not_report_successful_commit() {
        let mut parser = Frames::new(false, 1024).unwrap();
        let mut committed = false;
        let mut stream = frame(b'C', b"ROLLBACK\0");
        stream.extend(frame(b'E', b"SERROR\0\0"));
        stream.extend(frame(b'Z', b"I"));
        parser.append(&stream).unwrap();
        parser
            .consume(|kind, body| {
                if kind == b'C' && body == b"COMMIT\0" {
                    committed = true;
                }
                Ok(())
            })
            .unwrap();
        assert!(!committed);
    }
    #[test]
    fn oversized_frame_header_refuses_before_declared_body_allocation() {
        let mut parser = Frames::new(false, 1024).unwrap();
        let mut header = vec![b'D'];
        header.extend_from_slice(&i32::MAX.to_be_bytes());
        parser.append(&header).unwrap();
        assert_eq!(parser.consume(|_, _| Ok(())), Err(ProxyError::Protocol));
        assert!(parser.buffer.capacity() <= 1024 + 8192);
    }
    #[test]
    fn tls_negotiation_is_refused_by_the_owned_no_tls_fixture() {
        let mut parser = Frames::new(true, 1024).unwrap();
        let mut ssl = Vec::new();
        ssl.extend_from_slice(&8_i32.to_be_bytes());
        ssl.extend_from_slice(&80877103_i32.to_be_bytes());
        parser.append(&ssl).unwrap();
        assert_eq!(parser.consume(|_, _| Ok(())), Err(ProxyError::Protocol));
    }
}
