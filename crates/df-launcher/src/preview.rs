//! `dungeonflux --ui-preview <127.0.0.1:port>`: the same launcher pages over loopback HTTP and
//! no windows, for checking the pages in a browser during development and tests. The table
//! display opens in the default browser in this mode.
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};

use crate::{app::App, form::MAXIMUM_FORM_BYTES, ui};

pub fn serve(app: Arc<App>, address: SocketAddr) -> io::Result<()> {
    if !address.ip().is_loopback() {
        return Err(io::Error::other(
            "the preview listens on a loopback address only",
        ));
    }
    let listener = TcpListener::bind(address)?;
    let local = listener.local_addr()?;
    println!("PREVIEW url=http://{local}/ data={}", app.data().display());
    let hosts = [local.to_string(), format!("localhost:{}", local.port())];
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            continue;
        };
        if let Err(error) = answer(&app, stream, &hosts) {
            eprintln!("PREVIEW request_failed error={error}");
        }
    }
    Ok(())
}

/// `hosts` are the loopback names this listener answers to (`127.0.0.1:port`, `localhost:port`).
fn answer(app: &Arc<App>, stream: TcpStream, hosts: &[String]) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut parts = request_line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return write(stream, 400, "text/plain", b"Bad request", None);
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut length = 0usize;
    let mut host_ok = false;
    let mut origin_ok = true;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        header_bytes += read;
        if read == 0 || line == "\r\n" || line == "\n" || header_bytes > 16 * 1024 {
            break;
        }
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or(usize::MAX),
            // Refuses DNS-rebinding and cross-site form posts aimed at the preview port.
            "host" => host_ok = hosts.iter().any(|host| value == host),
            "origin" => {
                origin_ok =
                    value == "null" || hosts.iter().any(|host| value == format!("http://{host}"));
            }
            _ => {}
        }
    }
    if !host_ok || (method == "POST" && !origin_ok) {
        return write(stream, 403, "text/plain", b"Refused", None);
    }
    if length > MAXIMUM_FORM_BYTES {
        return write(stream, 413, "text/plain", b"Too large", None);
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let page = ui::handle(app, &method, &target, &body);
    write(
        stream,
        page.status,
        page.content_type,
        &page.body,
        page.location.as_deref(),
    )
}

fn write(
    mut stream: TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    location: Option<&str>,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        303 => "See Other",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Payload Too Large",
        422 => "Unprocessable Content",
        _ => "Response",
    };
    let location = location
        .map(|to| format!("Location: {to}\r\n"))
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{location}Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    stream.flush()
}
