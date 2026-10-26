//! Owned native byte proxy for generated tonic clients using the real WebSocket bridge.

use df_protocol::common as rpc;
use futures::{SinkExt, StreamExt};
use std::{io, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

type Error = Box<dyn std::error::Error + Send + Sync>;
const MAXIMUM_HTTP_BYTES: u64 = 4 * 1024 * 1024;

pub(super) async fn get(url: &str, path: &str) -> Result<Vec<u8>, Error> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let authority = authority(url)?;
        if !path.starts_with('/') || path.contains(['\r', '\n']) {
            return Err(io::Error::other("invalid asset path").into());
        }
        let mut stream = TcpStream::connect(authority).await?;
        stream
            .write_all(
                format!("GET {path} HTTP/1.0\r\nHost: {authority}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await?;
        let mut response = Vec::new();
        stream
            .take(MAXIMUM_HTTP_BYTES + 1)
            .read_to_end(&mut response)
            .await?;
        if u64::try_from(response.len())? > MAXIMUM_HTTP_BYTES {
            return Err(io::Error::other("HTTP asset exceeds bound").into());
        }
        let split = response
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .ok_or_else(|| io::Error::other("HTTP response malformed"))?;
        let head = std::str::from_utf8(&response[..split])?;
        if head.split_whitespace().nth(1) != Some("200") {
            return Err(io::Error::other("HTTP asset unavailable").into());
        }
        Ok(response[split + 4..].to_vec())
    })
    .await?
}

fn authority(url: &str) -> Result<&str, io::Error> {
    let authority = url
        .strip_prefix("http://127.0.0.1:")
        .ok_or_else(|| io::Error::other("load URL must be IPv4 loopback HTTP"))?;
    let port: u16 = authority
        .parse()
        .map_err(|_| io::Error::other("load port invalid"))?;
    if port == 0 || port == 63309 {
        return Err(io::Error::other("load port forbidden"));
    }
    url.strip_prefix("http://")
        .ok_or_else(|| io::Error::other("load URL invalid"))
}

async fn forward(stream: TcpStream, url: String) -> Result<(), Error> {
    let mut request =
        format!("{}/gameplay/tunnel", url.replacen("http://", "ws://", 1)).into_client_request()?;
    request
        .headers_mut()
        .insert("origin", HeaderValue::from_str(&url)?);
    let configuration = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
        .max_message_size(Some(df_rpc_bridge::MESSAGE_BYTES))
        .max_frame_size(Some(df_rpc_bridge::MESSAGE_BYTES));
    let (socket, _) = tokio::time::timeout(
        Duration::from_secs(20),
        tokio_tungstenite::connect_async_with_config(request, Some(configuration), false),
    )
    .await??;
    let (mut sender, mut receiver) = socket.split();
    let (mut read, mut write) = stream.into_split();
    let mut buffer = vec![0u8; df_rpc_bridge::FRAME_BYTES];
    loop {
        tokio::select! {
            read = read.read(&mut buffer) => {
                let count = read?;
                if count == 0 { break; }
                sender.send(Message::Binary(buffer[..count].to_vec().into())).await?;
            }
            incoming = receiver.next() => {
                match incoming {
                    Some(Ok(Message::Binary(bytes))) => write.write_all(&bytes).await?,
                    Some(Ok(Message::Ping(bytes))) => sender.send(Message::Pong(bytes)).await?,
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(error)) => return Err(error.into()),
                    Some(Ok(Message::Text(_))) => return Err(io::Error::other("nonbinary tunnel data").into()),
                    Some(Ok(_)) => {},
                }
            }
        }
    }
    Ok(())
}

/// A channel whose tunnel and view tasks are cancelled when the connection is dropped.
pub(super) struct Connection {
    pub(super) channel: tonic::transport::Channel,
    tasks: JoinSet<Result<(), Error>>,
}

impl Connection {
    pub(super) async fn connect(url: &str) -> Result<Self, Error> {
        authority(url)?;
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let url = url.to_owned();
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let (stream, _) = listener.accept().await?;
            forward(stream, url).await
        });
        let endpoint = tonic::transport::Endpoint::from_shared(format!("http://{address}"))?
            .connect_timeout(Duration::from_secs(20));
        let channel = tokio::time::timeout(Duration::from_secs(20), endpoint.connect()).await??;
        Ok(Self { channel, tasks })
    }

    pub(super) async fn watch(
        &mut self,
        body: rpc::WatchViewRequest,
        binding: &str,
    ) -> Result<tokio::sync::watch::Receiver<Option<rpc::ViewMessage>>, Error> {
        let mut request = tonic::Request::new(body);
        request
            .metadata_mut()
            .insert("x-df-local-binding", binding.parse()?);
        let mut client =
            rpc::session_service_client::SessionServiceClient::new(self.channel.clone())
                .max_decoding_message_size(df_rpc_bridge::RPC_MESSAGE_BYTES);
        let mut stream = tokio::time::timeout(Duration::from_secs(20), client.watch(request))
            .await??
            .into_inner();
        let first = tokio::time::timeout(Duration::from_secs(20), stream.message())
            .await??
            .ok_or_else(|| io::Error::other("watch closed before initial view"))?;
        let (sender, receiver) = tokio::sync::watch::channel(Some(first));
        self.tasks.spawn(async move {
            while let Some(view) = stream.message().await? {
                sender
                    .send(Some(view))
                    .map_err(|_| io::Error::other("watch receiver dropped"))?;
            }
            Err(io::Error::other("watch ended during workload").into())
        });
        Ok(receiver)
    }

    pub(super) fn check(&mut self) -> Result<(), Error> {
        if let Some(result) = self.tasks.try_join_next() {
            result??;
            return Err(io::Error::other("transport stopped during workload").into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn load_urls_refuse_non_loopback_and_reserved_ports() {
        for url in [
            "http://127.0.0.1:63309",
            "http://example.com:80",
            "http://127.0.0.1:0",
            "http://127.0.0.1:63411/path",
        ] {
            assert!(authority(url).is_err());
        }
        assert_eq!(
            authority("http://127.0.0.1:63411").unwrap(),
            "127.0.0.1:63411"
        );
    }
}
