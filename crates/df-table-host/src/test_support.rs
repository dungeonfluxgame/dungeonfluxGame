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
use crate::observe::{HostEventSink, WallClock};
use crate::playground::PlaygroundFiles;
use crate::server;
use crate::state::AppState;

/// The kind files most tests start with: a character and a marker.
pub const STARTER_FILES: [(&str, &str); 2] = [
    ("character.json", CHARACTER_KIND),
    ("marker.json", MARKER_KIND),
];

/// A canvas being served, stopped when this value is dropped.
pub struct Running {
    pub port: u16,
    /// The host's state, so a test can tick the canvas itself instead of waiting for the clock.
    pub state: Arc<AppState>,
    clock: Option<tokio::task::JoinHandle<()>>,
    directory: TempDir,
    playground: TempDir,
    stop: Option<oneshot::Sender<()>>,
    server: JoinHandle<()>,
}

impl Running {
    /// Turns the playground's voice switches on (`tts` and `stt` start off), for a test of the
    /// calls that follow.
    pub fn switch_speech_on(&self) {
        self.state
            .playground()
            .change_all(
                vec![
                    df_playground::controls::ControlsChange::Tts(true),
                    df_playground::controls::ControlsChange::Stt(true),
                ],
                &[],
                0,
                |_| {},
            )
            .expect("the switches are changed");
    }

    /// Starts the host's real clock, for a test of travel that runs on its own.
    pub fn start_clock(&mut self) {
        if self.clock.is_none() {
            self.clock = Some(crate::server::spawn_clock(Arc::clone(&self.state)));
        }
    }

    pub fn kind_directory(&self) -> &Path {
        self.directory.path()
    }

    pub fn write_file(&self, file_name: &str, text: &str) {
        std::fs::write(self.directory.path().join(file_name), text).expect("writes a file");
    }

    /// Writes a playground asset (`index.html`, `theme.css`, or `fixtures/<name>.json`). A fixture
    /// is written the way `fixture write` does it: the host then reconciles what it shows with the
    /// files (a read never does) and tells the pages.
    pub fn write_asset(&self, relative_path: &str, text: &str) {
        let path = self.playground.path().join("assets").join(relative_path);
        std::fs::create_dir_all(path.parent().expect("has a parent")).expect("makes a directory");
        std::fs::write(path, text).expect("writes an asset");
        if relative_path.starts_with("fixtures/") {
            self.state.playground().fixture_files_changed();
            crate::playground::reconcile_fixtures(&self.state).expect("lists the fixtures");
            self.state.announce_change();
        }
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
        if let Some(clock) = self.clock.take() {
            clock.abort();
        }
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
    let sink = HostEventSink::new(Arc::clone(&log), Arc::new(WallClock));
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
    let served_state = Arc::clone(&state);
    let server = tokio::spawn(async move {
        let _ = server::serve_listener(listener, served_state, async {
            let _ = stopped.await;
        })
        .await;
    });
    Running {
        port,
        state,
        clock: None,
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

/// A one-region layout naming `name`, bound to `picture` whose bytes are its own name.
pub fn layout_of(picture: &str, name: &str) -> df_scene_layout::SceneLayout {
    use df_scene_layout::{PictureBinding, RegionGeometry, SceneLayout, SceneRegion, region_id};
    let binding = PictureBinding {
        name: picture.to_owned(),
        sha256: df_imagery::oneshot_segment::picture_sha256(picture.as_bytes()),
        width_px: 100,
        height_px: 50,
    };
    let geometry = RegionGeometry {
        left_permille: 300,
        top_permille: 200,
        right_permille: 600,
        bottom_permille: 1000,
        head_x_permille: 450,
        head_y_permille: 200,
    };
    let region = SceneRegion::new(region_id(name.as_bytes()), geometry, 900)
        .unwrap()
        .with_label(Some(name.to_owned()));
    SceneLayout::new(binding, "sam-3.1", "person", vec![region])
}
