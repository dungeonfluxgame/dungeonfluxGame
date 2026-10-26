//! Bounded one-use authorization codes for the local preview. Never contacts Google.

use df_customer::{
    EmailAddress, OAuthAuthorize, OAuthCall, OAuthExchange, OAuthFailure, OAuthIdentity,
    OAuthProvider, OAuthSecret, pkce_challenge,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::Instant;

const MAXIMUM_CODES: usize = 128;

struct Code {
    challenge: [u8; 32],
    nonce: String,
    require_fresh: bool,
    issued_at: Instant,
    subject: String,
    email: EmailAddress,
}

struct Identity {
    subject: String,
    email: EmailAddress,
    failure: Option<OAuthFailure>,
    cancel: bool,
}

#[derive(Clone)]
pub struct FakeGoogle {
    callback: String,
    codes: Arc<Mutex<HashMap<String, Code>>>,
    identity: Arc<Mutex<Identity>>,
}

impl FakeGoogle {
    pub fn new(public_origin: &str) -> Self {
        Self {
            callback: format!("{public_origin}/app/api/oauth/google/callback"),
            codes: Arc::default(),
            identity: Arc::new(Mutex::new(Identity {
                subject: "fake-google-subject".to_owned(),
                email: EmailAddress::from_stored(
                    "google@example.test".to_owned(),
                    "google@example.test".to_owned(),
                ),
                failure: None,
                cancel: false,
            })),
        }
    }

    pub fn set_identity(&self, subject: &str, email: &str) -> Result<(), OAuthFailure> {
        if subject.is_empty() || subject.len() > 256 || subject.chars().any(char::is_control) {
            return Err(OAuthFailure::Malformed);
        }
        let email = EmailAddress::parse(email).map_err(|_| OAuthFailure::Malformed)?;
        let mut identity = self
            .identity
            .lock()
            .map_err(|_| OAuthFailure::Unavailable)?;
        identity.subject = subject.to_owned();
        identity.email = email;
        identity.failure = None;
        identity.cancel = false;
        Ok(())
    }

    pub fn cancel_next_authorization(&self) -> Result<(), OAuthFailure> {
        self.identity
            .lock()
            .map_err(|_| OAuthFailure::Unavailable)?
            .cancel = true;
        Ok(())
    }

    pub fn fail_next_exchange(&self, failure: OAuthFailure) -> Result<(), OAuthFailure> {
        self.identity
            .lock()
            .map_err(|_| OAuthFailure::Unavailable)?
            .failure = Some(failure);
        Ok(())
    }
}

impl OAuthProvider for FakeGoogle {
    fn redirect_uri(&self) -> &str {
        &self.callback
    }

    fn authorize(&self, request: OAuthAuthorize<'_>) -> Result<OAuthSecret, OAuthFailure> {
        let (canceled, subject, email) = {
            let mut identity = self
                .identity
                .lock()
                .map_err(|_| OAuthFailure::Unavailable)?;
            (
                std::mem::take(&mut identity.cancel),
                identity.subject.clone(),
                identity.email.clone(),
            )
        };
        if canceled {
            let mut url =
                reqwest::Url::parse(&self.callback).map_err(|_| OAuthFailure::Malformed)?;
            url.query_pairs_mut().extend_pairs([
                ("state", request.state.expose()),
                ("error", "access_denied"),
            ]);
            return Ok(OAuthSecret::new(url.into()));
        }
        let mut codes = self.codes.lock().map_err(|_| OAuthFailure::Unavailable)?;
        codes.retain(|_, code| code.issued_at.elapsed() < Duration::from_secs(600));
        if codes.len() >= MAXIMUM_CODES {
            return Err(OAuthFailure::Unavailable);
        }
        let code = request.state.expose().to_owned();
        codes.insert(
            code.clone(),
            Code {
                challenge: request.challenge,
                nonce: request.nonce.expose().to_owned(),
                require_fresh: request.require_fresh,
                issued_at: Instant::now(),
                subject,
                email,
            },
        );
        let mut url = reqwest::Url::parse(&self.callback).map_err(|_| OAuthFailure::Malformed)?;
        url.query_pairs_mut()
            .extend_pairs([("code", code.as_str()), ("state", request.state.expose())]);
        Ok(OAuthSecret::new(url.into()))
    }

    fn exchange<'a>(&'a self, request: OAuthExchange<'a>) -> OAuthCall<'a> {
        Box::pin(async move {
            let code = self
                .codes
                .lock()
                .map_err(|_| OAuthFailure::Unavailable)?
                .remove(request.code.expose())
                .ok_or(OAuthFailure::Rejected)?;
            if code.challenge != pkce_challenge(request.verifier)
                || code.nonce != request.nonce.expose()
                || code.require_fresh != request.require_fresh
                || code.issued_at.elapsed() >= Duration::from_secs(600)
            {
                return Err(OAuthFailure::Rejected);
            }
            let mut identity = self
                .identity
                .lock()
                .map_err(|_| OAuthFailure::Unavailable)?;
            if let Some(failure) = identity.failure.take() {
                return Err(failure);
            }
            Ok(OAuthIdentity {
                subject: code.subject,
                email: code.email,
                authenticated_at_s: request.require_fresh.then_some(request.now_s),
            })
        })
    }
}
