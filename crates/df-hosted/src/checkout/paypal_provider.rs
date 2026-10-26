//! Bounded authenticated Orders v2 calls; provider URLs and tokens never reach the client.

use std::time::{Duration, Instant};

use axum::http::HeaderMap;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{paypal_settings::Settings, provider::Error};

/// Maximum decoded provider response, including token and signature responses.
const MAXIMUM_RESPONSE_BYTES: usize = 65_536;

pub(super) struct Provider {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    pub(super) settings: Settings,
    origin: String,
    token: tokio::sync::Mutex<Option<Token>>,
}

struct Token {
    value: String,
    refresh_at: Instant,
}

#[derive(Deserialize)]
struct AccessToken {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Deserialize)]
pub(super) struct Order {
    pub(super) id: String,
    pub(super) status: String,
    #[serde(default)]
    pub(super) purchase_units: Vec<PurchaseUnit>,
    #[serde(default)]
    links: Vec<Link>,
}

#[derive(Deserialize)]
pub(super) struct PurchaseUnit {
    #[serde(default)]
    pub(super) custom_id: String,
    pub(super) amount: Option<Amount>,
    pub(super) payments: Option<Payments>,
}

#[derive(Deserialize)]
pub(super) struct Payments {
    pub(super) captures: Vec<Capture>,
}

#[derive(Deserialize, Clone)]
pub(super) struct Capture {
    pub(super) id: String,
    pub(super) status: String,
    pub(super) amount: Amount,
    pub(super) custom_id: Option<String>,
    pub(super) supplementary_data: Option<SupplementaryData>,
}

#[derive(Deserialize, Clone, PartialEq, Eq)]
pub(super) struct Amount {
    pub(super) currency_code: String,
    pub(super) value: String,
}

#[derive(Deserialize, Clone)]
pub(super) struct SupplementaryData {
    pub(super) related_ids: RelatedIds,
}

#[derive(Deserialize, Clone)]
pub(super) struct RelatedIds {
    pub(super) order_id: String,
}

#[derive(Deserialize)]
struct Link {
    rel: String,
    href: String,
}

pub(super) fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub(super) fn dollars(cents: u32) -> String {
    format!("{}.{:02}", cents / 100, cents % 100)
}

