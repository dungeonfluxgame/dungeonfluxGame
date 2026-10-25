//! One bounded Stripe session request, with no automatic redirect, retry or proxy.

use std::time::Duration;

use serde::Deserialize;

use super::settings::Settings;

pub(super) struct Provider {
    client: reqwest::Client,
    endpoint: reqwest::Url,
    pub(super) settings: Settings,
    origin: String,
}

#[derive(Debug)]
pub(super) enum Error {
    Configuration,
    Transport(reqwest::Error),
    Rejected,
    Response,
    Database(tokio_postgres::Error),
    Finance(df_portal::app_api::ports::PortError),
    Mismatch,
}

impl Error {
    pub(super) fn audit(&self) {
        let class = match self {
            Self::Configuration => "configuration",
            Self::Transport(cause) => {
                if cause.is_timeout() {
                    "timeout"
                } else {
                    "transport"
                }
            }
            Self::Rejected => "processor_refused",
            Self::Response => "processor_response",
            Self::Database(cause) => cause.code().map_or("database", |code| code.code()),
            Self::Finance(cause) => match cause {
                df_portal::app_api::ports::PortError::Forbidden => "account_refused",
                _ => "finance",
            },
            Self::Mismatch => "receipt_mismatch",
        };
        df_observe::record_audit_line(
            "hosted.stripe_failed",
            format!("SPEND stripe_failed error={class}"),
        );
    }
}

#[derive(Deserialize)]
pub(super) struct Session {
    pub(super) id: String,
    pub(super) url: String,
    pub(super) livemode: bool,
}

impl Provider {
    pub(super) fn new(settings: Settings, origin: &str) -> Result<Self, Error> {
        #[cfg(test)]
        if let Some(endpoint) = settings.endpoint.clone() {
            return Ok(Self::loopback(settings, origin, &endpoint));
        }
        let endpoint = reqwest::Url::parse("https://api.stripe.com/v1/checkout/sessions")
            .map_err(|_| Error::Configuration)?;
        Self::at_endpoint(settings, origin, endpoint)
    }

    fn at_endpoint(
        settings: Settings,
        origin: &str,
        endpoint: reqwest::Url,
    ) -> Result<Self, Error> {
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
        })
    }

    #[cfg(test)]
    pub(super) fn loopback(settings: Settings, origin: &str, endpoint: &str) -> Self {
        let endpoint = reqwest::Url::parse(endpoint).unwrap();
        assert_eq!(endpoint.scheme(), "http");
        assert!(
            endpoint
                .host_str()
                .unwrap()
                .parse::<std::net::IpAddr>()
                .unwrap()
                .is_loopback()
        );
        Self::at_endpoint(settings, origin, endpoint).unwrap()
    }

    pub(super) async fn create(&self, operation: &str, cents: u32) -> Result<Session, Error> {
        let amount = cents.to_string();
        let success = format!("{}/app/funds?checkout={operation}", self.origin);
        let cancel = format!("{}/app/credit", self.origin);
        let fields = [
            ("mode", "payment"),
            ("payment_method_types[0]", "card"),
            ("line_items[0][price_data][currency]", "usd"),
            ("line_items[0][price_data][unit_amount]", amount.as_str()),
            (
                "line_items[0][price_data][product_data][name]",
                "DungeonFlux credit",
            ),
            ("line_items[0][quantity]", "1"),
            ("metadata[operation_id]", operation),
            ("success_url", success.as_str()),
            ("cancel_url", cancel.as_str()),
        ];
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .basic_auth(&self.settings.secret_key, None::<&str>)
            .header("Idempotency-Key", operation)
            .form(&fields)
            .send()
            .await
            .map_err(Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Rejected);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            if bytes.len() + chunk.len() > 65_536 {
                return Err(Error::Response);
            }
            bytes.extend_from_slice(&chunk);
        }
        let session: Session = serde_json::from_slice(&bytes).map_err(|_| Error::Response)?;
        let url = reqwest::Url::parse(&session.url).map_err(|_| Error::Response)?;
        if url.scheme() != "https"
            || url.host_str() != Some("checkout.stripe.com")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || session.id.is_empty()
            || session.id.len() > 255
            || session.livemode == self.settings.is_test_mode
        {
            return Err(Error::Response);
        }
        Ok(session)
    }
}
