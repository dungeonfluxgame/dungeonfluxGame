//! A running canvas on an ephemeral port, for the host's tests.

use std::path::Path;
use std::sync::Arc;

use df_table::canvas::Canvas;
use df_table::testkit::{CHARACTER_KIND, MARKER_KIND};
use tempfile::TempDir;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::event_log::EventLog;
use crate::kind_files::load_kind_files;
use crate::playground::PlaygroundFiles;
use crate::server;
use crate::sink::HostSink;
use crate::state::AppState;

/// The kind files most tests start with: a character and a marker.
pub const STARTER_FILES: [(&str, &str); 2] = [
    ("character.json", CHARACTER_KIND),
    ("marker.json", MARKER_KIND),
];

/// A canvas being served, stopped when this value is dropped.
pub struct Running {
    pub port: u16,
    directory: TempDir,
    playground: TempDir,
    stop: Option<oneshot::Sender<()>>,
    server: JoinHandle<()>,
}

impl Running {
    pub fn kind_directory(&self) -> &Path {
        self.directory.path()
    }

    pub fn write_file(&self, file_name: &str, text: &str) {
        std::fs::write(self.directory.path().join(file_name), text).expect("writes a file");
    }

    /// Writes a playground asset (`index.html`, `theme.css`, or `fixtures/<name>.json`).
    pub fn write_asset(&self, relative_path: &str, text: &str) {
        let path = self.playground.path().join("assets").join(relative_path);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes a directory");
        std::fs::write(path, text).expect("writes an asset");
    }

    /// A path under the playground's temporary directory (`assets/...` or `pkg/...`).
    pub fn playground_path(&self, relative_path: &str) -> std::path::PathBuf {
        self.playground.path().join(relative_path)
    }

    /// Writes a file of the playground's compiled package.
    pub fn write_package_file(&self, name: &str, bytes: &[u8]) {
        let path = self.playground.path().join("pkg").join(name);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes a directory");
        std::fs::write(path, bytes).expect("writes a package file");
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.server.abort();
    }
}

/// Starts a canvas whose kind directory holds `files` (file name, text). Moves are recorded, so
/// tests can see every event.
pub async fn start(files: &[(&str, &str)]) -> Running {
    let directory = TempDir::new().expect("makes a temporary directory");
    for (name, text) in files {
        std::fs::write(directory.path().join(name), text).expect("writes a kind file");
    }
    let registry = load_kind_files(directory.path()).expect("the test kinds are valid");
    let log = Arc::new(EventLog::default());
    let sink = HostSink::new(Arc::clone(&log), true);
    let playground = TempDir::new().expect("makes a temporary directory");
    let playground_files = PlaygroundFiles::new(
        playground.path().join("assets"),
        playground.path().join("pkg"),
    );
    let state = Arc::new(AppState::new(
        Canvas::new(registry),
        sink,
        None,
        directory.path().to_path_buf(),
        playground_files,
    ));
    let listener = server::bind(0).await.expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        let _ = server::serve_listener(listener, state, async {
            let _ = stopped.await;
        })
        .await;
    });
    Running {
        port,
        directory,
        playground,
        stop: Some(stop),
        server,
    }
}

/// A port nothing listens on: bind one, note it, and drop the listener.
pub async fn closed_port() -> u16 {
    let listener = server::bind(0).await.expect("binds an ephemeral port");
    listener.local_addr().expect("has an address").port()
}

/// Serves one connection: reads the request head, sends `answer` exactly as given, and closes.
/// Returns the port. For testing how a client copes with an answer a real host would not send.
pub async fn canned_server(answer: Vec<u8>) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = server::bind(0).await.expect("binds an ephemeral port");
    let port = listener.local_addr().expect("has an address").port();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let mut head = Vec::new();
        let mut chunk = [0_u8; 512];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(count) => head.extend_from_slice(&chunk[..count]),
            }
        }
        let _ = stream.write_all(&answer).await;
        let _ = stream.shutdown().await;
    });
    port
}

/// Sends raw bytes to the port and reads everything the server answers until it closes.
pub async fn raw_exchange(port: u16, request: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connects");
    stream.write_all(request.as_bytes()).await.expect("sends");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.expect("reads");
    answer
}

/// A POST with a JSON body and the headers a local client would send, plus `extra_headers`.
pub fn post_request(path: &str, body: &str, extra_headers: &str) -> String {
    format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         {extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
