//! The one PostgreSQL transport policy for df-persistence connections (security review,
//! 2026-10-24): TLS with certificate and hostname verification for every host that is not
//! loopback; plaintext only to 127.0.0.1, ::1, localhost or a Unix socket, and only when the
//! configuration says so explicitly. Shared so the production composition uses this connector
//! rather than a second one.

use std::{
    future::Future,
    io,
    net::IpAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use rustls_pki_types::{CertificateDer, pem::PemObject};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_postgres::{
    Client, Config, Socket,
    config::{Host, SslMode},
    tls::{ChannelBinding, MakeTlsConnect, TlsConnect, TlsStream},
};

/// How a store reaches PostgreSQL.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PostgresTransport {
    /// TLS verified against the CA certificates in this PEM file (a managed database's CA).
    VerifiedTls { ca_file: PathBuf },
    /// No TLS; refused unless every configured host is loopback or a Unix socket.
    PlaintextLoopbackOnly,
}

/// Why a transport was refused before any connection was attempted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportRefusal {
    /// Plaintext was requested for a host that is not loopback.
    PlaintextToRemoteHost,
    NoHost,
    /// The CA file could not be read or held no certificate.
    CaUnreadable,
}

fn is_loopback(host: &Host) -> bool {
    match host {
        #[cfg(unix)]
        Host::Unix(_) => true,
        Host::Tcp(name) => {
            name == "localhost"
                || name
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
    }
}

/// Whether every configured host is loopback or a Unix socket (a database on this machine).
pub fn all_hosts_loopback(config: &Config) -> bool {
    let hosts = config.get_hosts();
    !hosts.is_empty() && hosts.iter().all(is_loopback)
}

/// Checks the transport against the configured hosts.
pub fn check_transport(
    config: &Config,
    transport: &PostgresTransport,
) -> Result<(), TransportRefusal> {
    let hosts = config.get_hosts();
    if hosts.is_empty() {
        return Err(TransportRefusal::NoHost);
    }
    match transport {
        PostgresTransport::VerifiedTls { .. } => Ok(()),
        PostgresTransport::PlaintextLoopbackOnly if hosts.iter().all(is_loopback) => Ok(()),
        PostgresTransport::PlaintextLoopbackOnly => Err(TransportRefusal::PlaintextToRemoteHost),
    }
}

/// A rustls connector that verifies the server certificate chain and its host name.
#[derive(Clone)]
pub struct VerifiedTlsConnector {
    config: Arc<ClientConfig>,
}

impl VerifiedTlsConnector {
    pub fn from_ca_file(ca_file: &Path) -> Result<Self, TransportRefusal> {
        let mut roots = RootCertStore::empty();
        for certificate in
            CertificateDer::pem_file_iter(ca_file).map_err(|_| TransportRefusal::CaUnreadable)?
        {
            let certificate = certificate.map_err(|_| TransportRefusal::CaUnreadable)?;
            roots
                .add(certificate)
                .map_err(|_| TransportRefusal::CaUnreadable)?;
        }
        if roots.is_empty() {
            return Err(TransportRefusal::CaUnreadable);
        }
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(Self {
            config: Arc::new(config),
        })
    }
}

/// The verified TLS stream tokio-postgres speaks over.
pub struct VerifiedTlsStream(tokio_rustls::client::TlsStream<Socket>);

impl AsyncRead for VerifiedTlsStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for VerifiedTlsStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(context)
    }
}

impl TlsStream for VerifiedTlsStream {
    fn channel_binding(&self) -> ChannelBinding {
        // SCRAM channel binding is optional; the verified certificate already binds the server.
        ChannelBinding::none()
    }
}

/// One connection's TLS handshake, bound to the host name it must verify.
pub struct VerifiedTlsConnect {
    config: Arc<ClientConfig>,
    server_name: ServerName<'static>,
}

impl TlsConnect<Socket> for VerifiedTlsConnect {
    type Stream = VerifiedTlsStream;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = io::Result<VerifiedTlsStream>> + Send>>;

