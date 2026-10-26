//! Exact Google endpoints and bounded, non-retrying token exchange.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use df_customer::{
    OAuthAuthorize, OAuthCall, OAuthExchange, OAuthFailure, OAuthProvider, OAuthSecret,
};
use reqwest::{Client, Url, redirect::Policy};
use serde::Deserialize;

const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const MAXIMUM_RESPONSE_BYTES: usize = 64 * 1024;
const EXCHANGE_DEADLINE: Duration = Duration::from_secs(10);

pub struct Config {
    client_id: String,
    client_secret: OAuthSecret,
    redirect_uri: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    Invalid,
    Transport,
}

impl Config {
    /// The public origin is validated by the account host configuration, never a request header.
    pub fn new(
        client_id: String,
        client_secret: OAuthSecret,
        hosts: &df_customer::HostConfig,
    ) -> Result<Self, ConfigError> {
        if client_id.is_empty()
            || client_secret.expose().is_empty()
            || client_id.chars().any(char::is_control)
        {
            return Err(ConfigError::Invalid);
        }
        Ok(Self {
            client_id,
            client_secret,
            redirect_uri: format!("{}/app/api/oauth/google/callback", hosts.origin().as_str()),
        })
    }
}

pub struct Google {
    config: Config,
    client: Client,
    token_url: Url,
}

impl Google {
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(3))
            .read_timeout(Duration::from_secs(5))
            .timeout(EXCHANGE_DEADLINE)
            .build()
            .map_err(|_| ConfigError::Transport)?;
        Ok(Self {
            config,
            client,
            token_url: Url::parse(TOKEN_URL).map_err(|_| ConfigError::Invalid)?,
        })
    }

    #[cfg(any(test, feature = "test-fakes"))]
    pub fn loopback_fixture(config: Config, endpoint: &str) -> Result<Self, ConfigError> {
        let mut google = Self::new(config)?;
        let url = Url::parse(endpoint).map_err(|_| ConfigError::Invalid)?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(ConfigError::Invalid);
        }
        google.token_url = url;
        Ok(google)
    }

    async fn exchange_code(
        &self,
        mut request: OAuthExchange<'_>,
    ) -> Result<df_customer::OAuthIdentity, OAuthFailure> {
        let started = tokio::time::Instant::now();
        let form = [
            ("grant_type", "authorization_code"),
            ("code", request.code.expose()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.expose()),
            ("code_verifier", request.verifier.expose()),
        ];
        let mut response = self
            .client
            .post(self.token_url.clone())
            .form(&form)
            .send()
            .await
            .map_err(transport_failure)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(
                if response.status().is_server_error()
                    || response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                {
                    OAuthFailure::Unavailable
                } else {
                    OAuthFailure::Rejected
                },
            );
        }
        if response
            .content_length()
            .is_some_and(|bytes| bytes > MAXIMUM_RESPONSE_BYTES as u64)
        {
            return Err(OAuthFailure::Oversize);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(transport_failure)? {
            if bytes.len().saturating_add(chunk.len()) > MAXIMUM_RESPONSE_BYTES {
                return Err(OAuthFailure::Oversize);
            }
            bytes.extend_from_slice(&chunk);
        }
        #[derive(Deserialize)]
        struct Tokens {
            id_token: String,
        }
        let tokens: Tokens = serde_json::from_slice(&bytes).map_err(|_| OAuthFailure::Malformed)?;
        request.now_s = request.now_s.saturating_add(started.elapsed().as_secs());
        super::claims::validate(&tokens.id_token, &self.config.client_id, &request)
    }
}

fn transport_failure(error: reqwest::Error) -> OAuthFailure {
    if error.is_timeout() {
        OAuthFailure::Timeout
    } else {
        OAuthFailure::Unavailable
    }
}

impl OAuthProvider for Google {
    fn redirect_uri(&self) -> &str {
        &self.config.redirect_uri
    }

    fn authorize(&self, request: OAuthAuthorize<'_>) -> Result<OAuthSecret, OAuthFailure> {
        let mut url = Url::parse(AUTHORIZE_URL).map_err(|_| OAuthFailure::Malformed)?;
        let challenge = URL_SAFE_NO_PAD.encode(request.challenge);
        url.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", self.config.client_id.as_str()),
            ("redirect_uri", self.config.redirect_uri.as_str()),
            ("scope", "openid email profile"),
            ("state", request.state.expose()),
            ("nonce", request.nonce.expose()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        if request.require_fresh {
            url.query_pairs_mut().append_pair("max_age", "0");
        }
        Ok(OAuthSecret::new(url.into()))
    }

    fn exchange<'a>(&'a self, request: OAuthExchange<'a>) -> OAuthCall<'a> {
        Box::pin(async move {
            tokio::time::timeout(EXCHANGE_DEADLINE, self.exchange_code(request))
                .await
                .map_err(|_| OAuthFailure::Timeout)?
        })
    }
}
