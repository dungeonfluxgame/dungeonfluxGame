use std::{io, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};

#[derive(Clone, Copy, Debug)]
pub struct FaultProfile {
    pub latency: Duration,
    pub jitter: Duration,
    pub slow_reader_delay: Duration,
    pub chunk_bytes: usize,
    pub seed: u64,
    pub hard_drop_after_bytes: Option<usize>,
    pub half_close_after_bytes: Option<usize>,
    pub blackhole_after_bytes: Option<usize>,
}

impl Default for FaultProfile {
    fn default() -> Self {
        Self {
            latency: Duration::ZERO,
            jitter: Duration::ZERO,
            slow_reader_delay: Duration::ZERO,
            chunk_bytes: 4096,
            seed: 1,
            hard_drop_after_bytes: None,
            half_close_after_bytes: None,
            blackhole_after_bytes: None,
        }
    }
}

/// Test-owned loopback TCP proxy for reproducible transport faults.
pub struct FaultProxy {
    address: SocketAddr,
    task: JoinHandle<()>,
}

impl FaultProxy {
    pub async fn start(upstream: SocketAddr, profile: FaultProfile) -> io::Result<Self> {
        if profile.chunk_bytes == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fault proxy chunk size must be nonzero",
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let address = listener.local_addr()?;
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((client, _)) = accepted else { break };
                        connections.spawn(async move {
                            if let Ok(server) = TcpStream::connect(upstream).await {
                                let _ = relay(client, server, profile).await;
                            }
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
            connections.abort_all();
        });
        Ok(Self { address, task })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Drop for FaultProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn relay(client: TcpStream, server: TcpStream, profile: FaultProfile) -> io::Result<()> {
    let (mut client_reader, mut client_writer) = client.into_split();
    let (mut server_reader, mut server_writer) = server.into_split();
    tokio::select! {
        result = forward(&mut client_reader, &mut server_writer, profile, profile.seed) => result,
        result = forward(&mut server_reader, &mut client_writer, profile, profile.seed ^ 0x9e37_79b9) => result,
    }
}

async fn forward<Reader, Writer>(
    reader: &mut Reader,
    writer: &mut Writer,
    profile: FaultProfile,
    mut random: u64,
) -> io::Result<()>
where
    Reader: AsyncRead + Unpin,
    Writer: AsyncWrite + Unpin,
{
    let mut transferred_bytes = 0;
    let mut half_closed = false;
    let mut buffer = vec![0; profile.chunk_bytes];
    loop {
        let count = reader.read(&mut buffer).await?;
        if count == 0 {
            writer.shutdown().await?;
            return Ok(());
        }
        if profile
            .hard_drop_after_bytes
            .is_some_and(|limit| transferred_bytes >= limit)
        {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "injected hard drop",
            ));
        }
        if !half_closed
            && profile
                .half_close_after_bytes
                .is_some_and(|limit| transferred_bytes >= limit)
        {
            writer.shutdown().await?;
            half_closed = true;
            continue;
        }
        if half_closed {
            continue;
        }
        if profile
            .blackhole_after_bytes
            .is_some_and(|limit| transferred_bytes >= limit)
        {
            continue;
        }
        random = random
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        let jitter_nanos = profile.jitter.as_nanos();
        let jitter = if jitter_nanos == 0 {
            Duration::ZERO
        } else {
            Duration::from_nanos((u128::from(random) % (jitter_nanos + 1)) as u64)
        };
        let delay = profile.latency + jitter + profile.slow_reader_delay;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        writer.write_all(&buffer[..count]).await?;
        transferred_bytes = transferred_bytes.saturating_add(count);
    }
}
