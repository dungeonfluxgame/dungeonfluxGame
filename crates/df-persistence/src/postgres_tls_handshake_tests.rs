//! A real TLS handshake through `postgres_tls::connect`, against a loopback server that speaks
//! PostgreSQL's `SSLRequest` and then TLS with a certificate issued by a CA generated inside each
//! test. Nothing here is a fixture file and no key is ever written down: the CA certificate goes to
//! a temporary file the connector reads (it takes a file, as a managed database's CA is one), and
//! the private keys stay in memory. The server only proves what the client verified: it records
//! whether the handshake completed and whether the client then sent its startup message.

use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};
use rustls::{ServerConfig, pki_types::PrivateKeyDer};
use rustls_pki_types::CertificateDer;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_postgres::Config;
use tokio_rustls::TlsAcceptor;

use super::{ConnectError, PostgresTransport, connect};

/// What the server saw of one connection attempt.
#[derive(Debug, Eq, PartialEq)]
struct Saw {
    handshake_completed: bool,
    startup_message_received: bool,
}

struct TestCa {
    certificate: rcgen::Certificate,
    key: KeyPair,
}

impl TestCa {
    fn new(name: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let certificate = params.self_signed(&key).unwrap();
        Self { certificate, key }
    }

    /// A server certificate for `dns_name`, issued by this CA, as a rustls server config.
    fn server_config(&self, dns_name: &str) -> Arc<ServerConfig> {
        let key = KeyPair::generate().unwrap();
        let params = CertificateParams::new(vec![dns_name.to_owned()]).unwrap();
        let leaf = params
            .signed_by(&key, &self.certificate, &self.key)
            .unwrap();
        let chain: Vec<CertificateDer<'static>> = vec![leaf.der().clone()];
        let key = PrivateKeyDer::try_from(key.serialize_der()).unwrap();
        Arc::new(
            ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .unwrap(),
        )
    }

    /// The CA certificate in a temporary file, for `PostgresTransport::VerifiedTls`.
    fn pem_file(&self, label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "df-tls-test-{label}-{}-{}.pem",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, self.certificate.pem()).unwrap();
        path
    }
}

/// Accepts one connection, answers `SSLRequest` with `S`, runs the TLS handshake and then waits
/// for the client's startup message.
async fn serve_once(
    listener: TcpListener,
    config: Arc<ServerConfig>,
) -> tokio::task::JoinHandle<Saw> {
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 8];
        socket.read_exact(&mut request).await.unwrap();
        // Length 8, then the SSLRequest code 80877103.
        assert_eq!(request, [0, 0, 0, 8, 4, 210, 22, 47]);
        socket.write_all(b"S").await.unwrap();
        match TlsAcceptor::from(config).accept(socket).await {
            Err(_) => Saw {
                handshake_completed: false,
                startup_message_received: false,
            },
            Ok(mut tls) => {
                let mut length = [0_u8; 4];
                let received = tls.read_exact(&mut length).await.is_ok();
                Saw {
                    handshake_completed: true,
                    startup_message_received: received,
                }
            }
        }
    })
}

/// The client config for a database that is `db.example.test` by name and loopback by address.
fn client_config(address: SocketAddr) -> Config {
    let mut config = Config::new();
    config
        .host("db.example.test")
        .hostaddr(address.ip())
        .port(address.port())
        .user("postgres")
        .dbname("postgres")
        .connect_timeout(std::time::Duration::from_secs(5));
    config
}

async fn attempt(client_trusts: &TestCa, server_ca: &TestCa, certificate_name: &str) -> Saw {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = serve_once(listener, server_ca.server_config(certificate_name)).await;
    let ca_file = client_trusts.pem_file("client");
    let outcome = connect(
        &client_config(address),
        &PostgresTransport::VerifiedTls {
            ca_file: ca_file.clone(),
        },
    )
    .await;
    // The test server never answers the startup message, so a connection never completes; what
    // matters is how far the handshake got.
    assert!(matches!(outcome, Err(ConnectError::Postgres(_))));
    let _removed = std::fs::remove_file(ca_file);
    server.await.unwrap()
}

fn run<F: std::future::Future<Output = ()>>(test: F) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(test);
}

#[test]
fn a_server_certificate_from_the_trusted_ca_for_the_right_name_completes_the_handshake() {
    run(async {
        let ca = TestCa::new("DungeonFlux test CA");
        let saw = attempt(&ca, &ca, "db.example.test").await;
        assert_eq!(
            saw,
            Saw {
                handshake_completed: true,
                startup_message_received: true
            }
        );
    });
}

#[test]
fn a_certificate_from_a_different_ca_is_refused_by_the_client() {
    run(async {
        let trusted = TestCa::new("trusted CA");
        let impostor = TestCa::new("impostor CA");
        let saw = attempt(&trusted, &impostor, "db.example.test").await;
        assert!(!saw.handshake_completed, "{saw:?}");
        assert!(!saw.startup_message_received);
    });
}

#[test]
fn a_certificate_for_another_host_name_is_refused_by_the_client() {
    run(async {
        let ca = TestCa::new("DungeonFlux test CA");
        let saw = attempt(&ca, &ca, "other.example.test").await;
        assert!(!saw.handshake_completed, "{saw:?}");
        assert!(!saw.startup_message_received);
    });
}
