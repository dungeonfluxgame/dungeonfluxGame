//! One owned loopback token response for transport-boundary tests; captured requests stay private.

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
};

type Response = Option<(u16, Vec<u8>)>;

pub(super) struct Endpoint {
    pub(super) url: String,
    pub(super) request: oneshot::Receiver<String>,
    pub(super) release: Option<oneshot::Sender<Response>>,
    task: JoinHandle<()>,
}

impl Endpoint {
    pub(super) async fn open() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/token", listener.local_addr().unwrap());
        let (sent, request) = oneshot::channel();
        let (release, wait) = oneshot::channel::<Response>();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    return;
                }
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() <= 16384);
                let text = String::from_utf8_lossy(&bytes);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if body.len() >= length {
                        break;
                    }
                }
            }
            sent.send(String::from_utf8(bytes).unwrap()).unwrap();
            if let Some((status, body)) = wait.await.unwrap() {
                let location = if status == 302 {
                    "Location: /unexpected-follow\r\n"
                } else {
                    ""
                };
                let head = format!(
                    "HTTP/1.1 {status} Fixture\r\n{location}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
            } else {
                std::future::pending::<()>().await;
            }
        });
        Self {
            url,
            request,
            release: Some(release),
            task,
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.task.abort();
    }
}
