//! Mail delivery selects one connector at startup; a failed provider never falls back to outbox.

use std::{sync::Arc, time::Duration};

use df_customer::{DevOutbox, EmailError, EmailSender, Message};

#[derive(Clone)]
pub(crate) enum Sender {
    Outbox(Arc<DevOutbox>),
    Resend {
        client: reqwest::Client,
        api_key: Arc<str>,
    },
}

impl Sender {
    pub(crate) fn resend(api_key: String) -> Result<Self, reqwest::Error> {
        Ok(Self::Resend {
            client: reqwest::Client::builder()
                .tls_backend_rustls()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .no_proxy()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(9))
                .build()?,
            api_key: Arc::from(api_key),
        })
    }
}

impl EmailSender for Sender {
    async fn send(&self, message: Message) -> Result<(), EmailError> {
        match self {
            Self::Outbox(outbox) => outbox.send(message).await,
            Self::Resend { client, api_key } => {
                // TODO(B): replace with df-customer::resend when its sender is exported.
                let response = client
                    .post("https://api.resend.com/emails")
                    .bearer_auth(api_key.as_ref())
                    .json(&serde_json::json!({
                        "from": message.from,
                        "to": [message.to],
                        "reply_to": message.reply_to,
                        "subject": message.subject,
                        "text": message.body,
                    }))
                    .send()
                    .await
                    .map_err(|error| {
                        if error.is_connect() {
                            EmailError::Failed
                        } else {
                            EmailError::Unknown
                        }
                    })?;
                if response.status().is_success() {
                    Ok(())
                } else {
                    Err(EmailError::Unknown)
                }
            }
        }
    }
}