    fn connect(self, stream: Socket) -> Self::Future {
        Box::pin(async move {
            tokio_rustls::TlsConnector::from(self.config)
                .connect(self.server_name, stream)
                .await
                .map(VerifiedTlsStream)
        })
    }
}

impl MakeTlsConnect<Socket> for VerifiedTlsConnector {
    type Stream = VerifiedTlsStream;
    type TlsConnect = VerifiedTlsConnect;
    type Error = io::Error;

    fn make_tls_connect(&mut self, domain: &str) -> io::Result<VerifiedTlsConnect> {
        let server_name = ServerName::try_from(domain.to_owned())
            .map_err(|_| io::Error::other("invalid PostgreSQL host name"))?;
        Ok(VerifiedTlsConnect {
            config: Arc::clone(&self.config),
            server_name,
        })
    }
}

/// A connection's driver; it must be polled (spawned) for the client to make progress.
pub type Driver = Pin<Box<dyn Future<Output = Result<(), tokio_postgres::Error>> + Send>>;

#[derive(Debug)]
pub enum ConnectError {
    Refused(TransportRefusal),
    Postgres(tokio_postgres::Error),
}

/// Connects under the transport policy. TLS connections require TLS (`sslmode=require`) and
/// verify the chain and host name with rustls.
pub async fn connect(
    config: &Config,
    transport: &PostgresTransport,
) -> Result<(Client, Driver), ConnectError> {
    check_transport(config, transport).map_err(ConnectError::Refused)?;
    match transport {
        PostgresTransport::VerifiedTls { ca_file } => {
            let connector =
                VerifiedTlsConnector::from_ca_file(ca_file).map_err(ConnectError::Refused)?;
            let mut config = config.clone();
            config.ssl_mode(SslMode::Require);
            let (client, connection) = config
                .connect(connector)
                .await
                .map_err(ConnectError::Postgres)?;
            Ok((client, Box::pin(connection)))
        }
        PostgresTransport::PlaintextLoopbackOnly => {
            let (client, connection) = config
                .connect(tokio_postgres::NoTls)
                .await
                .map_err(ConnectError::Postgres)?;
            Ok((client, Box::pin(connection)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(host: &str) -> Config {
        let mut config = Config::new();
        config.host(host).port(5432).user("postgres");
        config
    }

    #[test]
    fn plaintext_to_a_remote_host_is_refused() {
        for host in ["db.example.test", "10.0.0.5", "192.168.1.20"] {
            assert_eq!(
                check_transport(&config(host), &PostgresTransport::PlaintextLoopbackOnly),
                Err(TransportRefusal::PlaintextToRemoteHost),
                "{host}"
            );
        }
    }

    #[test]
    fn plaintext_to_loopback_or_a_unix_socket_is_allowed() {
        let hosts: &[&str] = if cfg!(unix) {
            &["127.0.0.1", "::1", "localhost", "/var/run/postgresql"]
        } else {
            &["127.0.0.1", "::1", "localhost"]
        };
        for host in hosts {
            assert_eq!(
                check_transport(&config(host), &PostgresTransport::PlaintextLoopbackOnly),
                Ok(()),
                "{host}"
            );
        }
    }

    #[test]
    fn one_remote_host_among_several_refuses_plaintext() {
        let mut mixed = config("127.0.0.1");
        mixed.host("db.example.test");
        assert_eq!(
            check_transport(&mixed, &PostgresTransport::PlaintextLoopbackOnly),
            Err(TransportRefusal::PlaintextToRemoteHost)
        );
    }

    #[test]
    fn a_missing_ca_file_is_refused_before_connecting() {
        assert!(matches!(
            VerifiedTlsConnector::from_ca_file(Path::new("does-not-exist.pem")),
            Err(TransportRefusal::CaUnreadable)
        ));
    }
}

#[cfg(test)]
#[path = "postgres_tls_handshake_tests.rs"]
mod handshake_tests;
