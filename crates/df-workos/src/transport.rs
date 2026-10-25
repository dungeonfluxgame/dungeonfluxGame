//! The HTTP seam under the WorkOS adapter. The adapter builds requests as values; a transport
//! sends them. The real transport is `ReqwestTransport` (HTTPS only, no redirects, no proxies,
//! no automatic retries, bounded response); `FakeWorkOsServer` is the in-memory transport the
//! tests use. Nothing here is called outside a deployment that configured a WorkOS API key.

use std::{fmt, future::Future, time::Duration};

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

/// A request to the WorkOS API. The bearer credential and the body (which carries passwords)
/// never appear in `Debug` output.
#[derive(Clone)]
pub struct HttpRequest {
    pub method: Method,
    /// Path and query, starting with `/`.
    pub path: String,
    /// The API key sent as `Authorization: Bearer`, when the endpoint takes one.
    pub bearer: Option<String>,
    pub body: Option<Value>,
}

impl fmt::Debug for HttpRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HttpRequest")
            .field("method", &self.method)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    /// The request was not sent (could not connect): nothing happened at WorkOS.
    NotSent,
    /// The request may have been received: a timeout or a lost answer.
    Unknown,
}

pub trait HttpTransport: Send + Sync + 'static {
    fn send(
        &self,
        request: HttpRequest,
    ) -> impl Future<Output = Result<HttpResponse, TransportError>> + Send;
}

/// Largest response body kept.
pub const MAXIMUM_RESPONSE_BYTES: usize = 256 * 1024;

/// Where the transport may connect. The API key travels in every request, so the host is not a
/// prefix test: production is exactly `https://api.workos.com`, and the only other endpoint is
/// an explicit loopback `http://127.0.0.1:<port>` for a test server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkOsEndpoint {
    Production,
    Loopback { port: u16 },
}

/// The one production API host.
pub const PRODUCTION_HOST: &str = "api.workos.com";

impl WorkOsEndpoint {
    /// Parses a configured base URL by parts, not by prefix. `https://api.workos.com` (with one
    /// optional trailing slash) is production; `http://127.0.0.1:<port>` is loopback and is
    /// accepted only when `allow_loopback` is set (tests). Everything else, including
    /// `https://api.workos.com.evil.test`, `https://api.workos.com@evil.test`, another port, a
    /// path, a query or a fragment, is refused.
    pub fn parse(text: &str, allow_loopback: bool) -> Option<Self> {
        let text = text.strip_suffix('/').unwrap_or(text);
        if let Some(host) = text.strip_prefix("https://") {
            return (host == PRODUCTION_HOST).then_some(Self::Production);
        }
        let authority = text.strip_prefix("http://")?;
        let port = authority.strip_prefix("127.0.0.1:")?;
        let valid =
            !port.is_empty() && port.len() <= 5 && port.bytes().all(|byte| byte.is_ascii_digit());
        let port: u16 = port.parse().ok().filter(|port| valid && *port != 0)?;
        allow_loopback.then_some(Self::Loopback { port })
    }

    fn base(self) -> String {
        match self {
            Self::Production => format!("https://{PRODUCTION_HOST}"),
            Self::Loopback { port } => format!("http://127.0.0.1:{port}"),
        }
    }
}

/// The production transport.
pub struct ReqwestTransport {
    client: reqwest::Client,
    base: String,
}

impl ReqwestTransport {
    /// `deadline` bounds a whole call.
    pub fn new(endpoint: WorkOsEndpoint, deadline: Duration) -> Result<Self, TransportError> {
        let loopback = matches!(endpoint, WorkOsEndpoint::Loopback { .. });
        let client = reqwest::Client::builder()
            .tls_backend_rustls()
            .https_only(!loopback)
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .referer(false)
            .no_proxy()
            .connect_timeout(deadline.min(Duration::from_secs(5)))
            .timeout(deadline)
            .build()
            .map_err(|_| TransportError::NotSent)?;
        Ok(Self {
            client,
            base: endpoint.base(),
        })
    }
}

impl HttpTransport for ReqwestTransport {
    async fn send(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        let method = match request.method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Put => reqwest::Method::PUT,
            Method::Delete => reqwest::Method::DELETE,
        };
        let mut builder = self
            .client
            .request(method, format!("{}{}", self.base, request.path));
        if let Some(key) = &request.bearer {
            builder = builder.bearer_auth(key);
        }
        if let Some(body) = &request.body {
            builder = builder
                .header("Content-Type", "application/json")
                .body(body.to_string());
        }
        let mut response = builder.send().await.map_err(|error| {
            if error.is_connect() {
                TransportError::NotSent
            } else {
                TransportError::Unknown
            }
        })?;
        let status = response.status().as_u16();
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| TransportError::Unknown)?
        {
            if body.len() + chunk.len() > MAXIMUM_RESPONSE_BYTES {
                return Err(TransportError::Unknown);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(HttpResponse { status, body })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_never_prints_its_key_or_body() {
        let request = HttpRequest {
            method: Method::Post,
            path: "/user_management/users".to_owned(),
            bearer: Some("sk_test_secret".to_owned()),
            body: Some(serde_json::json!({"password": "hunter2 hunter2"})),
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("sk_test_secret"));
        assert!(!shown.contains("hunter2"));
    }

    #[test]
    fn only_the_exact_api_host_is_accepted_and_loopback_only_when_asked() {
        assert_eq!(
            WorkOsEndpoint::parse("https://api.workos.com", false),
            Some(WorkOsEndpoint::Production)
        );
        assert_eq!(
            WorkOsEndpoint::parse("https://api.workos.com/", false),
            Some(WorkOsEndpoint::Production)
        );
        for bad in [
            "https://api.workos.com.evil.test",
            "https://api.workos.com@evil.test",
            "https://evil.test/api.workos.com",
            "https://api.workos.com:444",
            "https://api.workos.com/v1",
            "https://api.workos.com?x=1",
            "https://API.workos.com",
            "https://xapi.workos.com",
            "http://api.workos.com",
            "http://127.0.0.1.evil.test:80",
            "http://127.0.0.1.evil.test",
            "http://127.0.0.1:80@evil.test",
            "http://127.0.0.1",
            "http://127.0.0.1:0",
            "http://127.0.0.1:99999",
            "http://localhost:80",
            "ftp://api.workos.com",
            "",
        ] {
            assert_eq!(WorkOsEndpoint::parse(bad, true), None, "{bad}");
        }
        assert_eq!(
            WorkOsEndpoint::parse("http://127.0.0.1:8123", true),
            Some(WorkOsEndpoint::Loopback { port: 8123 })
        );
        assert_eq!(
            WorkOsEndpoint::parse("http://127.0.0.1:8123", false),
            None,
            "production configuration never accepts loopback"
        );
    }

    #[test]
    fn a_transport_builds_for_either_endpoint() {
        assert!(ReqwestTransport::new(WorkOsEndpoint::Production, Duration::from_secs(1)).is_ok());
        assert!(
            ReqwestTransport::new(WorkOsEndpoint::Loopback { port: 9 }, Duration::from_secs(1))
                .is_ok()
        );
    }
}
