//! df-docker-proxy: listens on a unix socket for the trusted runner, checks every request against the policy, and forwards
//! the allowed ones to the Docker daemon's socket. One request per connection (`Connection: close`), bounded head and
//! body, bounded concurrency. It reads the digest allow-list on every request, so a verified release takes effect at once
//! and an image removed from the list stops being startable at once.
//!
//! usage: df-docker-proxy --listen /run/dungeonflux/runner/docker.sock --docker /var/run/docker.sock \
//!                        --allowed-images /etc/dungeonflux/allowed-images
//! The allow-list file has lines `key=reference`; the proxy takes the `campaign-runtime` lines.
//! STATUS: the policy and the HTTP framing are unit-tested (cargo test); the socket server below has NOT been run
//! (it needs Linux and a Docker daemon).

#[cfg(unix)]
mod server {
    use df_docker_proxy::http::{MAX_HEAD_BYTES, forward_head, parse_head, refusal_response};
    use df_docker_proxy::policy::{FsProbe, Policy, canonical_body};
    use std::io::{Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const MAX_CONNECTIONS: usize = 16;

    struct Settings {
        listen: String,
        docker: String,
        allowed_images: String,
    }

    fn settings() -> Result<Settings, String> {
        let mut listen = None;
        let mut docker = None;
        let mut allowed = None;
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
            match flag.as_str() {
                "--listen" => listen = Some(value),
                "--docker" => docker = Some(value),
                "--allowed-images" => allowed = Some(value),
                other => return Err(format!("unknown argument {other}")),
            }
        }
        Ok(Settings {
            listen: listen.ok_or("--listen is required")?,
            docker: docker.ok_or("--docker is required")?,
            allowed_images: allowed.ok_or("--allowed-images is required")?,
        })
    }

    fn read_allowed(path: &str) -> Vec<String> {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        text.lines()
            .filter_map(|line| line.strip_prefix("campaign-runtime="))
            .map(|r| r.trim().to_string())
            .collect()
    }

    fn read_head(stream: &mut UnixStream) -> Result<Vec<u8>, &'static str> {
        let mut head = Vec::with_capacity(1024);
        let mut byte = [0u8; 1];
        while head.len() < MAX_HEAD_BYTES {
            match stream.read(&mut byte) {
                Ok(1) => head.push(byte[0]),
                _ => return Err("connection closed inside the request head"),
            }
            if head.ends_with(b"\r\n\r\n") {
                return Ok(head);
            }
        }
        Err("request head too large")
    }

    fn handle(mut client: UnixStream, settings: &Settings) {
        let _ = client.set_read_timeout(Some(Duration::from_secs(10)));
        let _ = client.set_write_timeout(Some(Duration::from_secs(30)));
        let raw = match read_head(&mut client) {
            Ok(raw) => raw,
            Err(why) => {
                let _ = client.write_all(&refusal_response(why));
                return;
            }
        };
        let head = match parse_head(&raw) {
            Ok(head) => head,
            Err(why) => {
                eprintln!("df-docker-proxy: refused: {}", why.0);
                let _ = client.write_all(&refusal_response(why.0));
                return;
            }
        };
        let mut body = vec![0u8; head.content_length];
        if client.read_exact(&mut body).is_err() {
            let _ = client.write_all(&refusal_response("short request body"));
            return;
        }
        let policy = Policy::standard(read_allowed(&settings.allowed_images), Arc::new(FsProbe));
        // The body that is forwarded is the canonical serialisation of what the policy checked (never the received bytes), with its
        // own Content-Length: duplicate keys, escapes and odd spacing cannot make the daemon decode something else.
        let forwarded_body = match canonical_body(&policy, &head.method, &head.target, &body) {
            Ok(forwarded) => forwarded,
            Err(refusal) => {
                eprintln!(
                    "df-docker-proxy: refused {} {}: {}",
                    head.method,
                    head.target.split('?').next().unwrap_or(""),
                    refusal.0
                );
                let _ = client.write_all(&refusal_response(&refusal.0));
                return;
            }
        };
        let mut upstream = match UnixStream::connect(&settings.docker) {
            Ok(upstream) => upstream,
            Err(error) => {
                eprintln!("df-docker-proxy: cannot reach the Docker daemon: {error}");
                let _ = client.write_all(&refusal_response("the Docker daemon is unreachable"));
                return;
            }
        };
        let _ = upstream.set_read_timeout(Some(Duration::from_secs(330)));
        if upstream
            .write_all(&forward_head(&head, forwarded_body.len()))
            .and_then(|_| upstream.write_all(&forwarded_body))
            .is_err()
        {
            return;
        }
        // The daemon closes the connection after the response (Connection: close); relay it unchanged.
        let _ = std::io::copy(&mut upstream, &mut client);
    }

    pub fn run() -> Result<(), String> {
        let settings = Arc::new(settings()?);
        let _ = std::fs::remove_file(&settings.listen);
        let listener = UnixListener::bind(&settings.listen)
            .map_err(|e| format!("cannot listen on {}: {e}", settings.listen))?;
        // Only the runner's group may connect: the socket is 0660 and the directory is owned by the runner's group.
        std::fs::set_permissions(&settings.listen, std::fs::Permissions::from_mode(0o660))
            .map_err(|e| e.to_string())?;
        let active = Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                active.fetch_sub(1, Ordering::SeqCst);
                let mut stream = stream;
                let _ = stream.write_all(&refusal_response("too many connections"));
                continue;
            }
            let settings = Arc::clone(&settings);
            let active = Arc::clone(&active);
            std::thread::spawn(move || {
                handle(stream, &settings);
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
        Ok(())
    }
}

#[cfg(unix)]
fn main() {
    if let Err(message) = server::run() {
        eprintln!("df-docker-proxy: {message}");
        std::process::exit(1);
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!(
        "df-docker-proxy serves a unix socket and runs on Linux only; the policy is tested with `cargo test`."
    );
    std::process::exit(2);
}
