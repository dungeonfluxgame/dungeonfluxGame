//! Server-owned attempts, Google identity resolution and first-party sessions.

use crate::{
    AccountStatus, AccountStore, Authenticated, Client, CustomerService, EmailSender, EventKind,
    IssuedSession, OAuthAuthorize, OAuthExchange, OAuthFailure, OAuthProvider, OAuthSecret,
    SecretToken, ServiceError,
    oauth_store::{
        GoogleResolution, GoogleResolved, GoogleStore, OAuthAttempt, OAuthIntent, OAuthRefusal,
    },
    pkce_challenge,
    service::{PROVIDER_DEADLINE, audit, read, write},
    throttle::{Subject, ThrottleScope},
    tokens::{new_account_id, tagged_digest},
};
use std::sync::Arc;

const ATTEMPT_LIFETIME_MS: u64 = 10 * 60 * 1000;

pub(crate) struct OAuthState {
    pub(crate) provider: Option<Arc<dyn OAuthProvider>>,
    pub(crate) store: Arc<dyn GoogleStore>,
}

pub struct OAuthStart {
    pub authorization_url: OAuthSecret,
    pub cookie: SecretToken,
}

pub struct OAuthStartForm<'a> {
    pub intent: OAuthIntent,
    pub adult_attested: bool,
    pub terms_version: &'a str,
    pub current_password: &'a str,
}

pub enum OAuthOutcome {
    SignedUp(IssuedSession),
    SignedIn(IssuedSession),
    Linked,
    Refused(OAuthRefusal),
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    /// Composition supplies the real Google adapter and the same account database's OAuth store.
    pub fn with_google(mut self, provider: impl OAuthProvider, store: impl GoogleStore) -> Self {
        self.google = Some(Arc::new(OAuthState {
            provider: Some(Arc::new(provider)),
            store: Arc::new(store),
        }));
        self
    }

    /// Provenance remains enforced when Google credentials are temporarily not configured.
    pub fn with_google_store(mut self, store: impl GoogleStore) -> Self {
        self.google = Some(Arc::new(OAuthState {
            provider: None,
            store: Arc::new(store),
        }));
        self
    }

    pub fn google_enabled(&self) -> bool {
        self.google
            .as_ref()
            .is_some_and(|google| google.provider.is_some())
    }

    pub async fn google_is_linked(&self, caller: &Authenticated) -> Result<bool, ServiceError> {
        match &self.google {
            Some(google) => read(google.store.is_linked(caller.account.id)).await,
            None => Ok(false),
        }
    }

    pub async fn has_password_method(&self, caller: &Authenticated) -> Result<bool, ServiceError> {
        Ok(caller.account.password_hash.is_some()
            || !read(self.shared.store.identities_of(caller.account.id))
                .await?
                .is_empty())
    }

    pub async fn has_fresh_google_proof(
        &self,
        caller: &Authenticated,
    ) -> Result<bool, ServiceError> {
        Ok(self
            .google_session_proof(caller)
            .await?
            .flatten()
            .is_some_and(|expires_at_ms| self.now_ms() < expires_at_ms))
    }

    pub async fn start_google(
        &self,
        form: OAuthStartForm<'_>,
        caller: Option<&Authenticated>,
        client: Client,
    ) -> Result<Result<OAuthStart, OAuthRefusal>, ServiceError> {
        let intent = form.intent;
        let outcome = self.start_google_attempt(form, caller, client).await;
        let refusal = match &outcome {
            Ok(Err(reason)) => Some(reason.label()),
            Err(error) => Some(error.label()),
            _ => None,
        };
        if let Some(reason) = refusal {
            audit(
                "account.oauth",
                format!(
                    "ACCOUNT oauth_start_refused intent={} outcome={reason}",
                    intent.label()
                ),
            );
        }
        outcome
    }