impl Provider {
    pub(super) fn new(settings: Settings, origin: &str) -> Result<Self, Error> {
        let base = if settings.is_test_mode {
            "https://api-m.sandbox.paypal.com/"
        } else {
            "https://api-m.paypal.com/"
        };
        #[cfg(test)]
        let base = settings.endpoint.as_deref().unwrap_or(base);
        let endpoint = reqwest::Url::parse(base).map_err(|_| Error::Configuration)?;
        let origin_url = reqwest::Url::parse(origin).map_err(|_| Error::Configuration)?;
        if origin_url.host_str().is_none()
            || origin_url.query().is_some()
            || origin_url.fragment().is_some()
            || origin_url.path() != "/"
            || !origin_url.username().is_empty()
            || origin_url.password().is_some()
            || !matches!(origin_url.scheme(), "https" | "http")
        {
            return Err(Error::Configuration);
        }
        #[cfg(test)]
        if endpoint.scheme() != "https" {
            assert_eq!(endpoint.scheme(), "http");
            assert!(
                endpoint
                    .host_str()
                    .unwrap()
                    .parse::<std::net::IpAddr>()
                    .unwrap()
                    .is_loopback()
            );
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(9))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .build()
            .map_err(Error::Transport)?;
        Ok(Self {
            client,
            endpoint,
            settings,
            origin: origin.trim_end_matches('/').to_owned(),
            token: tokio::sync::Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(super) async fn expire_token(&self) {
        if let Some(token) = self.token.lock().await.as_mut() {
            token.refresh_at = Instant::now();
        }
    }

    async fn token(&self) -> Result<String, Error> {
        // Serializes only token refresh; cancellation releases the lock and leaves the old cache.
        // No receipt/DB lock is held by callers during this bounded network call.
        let mut cached = self.token.lock().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| Instant::now() < token.refresh_at)
        {
            return Ok(token.value.clone());
        }
        let started = Instant::now();
        let response = self
            .client
            .post(self.url("v1/oauth2/token")?)
            .basic_auth(&self.settings.client_id, Some(&self.settings.client_secret))
            .form(&[("grant_type", "client_credentials")])
            .send()
            .await
            .map_err(Error::Transport)?;
        let token: AccessToken = decode(response).await?;
        if token.access_token.is_empty()
            || token.token_type != "Bearer"
            || !(61..=86_400).contains(&token.expires_in)
        {
            return Err(Error::Response);
        }
        *cached = Some(Token {
            value: token.access_token.clone(),
            refresh_at: started + Duration::from_secs(token.expires_in - 60),
        });
        Ok(token.access_token)
    }

    fn url(&self, path: &str) -> Result<reqwest::Url, Error> {
        self.endpoint.join(path).map_err(|_| Error::Configuration)
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        request_id: Option<&str>,
        body: Option<&Value>,
    ) -> Result<T, Error> {
        let token = self.token().await?;
        let mut request = self
            .client
            .request(method, self.url(path)?)
            .bearer_auth(token)
            .header("Prefer", "return=representation");
        if let Some(request_id) = request_id {
            request = request.header("PayPal-Request-Id", request_id);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        decode(request.send().await.map_err(Error::Transport)?).await
    }

    pub(super) async fn create(
        &self,
        operation: &str,
        cents: u32,
    ) -> Result<(String, String), Error> {
        let body = json!({
            "intent":"CAPTURE",
            "purchase_units":[{"custom_id":operation,"amount":{"currency_code":"USD","value":dollars(cents)}}],
            "application_context":{
                "return_url":format!("{}/app/api/payments/paypal/return?checkout={operation}",self.origin),
                "cancel_url":format!("{}/app/api/payments/paypal/cancel?checkout={operation}",self.origin),
                "user_action":"PAY_NOW"
            }
        });
        let order: Order = self
            .request(
                reqwest::Method::POST,
                "v2/checkout/orders",
                Some(operation),
                Some(&body),
            )
            .await?;
        if !valid_id(&order.id)
            || !matches!(order.status.as_str(), "CREATED" | "PAYER_ACTION_REQUIRED")
        {
            return Err(Error::Response);
        }
        let link = order
            .links
            .into_iter()
            .find(|link| matches!(link.rel.as_str(), "payer-action" | "approve"))
            .ok_or(Error::Response)?;
        let url = reqwest::Url::parse(&link.href).map_err(|_| Error::Response)?;
        let host = if self.settings.is_test_mode {
            "www.sandbox.paypal.com"
        } else {
            "www.paypal.com"
        };
        if url.scheme() != "https"
            || url.host_str() != Some(host)
            || url.port().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Response);
        }
        Ok((order.id, link.href))
    }

    pub(super) async fn capture(&self, order: &str, operation: &str) -> Result<Order, Error> {
        if !valid_id(order) {
            return Err(Error::Mismatch);
        }
        self.request(
            reqwest::Method::POST,
            &format!("v2/checkout/orders/{order}/capture"),
            Some(&format!("c-{operation}")),
            Some(&json!({})),
        )
        .await
    }

    pub(super) async fn inspect(&self, order: &str) -> Result<Order, Error> {
        if !valid_id(order) {
            return Err(Error::Mismatch);
        }
        self.request(
            reqwest::Method::GET,
            &format!("v2/checkout/orders/{order}"),
            None,
            None,
        )
        .await
    }

    pub(super) async fn verify(&self, headers: &HeaderMap, event: &Value) -> Result<(), Error> {
        let webhook_id = self.settings.webhook_id.as_ref().ok_or(Error::Mismatch)?;
        let header = |name| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .filter(|value| !value.is_empty() && value.len() <= 4096)
                .ok_or(Error::Mismatch)
        };
        let body = json!({
            "auth_algo":header("paypal-auth-algo")?,
            "cert_url":header("paypal-cert-url")?,
            "transmission_id":header("paypal-transmission-id")?,
            "transmission_sig":header("paypal-transmission-sig")?,
            "transmission_time":header("paypal-transmission-time")?,
            "webhook_id":webhook_id,
            "webhook_event":event
        });
        #[derive(Deserialize)]
        struct Verification {
            verification_status: String,
        }
        let verified: Verification = self
            .request(
                reqwest::Method::POST,
                "v1/notifications/verify-webhook-signature",
                None,
                Some(&body),
            )
            .await?;
        if verified.verification_status != "SUCCESS" {
            return Err(Error::Mismatch);
        }
        Ok(())
    }
}

async fn decode<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T, Error> {
    if !response.status().is_success() {
        return Err(Error::Rejected);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
        if bytes.len() + chunk.len() > MAXIMUM_RESPONSE_BYTES {
            return Err(Error::Response);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::Response)
}
