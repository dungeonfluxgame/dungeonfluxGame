//! The open launcher window's line for agents: `dungeonflux window` reads and sets the control
//! and table display windows' properties through it. The window listens on a loopback port and
//! writes the port and a random token to `<data>/launcher-control.toml`; a request must start
//! with that token, so a web page (which cannot read the file) can never drive the window. The
//! file goes when the window closes.
use std::{
    io::{self, BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::{files, log};

pub const FILE: &str = "launcher-control.toml";

/// How long one request may take, from either side.
const WAIT: Duration = Duration::from_secs(10);

#[derive(Debug, Eq, PartialEq)]
pub enum Request {
    Read,
    Set { name: String, value: String },
}

/// Properties as `(name, value)`, or why the request was refused.
pub type Reply = Result<Vec<(String, String)>, String>;

#[derive(Deserialize, Serialize)]
struct Endpoint {
    port: u16,
    token: String,
    pid: u32,
}

/// The listening window's control file; removed by `close`.
pub struct Listening {
    file: PathBuf,
}

impl Listening {
    pub fn close(&self) {
        let _ = std::fs::remove_file(&self.file);
        log::event("CONTROL_CLOSED", "");
    }
}

/// Starts listening; each request goes to `deliver` with the channel its reply goes back on.
/// `deliver` returns false once the window is gone.
pub fn listen(
    data: &Path,
    deliver: impl Fn(Request, mpsc::Sender<Reply>) -> bool + Send + 'static,
) -> Result<Listening, io::Error> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    let mut secret = [0u8; 24];
    getrandom::getrandom(&mut secret).map_err(|error| io::Error::other(error.to_string()))?;
    let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let file = data.join(FILE);
    let endpoint = Endpoint {
        port,
        token: token.clone(),
        pid: std::process::id(),
    };
    files::write_atomic(
        &file,
        toml::to_string(&endpoint)
            .map_err(io::Error::other)?
            .as_bytes(),
    )?;
    log::event("CONTROL_LISTENING", format!("port={port}"));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if !answer(stream, &token, &deliver) {
                return;
            }
        }
    });
    Ok(Listening { file })
}

/// Handles one connection; false once the window no longer takes requests.
fn answer(
    stream: TcpStream,
    token: &str,
    deliver: &impl Fn(Request, mpsc::Sender<Reply>) -> bool,
) -> bool {
    let _ = stream.set_read_timeout(Some(WAIT));
    let _ = stream.set_write_timeout(Some(WAIT));
    let Ok(reading) = stream.try_clone() else {
        return true;
    };
    // Two short lines are all a request needs.
    let mut lines = BufReader::new(io::Read::take(reading, 4096));
    let mut first = String::new();
    let mut second = String::new();
    let read = lines
        .read_line(&mut first)
        .and_then(|_| lines.read_line(&mut second));
    let mut stream = stream;
    if read.is_err() || !same(first.trim_end(), token) {
        log::event("CONTROL_REFUSED", "reason=token");
        let _ = writeln!(stream, "ERROR refused");
        return true;
    }
    let request = match parse(second.trim_end()) {
        Ok(request) => request,
        Err(message) => {
            let _ = writeln!(stream, "ERROR {message}");
            return true;
        }
    };
    let (sender, receiver) = mpsc::channel();
    if !deliver(request, sender) {
        let _ = writeln!(stream, "ERROR the launcher window is closing");
        return false;
    }
    let reply = receiver
        .recv_timeout(WAIT)
        .unwrap_or_else(|_| Err("the launcher window did not answer".to_owned()));
    let _ = match reply {
        Ok(properties) => properties
            .iter()
            .try_for_each(|(name, value)| writeln!(stream, "WINDOW name={name} value={value}")),
        Err(message) => writeln!(stream, "ERROR {message}"),
    };
    true
}

