//! The running table as the game server reports it to the launcher that started it (REF-064):
//! `GET /gameplay/launcher-status` with the run's secret, answered with plain tab-separated
//! lines (see `df-tools/src/gameplay/launcher_status.rs`). Hero portraits are fetched the same
//! way so the launcher page can show them from its own origin.

use std::{
    io::{self, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    time::Duration,
};

const TOKEN_HEADER: &str = "X-DungeonFlux-Launcher";
const CONNECT_LIMIT: Duration = Duration::from_millis(700);
const READ_LIMIT: Duration = Duration::from_millis(2500);
const MAXIMUM_STATUS_BYTES: usize = 64 * 1024;
/// A creator portrait or canonical image; larger answers are refused.
pub const PORTRAIT_BYTES_MAX: usize = 6 * 1024 * 1024;
/// The party the server can seat (REF-062); extra lines are ignored.
const MAXIMUM_HEROES: usize = 6;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TableStatus {
    pub phase: String,
    pub adventure: String,
    pub scene: String,
    pub place: String,
    pub heroes: Vec<Hero>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Hero {
    pub name: String,
    pub ready: bool,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    /// A server path such as `/gameplay/imagery/<key>`, or empty.
    pub portrait: String,
}

impl TableStatus {
    /// Where the table is, in the launcher's words.
    pub fn moment(&self) -> &'static str {
        match self.phase.as_str() {
            "room" => "Gathering in the lobby",
            "creation" => "Creating heroes",
            "opening" => "The story is beginning",
            "dialogue" => "In the story",
            "combat" => "In a fight",
            "complete" => "The adventure is over",
            _ => "At the table",
        }
    }
}

pub fn fetch(display_url: &str, token: &str) -> io::Result<TableStatus> {
    let body = get(
        display_url,
        "/gameplay/launcher-status",
        token,
        MAXIMUM_STATUS_BYTES,
    )?;
    Ok(parse(&String::from_utf8_lossy(&body)))
}

/// A hero portrait from the game server; only the server's own image routes are asked for.
pub fn portrait(display_url: &str, token: &str, path: &str) -> io::Result<Vec<u8>> {
    if !portrait_path(path) {
        return Err(io::Error::other("bad_path"));
    }
    get(display_url, path, token, PORTRAIT_BYTES_MAX)
}

/// A short, loggable reason for a failed request: the reason codes above, or the kind of a
/// connection error.
pub fn reason(error: &io::Error) -> String {
    match error.kind() {
        io::ErrorKind::Other => error.to_string(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "timeout".to_owned(),
        io::ErrorKind::ConnectionRefused => "connection_refused".to_owned(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {
            "connection_reset".to_owned()
        }
        kind => format!("{kind:?}").to_ascii_lowercase(),
    }
}

pub fn portrait_path(path: &str) -> bool {
    let key = path
        .strip_prefix("/gameplay/imagery/")
        .or_else(|| path.strip_prefix("/gameplay/creator-portrait/"));
    key.is_some_and(|key| {
        !key.is_empty()
            && key.len() <= 128
            && key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
            && !key.contains("..")
    })
}

pub fn parse(text: &str) -> TableStatus {
    let mut status = TableStatus::default();
    for line in text.lines() {
        let mut fields = line.split('\t');
        match fields.next() {
            Some("phase") => status.phase = fields.next().unwrap_or_default().to_owned(),
            Some("adventure") => status.adventure = fields.next().unwrap_or_default().to_owned(),
            Some("scene") => status.scene = fields.next().unwrap_or_default().to_owned(),
            Some("place") => status.place = fields.next().unwrap_or_default().to_owned(),
            Some("hero") if status.heroes.len() < MAXIMUM_HEROES => {
                let hero_fields: Vec<&str> = fields.collect();
                if let [ready, hit_points, maximum, portrait, name] = hero_fields.as_slice()
                    && !name.trim().is_empty()
                {
                    status.heroes.push(Hero {
                        name: (*name).to_owned(),
                        ready: *ready == "1",
                        hit_points: hit_points.parse().unwrap_or(0),
                        maximum_hit_points: maximum.parse().unwrap_or(0),
                        portrait: (*portrait).to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    status
}

/// One HTTP/1.1 GET to the game server's own address (from its display link).
fn get(
    display_url: &str,
    path: &str,
    token: &str,
    maximum_body_bytes: usize,
) -> io::Result<Vec<u8>> {
    let authority = display_url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .filter(|authority| !authority.is_empty())
        .ok_or_else(|| io::Error::other("not_http"))?;
    let address = authority
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::other("no_address"))?;
    let mut stream = TcpStream::connect_timeout(&address, CONNECT_LIMIT)?;
    stream.set_read_timeout(Some(READ_LIMIT))?;
    stream.set_write_timeout(Some(READ_LIMIT))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\n{TOKEN_HEADER}: {token}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = Vec::new();
    stream
        .take(u64::try_from(maximum_body_bytes + 16 * 1024).unwrap_or(u64::MAX))
        .read_to_end(&mut response)?;
    body(&response, maximum_body_bytes)
}

/// The body of a complete `200` response, plain or chunked.
fn body(response: &[u8], maximum_body_bytes: usize) -> io::Result<Vec<u8>> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| io::Error::other("incomplete"))?;
    let head = String::from_utf8_lossy(&response[..split]).to_ascii_lowercase();
    if !head.starts_with("http/1.1 200") && !head.starts_with("http/1.0 200") {
        let code = head.split_whitespace().nth(1).unwrap_or("unknown");
        return Err(io::Error::other(format!("http_{code}")));
    }
    let rest = &response[split + 4..];
    let content = if head.contains("transfer-encoding: chunked") {
        dechunk(rest)?
    } else {
        rest.to_vec()
    };
    if content.len() > maximum_body_bytes {
        return Err(io::Error::other("too_large"));
    }
    Ok(content)
}

fn dechunk(mut rest: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let line_end = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| io::Error::other("chunk"))?;
        let size_text = String::from_utf8_lossy(&rest[..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| io::Error::other("chunk"))?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        let chunk = rest.get(..size).ok_or_else(|| io::Error::other("chunk"))?;
        out.extend_from_slice(chunk);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_lines_become_the_table() {
        let status = parse(
            "phase\tdialogue\nadventure\tThe Broken Seal\n\
             hero\t1\t13\t13\t/gameplay/imagery/abc\tVix Ashlantern\n\
             hero\t0\t0\t0\t\tMossgrave\nhero\tbroken\n\
             scene\tThe Lantern Wharf\nplace\tsalt quay\nnoise\tx",
        );
        assert_eq!(status.moment(), "In the story");
        assert_eq!(status.adventure, "The Broken Seal");
        assert_eq!(status.scene, "The Lantern Wharf");
        assert_eq!(status.heroes.len(), 2);
        assert_eq!(
            status.heroes[0],
            Hero {
                name: "Vix Ashlantern".into(),
                ready: true,
                hit_points: 13,
                maximum_hit_points: 13,
                portrait: "/gameplay/imagery/abc".into(),
            }
        );
        assert!(!status.heroes[1].ready);
    }

    #[test]
    fn only_the_servers_image_routes_are_fetched() {
        assert!(portrait_path(
            "/gameplay/imagery/0a7c21432508bd807a61b82bdf2402ba"
        ));
        assert!(portrait_path("/gameplay/creator-portrait/8ce8d3479cf2"));
        for refused in [
            "/gameplay/imagery/",
            "/gameplay/imagery/../secret",
            "/gameplay/launcher-status",
            "http://elsewhere/x.png",
            "/gameplay/imagery/a b",
        ] {
            assert!(!portrait_path(refused), "{refused}");
        }
    }

    #[test]
    fn plain_and_chunked_answers_read_and_refusals_fail() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nhello";
        assert_eq!(body(plain, 100).unwrap(), b"hello");
        let chunked =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
        assert_eq!(body(chunked, 100).unwrap(), b"hello world");
        assert_eq!(
            reason(&body(b"HTTP/1.1 404 Not Found\r\n\r\n", 100).unwrap_err()),
            "http_404"
        );
        assert_eq!(reason(&body(plain, 3).unwrap_err()), "too_large");
        assert!(body(plain, 3).is_err());
    }
}