    async fn start_google_attempt(
        &self,
        form: OAuthStartForm<'_>,
        caller: Option<&Authenticated>,
        client: Client,
    ) -> Result<Result<OAuthStart, OAuthRefusal>, ServiceError> {
        let Some(google) = &self.google else {
            return Ok(Err(OAuthRefusal::Disabled));
        };
        let Some(provider) = &google.provider else {
            return Ok(Err(OAuthRefusal::Disabled));
        };
        if !self.admit(ThrottleScope::LoginPerIp, Subject::Ip(client.ip)) {
            return Ok(Err(OAuthRefusal::Throttled));
        }
        if form.intent == OAuthIntent::SignUp
            && !self.admit(ThrottleScope::SignupPerIp, Subject::Ip(client.ip))
        {
            return Ok(Err(OAuthRefusal::Throttled));
        }
        if form.intent == OAuthIntent::SignUp
            && (!form.adult_attested || form.terms_version != self.current_terms_version())
        {
            return Ok(Err(OAuthRefusal::ConsentRequired));
        }
        if form.intent.requires_fresh() {
            let Some(caller) = caller else {
                return Ok(Err(OAuthRefusal::ReauthenticationRequired));
            };
            if !self.admit(
                ThrottleScope::PasswordChangePerAccount,
                Subject::Account(caller.account.id),
            ) {
                return Ok(Err(OAuthRefusal::Throttled));
            }
            if !caller.account.is_verified || caller.account.status != AccountStatus::Active {
                return Ok(Err(OAuthRefusal::AccountUnavailable));
            }
            if form.intent == OAuthIntent::Link
                && !self.password_matches(caller, form.current_password).await?
            {
                return Ok(Err(OAuthRefusal::PasswordSignInRequired));
            }
            if form.intent == OAuthIntent::Reauthenticate
                && !read(google.store.is_linked(caller.account.id)).await?
            {
                return Ok(Err(OAuthRefusal::Conflict));
            }
        }
        let state = SecretToken::generate()?;
        let cookie = SecretToken::generate()?;
        let nonce = OAuthSecret::new(SecretToken::generate()?.text());
        let verifier = OAuthSecret::new(SecretToken::generate()?.text());
        let url = provider
            .authorize(OAuthAuthorize {
                state: &OAuthSecret::new(state.text()),
                nonce: &nonce,
                challenge: pkce_challenge(&verifier),
                require_fresh: form.intent.requires_fresh(),
            })
            .map_err(|_| ServiceError::Unavailable)?;
        let now_ms = self.now_ms();
        let attempt = OAuthAttempt {
            redirect_uri: provider.redirect_uri().to_owned(),
            state_digest: state.digest(),
            cookie_digest: cookie.digest(),
            client_digest: tagged_digest(
                "df-google-client",
                client
                    .ip
                    .map(|ip| ip.to_string())
                    .unwrap_or_default()
                    .as_bytes(),
            ),
            nonce,
            verifier,
            intent: form.intent,
            account: caller
                .filter(|_| form.intent.requires_fresh())
                .map(|caller| caller.account.id),
            session: caller
                .filter(|_| form.intent.requires_fresh())
                .map(|caller| caller.session),
            credential_generation: caller
                .filter(|_| form.intent.requires_fresh())
                .map(|caller| caller.account.credential_generation),
            terms_version: (form.intent == OAuthIntent::SignUp)
                .then(|| self.current_terms_version().to_owned()),
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ATTEMPT_LIFETIME_MS),
        };
        if !write(google.store.save_attempt(attempt)).await? {
            return Ok(Err(OAuthRefusal::Throttled));
        }
        audit(
            "account.oauth",
            format!(
                "ACCOUNT oauth_started attempt={} intent={}",
                crate::tokens::hex(state.digest().as_bytes()),
                form.intent.label()
            ),
        );
        Ok(Ok(OAuthStart {
            authorization_url: url,
            cookie,
        }))
    }

    pub async fn finish_google(
        &self,
        state: &str,
        cookie: &str,
        code: Option<&str>,
        canceled: bool,
        caller: Option<&Authenticated>,
        client: Client,
    ) -> Result<OAuthOutcome, ServiceError> {
        let outcome = self
            .finish_google_attempt(state, cookie, code, canceled, caller, client)
            .await;
        let label = match &outcome {
            Ok(OAuthOutcome::SignedUp(_)) => "signed_up",
            Ok(OAuthOutcome::SignedIn(_)) => "signed_in",
            Ok(OAuthOutcome::Linked) => "linked",
            Ok(OAuthOutcome::Refused(reason)) => reason.label(),
            Err(error) => error.label(),
        };
        let attempt = SecretToken::parse(state)
            .map(|state| crate::tokens::hex(state.digest().as_bytes()))
            .unwrap_or_else(|| "none".to_owned());
        audit(
            "account.oauth",
            format!("ACCOUNT oauth_callback attempt={attempt} outcome={label}"),
        );
        outcome
    }

    async fn finish_google_attempt(
        &self,
        state: &str,
        cookie: &str,
        code: Option<&str>,
        canceled: bool,
        caller: Option<&Authenticated>,
        client: Client,
    ) -> Result<OAuthOutcome, ServiceError> {
        if !self.admit(ThrottleScope::LoginPerIp, Subject::Ip(client.ip)) {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Throttled));
        }
        let Some(google) = &self.google else {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Disabled));
        };
        let Some(provider) = &google.provider else {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Disabled));
        };
        let Some((state, cookie)) = SecretToken::parse(state).zip(SecretToken::parse(cookie))
        else {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Expired));
        };
        let Some(attempt) = write(google.store.claim_attempt(
            state.digest(),
            cookie.digest(),
            self.now_ms(),
        ))
        .await?
        else {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Expired));
        };
        if canceled {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Canceled));
        }
        if attempt.redirect_uri != provider.redirect_uri() {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Expired));
        }
        if attempt.intent.requires_fresh()
            && !caller.is_some_and(|caller| {
                Some(caller.account.id) == attempt.account
                    && Some(caller.session) == attempt.session
                    && Some(caller.account.credential_generation) == attempt.credential_generation
            })
        {
            return Ok(OAuthOutcome::Refused(
                OAuthRefusal::ReauthenticationRequired,
            ));
        }
        if attempt
            .terms_version
            .as_deref()
            .is_some_and(|version| version != self.current_terms_version())
        {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::ConsentRequired));
        }
        let Some(code) = code.filter(|code| !code.is_empty() && code.len() <= 4096) else {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Expired));
        };
        let code = OAuthSecret::new(code.to_owned());
        let exchange = provider.exchange(OAuthExchange {
            code: &code,
            verifier: &attempt.verifier,
            nonce: &attempt.nonce,
            now_s: self.now_ms() / 1000,
            require_fresh: attempt.intent.requires_fresh(),
        });
        let identity = match tokio::time::timeout(PROVIDER_DEADLINE, exchange).await {
            Ok(Ok(identity)) => identity,
            error => {
                let refusal = match error {
                    Ok(Err(OAuthFailure::EmailNotVerified)) => OAuthRefusal::EmailNotVerified,
                    Ok(Err(OAuthFailure::Rejected)) => OAuthRefusal::Expired,
                    Ok(Err(OAuthFailure::FreshAuthenticationRequired)) => {
                        OAuthRefusal::ReauthenticationRequired
                    }
                    _ => OAuthRefusal::Unavailable,
                };
                return Ok(OAuthOutcome::Refused(refusal));
            }
        };
        if self.now_ms() >= attempt.expires_at_ms {
            return Ok(OAuthOutcome::Refused(OAuthRefusal::Expired));
        }
        let proof = attempt
            .intent
            .requires_fresh()
            .then_some(identity.authenticated_at_s)
            .flatten();
        if attempt.intent.requires_fresh()
            && !proof.is_some_and(|at_s| {
                let now_s = self.now_ms() / 1000;
                at_s <= now_s && now_s.saturating_sub(at_s) <= 300
            })
        {
            return Ok(OAuthOutcome::Refused(
                OAuthRefusal::ReauthenticationRequired,
            ));
        }
        let subject = identity.subject.clone();
        let resolved = write(google.store.resolve(GoogleResolution {
            subject: identity.subject,
            email: identity.email,
            intent: attempt.intent,
            account: attempt.account,
            credential_generation: attempt.credential_generation,
            originating_session: attempt.session,
            idle_before_ms: crate::retention::idle_before_ms(
                self.now_ms(),
                self.shared.tunables.sessions.idle,
            ),
            new_account: new_account_id()?,
            terms_version: attempt.terms_version,
            accepted_at_ms: attempt.created_at_ms,
            now_ms: self.now_ms(),
        }))
        .await?;
        let (mut account, created) = match resolved {
            GoogleResolved::Created(account) => (account, true),
            GoogleResolved::Account(account) => (account, false),
            GoogleResolved::Refused(reason) => return Ok(OAuthOutcome::Refused(reason)),
        };
        if attempt.intent == OAuthIntent::Link {
            return Ok(OAuthOutcome::Linked);
        }
        if account.status == AccountStatus::Closing {
            if !self.reopen(account.id).await? {
                return Ok(OAuthOutcome::Refused(OAuthRefusal::AccountUnavailable));
            }
            account = read(self.shared.store.account(account.id))
                .await?
                .ok_or(ServiceError::InvalidRecord)?;
        }
        let (session, token) =
            self.new_session(account.id, account.credential_generation, client)?;
        let issued = write(google.store.create_google_session(crate::GoogleSession {
            session,
            subject,
            reauthentication_expires_at_ms: proof.map(|time| {
                time.saturating_mul(1000)
                    .saturating_add(crate::CLOSURE_REAUTHENTICATION_MS)
                    .min(
                        self.now_ms()
                            .saturating_add(crate::CLOSURE_REAUTHENTICATION_MS),
                    )
            }),
            originating_session: attempt.session,
            idle_before_ms: crate::retention::idle_before_ms(
                self.now_ms(),
                self.shared.tunables.sessions.idle,
            ),
            event: crate::AccountEvent {
                kind: EventKind::LoggedIn,
                at_ms: self.now_ms(),
                device: client.device,
            },
        }))
        .await?;
        if !issued {
            return Ok(OAuthOutcome::Refused(
                OAuthRefusal::ReauthenticationRequired,
            ));
        }
        let session = IssuedSession {
            token,
            max_age_s: self.shared.tunables.sessions.absolute.as_secs(),
        };
        Ok(if created {
            OAuthOutcome::SignedUp(session)
        } else {
            OAuthOutcome::SignedIn(session)
        })
    }

    pub async fn unlink_google(
        &self,
        caller: &Authenticated,
        password: &str,
    ) -> Result<Result<(), OAuthRefusal>, ServiceError> {
        let outcome = self.unlink_google_method(caller, password).await;
        let label = match &outcome {
            Ok(Ok(())) => "unlinked",
            Ok(Err(reason)) => reason.label(),
            Err(error) => error.label(),
        };
        audit(
            "account.oauth",
            format!(
                "ACCOUNT oauth_unlink account={} outcome={label}",
                crate::service::account_label(caller.account.id)
            ),
        );
        outcome
    }

    async fn unlink_google_method(
        &self,
        caller: &Authenticated,
        password: &str,
    ) -> Result<Result<(), OAuthRefusal>, ServiceError> {
        let Some(google) = &self.google else {
            return Ok(Err(OAuthRefusal::Disabled));
        };
        if !self.admit(
            ThrottleScope::PasswordChangePerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(Err(OAuthRefusal::Throttled));
        }
        let password_method = caller.account.password_hash.is_some()
            || !read(self.shared.store.identities_of(caller.account.id))
                .await?
                .is_empty();
        if !password_method {
            return Ok(Err(OAuthRefusal::LastSignInMethod));
        }
        if !self.password_matches(caller, password).await? {
            return Ok(Err(OAuthRefusal::PasswordSignInRequired));
        }
        let outcome = write(
            google
                .store
                .unlink(caller.account.id, caller.account.credential_generation),
        )
        .await?;
        Ok(outcome)
    }

    pub(crate) async fn google_session_proof(
        &self,
        caller: &Authenticated,
    ) -> Result<Option<Option<u64>>, ServiceError> {
        match &self.google {
            Some(google) => read(google.store.session_proof(caller.session)).await,
            None => Ok(None),
        }
    }
}