/// Compares without stopping at the first difference.
fn same(given: &str, token: &str) -> bool {
    given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// `read`, or `set <name> <value>` (the value may contain spaces).
fn parse(line: &str) -> Result<Request, String> {
    let mut words = line.splitn(3, ' ');
    match (words.next(), words.next(), words.next()) {
        (Some("read"), None, None) => Ok(Request::Read),
        (Some("set"), Some(name), value) if !name.is_empty() => Ok(Request::Set {
            name: name.to_owned(),
            value: value.unwrap_or_default().to_owned(),
        }),
        _ => Err("send read, or set <name> <value>".to_owned()),
    }
}

/// Sends one request to the open launcher window and returns its reply lines.
pub fn send(data: &Path, request: &Request) -> Result<Vec<String>, String> {
    let closed = || "No launcher window is open for this data folder.".to_owned();
    let text = std::fs::read_to_string(data.join(FILE)).map_err(|_| closed())?;
    let endpoint: Endpoint = toml::from_str(&text).map_err(|_| closed())?;
    let mut stream =
        TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).map_err(|_| closed())?;
    let _ = stream.set_read_timeout(Some(WAIT + Duration::from_secs(2)));
    let line = match request {
        Request::Read => "read".to_owned(),
        Request::Set { name, value } => format!("set {name} {value}"),
    };
    writeln!(stream, "{}\n{line}", endpoint.token).map_err(|error| error.to_string())?;
    let reply: Vec<String> = BufReader::new(stream)
        .lines()
        .collect::<Result<_, _>>()
        .map_err(|error| format!("The launcher window did not answer: {error}"))?;
    match reply.first() {
        Some(first) if first.starts_with("ERROR ") => Err(first["ERROR ".len()..].to_owned()),
        _ => Ok(reply),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_parse_and_tokens_compare_whole() {
        assert_eq!(parse("read"), Ok(Request::Read));
        assert_eq!(
            parse("set control.size 900x700"),
            Ok(Request::Set {
                name: "control.size".to_owned(),
                value: "900x700".to_owned()
            })
        );
        assert_eq!(
            parse("set display.screenshot C:\\a b\\shot.jpg"),
            Ok(Request::Set {
                name: "display.screenshot".to_owned(),
                value: "C:\\a b\\shot.jpg".to_owned()
            })
        );
        for refused in ["", "read now", "set", "GET / HTTP/1.1", "delete x"] {
            assert!(parse(refused).is_err(), "{refused:?}");
        }
        assert!(same("abc", "abc"));
        assert!(!same("abd", "abc"));
        assert!(!same("ab", "abc"));
        assert!(!same("POST / HTTP/1.1", "abc"));
    }

    #[test]
    fn only_a_request_with_the_token_reaches_the_window() {
        let data = std::env::temp_dir().join(format!("df-launcher-control-{}", std::process::id()));
        std::fs::create_dir_all(&data).unwrap();
        let listening = listen(&data, |request, reply| {
            let answer = match request {
                Request::Read => Ok(vec![("control.visible".to_owned(), "true".to_owned())]),
                Request::Set { name, .. } => Err(format!("{name} is read-only")),
            };
            reply.send(answer).is_ok()
        })
        .unwrap();
        assert_eq!(
            send(&data, &Request::Read).unwrap(),
            ["WINDOW name=control.visible value=true"]
        );
        let refused = send(
            &data,
            &Request::Set {
                name: "control.title".to_owned(),
                value: "x".to_owned(),
            },
        );
        assert_eq!(refused, Err("control.title is read-only".to_owned()));
        // A browser's request line is not the token.
        let text = std::fs::read_to_string(data.join(FILE)).unwrap();
        let endpoint: Endpoint = toml::from_str(&text).unwrap();
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, endpoint.port)).unwrap();
        write!(stream, "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nread\n").unwrap();
        let reply: Vec<String> = BufReader::new(stream).lines().map(Result::unwrap).collect();
        assert_eq!(reply, ["ERROR refused"]);
        listening.close();
        assert!(send(&data, &Request::Read).is_err());
        std::fs::remove_dir_all(data).unwrap();
    }
}
