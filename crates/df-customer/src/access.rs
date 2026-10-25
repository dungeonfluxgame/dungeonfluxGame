//! Signing up, verifying, logging in and out, and session lookup and revocation.
//!
//! Responses that could reveal whether an address has an account are identical for both cases
//! (plan 6.3): signup always hashes and always queues exactly one message; login runs a dummy
//! verification for unknown addresses; failure throttles are keyed on the address, not an account.

use std::time::Instant;

use df_auth::{
    password_policy::{PasswordRefusal, check_new_password},
    web_session_policy::{SessionCheck, StoredSession, check_session},
};
use df_types::{OperationId, WebSessionId};

use crate::{
    ports::{AccountStore, EmailSender},
    provider_flows::ProviderSignup,
    records::{
        AccountEvent, AccountRecord, AccountStatus, CreateOutcome, EmailAddress, EmailRefusal,
        EventKind, NewAccount, NewSession, NewToken, OperationClaim, OperationKey,
        OperationNamespace, ReceiptOutcome, RevokeReason, RevokeScope, TokenPurpose, VerifyOutcome,
    },
    service::{Client, CustomerService, MailJob, ServiceError, account_label, audit, read, write},
    throttle::{Subject, ThrottleScope},
    tokens::{
        CsrfSecret, SecretToken, new_account_id, new_record_id, new_session_id,
        operation_issued_at_ms, tagged_digest,
    },
};

/// A verification link's lifetime.
pub const VERIFICATION_TOKEN_LIFETIME_MS: u64 = 24 * 60 * 60 * 1000;
/// Verification links have no attempt budget beyond single use.
const VERIFICATION_ATTEMPT_LIMIT: u32 = 1;
/// An operation id older than this (or from the future by more than the skew) is expired.
pub const OPERATION_LIFETIME_MS: u64 = 90 * 24 * 60 * 60 * 1000;
const OPERATION_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignupRefusal {
    Email(EmailRefusal),
    Password(PasswordRefusal),
    /// The form's operation id is too old or reused with different fields.
    StaleForm,
    /// The identity provider's own password policy refused the password.
    PasswordRejected,
    AdultAttestationRequired,
    TermsVersionChanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SignupOutcome {
    /// The same page whether the address was new or already registered.
    CheckYourEmail,
    Refused(SignupRefusal),
    Throttled,
}

pub struct SignupForm<'a> {
    pub email: &'a str,
    pub password: &'a str,
    pub operation: OperationId,
    pub adult_attested: bool,
    pub terms_version: String,
}

/// A newly issued session. The token goes into the cookie and nowhere else.
pub struct IssuedSession {
    pub token: SecretToken,
    pub max_age_s: u64,
}

pub enum LoginOutcome {
    SignedIn(IssuedSession),
    /// Wrong address or password, or no such account: one message for all.
    Refused,
    /// The password may be right but the address is not verified yet, so no session is issued
    /// (identity provider accounts only; local passwords sign in unverified).
    VerificationRequired,
    Throttled,
}

/// The authenticated caller of one request.
#[derive(Clone)]
pub struct Authenticated {
    pub session: WebSessionId,
    /// When this session was issued: the last time the person proved who they are.
    pub signed_in_at_ms: u64,
    pub account: AccountRecord,
    pub csrf: CsrfSecret,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifyEmailOutcome {
    Verified,
    AlreadyVerified,
    Invalid,
    Throttled,
}

pub enum VerifyWithPasswordOutcome {
    Verified(IssuedSession),
    AlreadyVerified,
    /// Unknown, expired or used link: one answer for all.
    InvalidLink,
    Mismatch,
    Refused(PasswordRefusal),
    /// The identity provider's own password policy refused the password; the link is unspent.
    ProviderRejectedPassword,
    Throttled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResendOutcome {
    Sent,
    AlreadyVerified,
    Throttled,
}

/// The receipt key of a signup: the principal is a digest of the normalized address.
fn signup_key(email: &EmailAddress, operation: OperationId) -> OperationKey {
    OperationKey {
        namespace: OperationNamespace::Signup,
        principal: tagged_digest("df-signup-principal", email.normalized().as_bytes()),
        operation,
    }
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    /// Whether a form's operation id is still within its lifetime.
    pub(crate) fn operation_is_current(&self, operation: &OperationId) -> bool {
        let issued_at_ms = operation_issued_at_ms(operation);
        let now_ms = self.now_ms();
        issued_at_ms <= now_ms.saturating_add(OPERATION_CLOCK_SKEW_MS)
            && now_ms.saturating_sub(issued_at_ms) < OPERATION_LIFETIME_MS
    }

    /// A fresh operation id for a form being rendered.
    pub fn new_operation(&self) -> Result<OperationId, ServiceError> {
        Ok(crate::tokens::new_operation_id(self.now_ms())?)
    }

    pub async fn signup(
        &self,
        form: SignupForm<'_>,
        client: Client,
    ) -> Result<SignupOutcome, ServiceError> {
        let started = tokio::time::Instant::now();
        let outcome = self.signup_unpadded(form, client).await;
        if self.provider().is_some() {
            // A new address makes more provider calls than a registered one: no answer comes
            // back before the same floor, so the time does not tell them apart.
            tokio::time::sleep_until(started + self.shared.signup_floor).await;
        }
        outcome
    }

    async fn signup_unpadded(
        &self,
        form: SignupForm<'_>,
        client: Client,
    ) -> Result<SignupOutcome, ServiceError> {
        if !self.admit(ThrottleScope::SignupPerIp, Subject::Ip(client.ip)) {
            return Ok(SignupOutcome::Throttled);
        }
        let email = match EmailAddress::parse(form.email) {
            Ok(email) => email,
            Err(refusal) => return Ok(SignupOutcome::Refused(SignupRefusal::Email(refusal))),
        };
        if !self.admit(
            ThrottleScope::SignupPerEmail,
            Subject::Email(email.normalized()),
        ) {
            return Ok(SignupOutcome::Throttled);
        }
        if let Err(refusal) = check_new_password(form.password, email.display(), self.is_common()) {
            return Ok(SignupOutcome::Refused(SignupRefusal::Password(refusal)));
        }
        if !form.adult_attested {
            return Ok(SignupOutcome::Refused(
                SignupRefusal::AdultAttestationRequired,
            ));
        }
        if form.terms_version != self.shared.current_terms_version {
            return Ok(SignupOutcome::Refused(SignupRefusal::TermsVersionChanged));
        }
        if !self.operation_is_current(&form.operation) {
            return Ok(SignupOutcome::Refused(SignupRefusal::StaleForm));
        }
        let key = signup_key(&email, form.operation);
        let fingerprint = tagged_digest("df-signup-fingerprint", email.normalized().as_bytes());
        match write(
            self.shared
                .store
                .claim_operation(key, fingerprint, self.now_ms()),
        )
        .await?
        {
            OperationClaim::New => {}
            OperationClaim::Replayed(_) | OperationClaim::InProgress => {
                return Ok(SignupOutcome::CheckYourEmail);
            }
            OperationClaim::Conflict => {
                return Ok(SignupOutcome::Refused(SignupRefusal::StaleForm));
            }
        }
        let result = match self.provider() {
            Some(provider) => self
                .provider_create_account(
                    provider.as_ref(),
                    &email,
                    form.password,
                    &form.terms_version,
                    client,
                )
                .await
                .map(|outcome| matches!(outcome, ProviderSignup::Created)),
            None => self
                .create_account(&email, form.password)
                .await
                .map(|()| true),
        };
        match result {
            Ok(false) => {
                // Refused by the provider's policy: the claim was never used.
                self.release_claim(key).await;
                Ok(SignupOutcome::Refused(SignupRefusal::PasswordRejected))
            }
            Ok(true) => {
                write(
                    self.shared
                        .store
                        .finish_operation(key, ReceiptOutcome::Done),
                )
                .await?;
                Ok(SignupOutcome::CheckYourEmail)
            }
            Err(error) => {
                if error != ServiceError::Unknown {
                    // The claim was never used: release it so the form can be sent again.
                    self.release_claim(key).await;
                }
                Err(error)
            }
        }
    }

    /// Forgets a claim whose operation changed nothing, so the corrected form can be sent
    /// again. A failure is audited: the form then waits for the claim to expire.
    pub(crate) async fn release_claim(&self, key: OperationKey) {
        if write(self.shared.store.release_operation(key))
            .await
            .is_err()
        {
            audit(
                "account.claim_not_released",
                format!(
                    "ACCOUNT claim_not_released namespace={}",
                    key.namespace.label()
                ),
            );
        }
    }

    async fn create_account(
        &self,
        email: &EmailAddress,
        password: &str,
    ) -> Result<(), ServiceError> {
        let permit = self.reserve_mail()?;
        // Hash before looking anything up, so new and registered addresses cost the same.
        let password_hash = self.shared.hasher.hash(password).await?;
        let token = SecretToken::generate()?;
        let now_ms = self.now_ms();
        let account_id = new_account_id()?;
        let account = NewAccount {
            id: account_id,
            email: email.clone(),
            password_hash,
            created_at_ms: now_ms,
            verification: NewToken {
                id: new_record_id()?,
                digest: token.digest(),
                account: account_id,
                purpose: TokenPurpose::VerifyEmail,
                email_change: None,
                credential_generation: 1,
                created_at_ms: now_ms,
                expires_at_ms: now_ms + VERIFICATION_TOKEN_LIFETIME_MS,
                attempt_limit: VERIFICATION_ATTEMPT_LIMIT,
            },
            adult_attested_at: now_ms,
            terms_version: self.shared.current_terms_version.clone(),
            terms_accepted_at: now_ms,
        };
        match write(self.shared.store.create_account(account)).await? {
            CreateOutcome::Created => {
                audit(
                    "account.signup",
                    format!("ACCOUNT signup account={}", account_label(account_id)),
                );
                permit.send(MailJob::Deliver(
                    self.shared.messages.verify_email(email.display(), &token),
                ));
            }
            CreateOutcome::EmailTaken => {
                audit(
                    "account.signup",
                    "ACCOUNT signup account=none reason=address_registered".to_owned(),
                );
                let existing = read(self.shared.store.account_by_email(email.normalized()))
                    .await?
                    .and_then(|account| account.email);
                let to = existing.as_ref().unwrap_or(email);
                permit.send(MailJob::Deliver(
                    self.shared.messages.signup_attempt(to.display()),
                ));
            }
        }
        Ok(())
    }

    /// The verification link's page: the owner of the mailbox proves it AND chooses the account's
    /// password in one step. Anyone who signed up with someone else's address loses the password
    /// they chose, and every session made before the proof is revoked, so the sign-up cannot be
    /// kept. With a provider, the provider's password is replaced first (a failure there leaves
    /// the link unspent, so the step can be repeated).
    pub async fn verify_email_and_set_password(
        &self,
        token_text: &str,
        new_password: &str,
        confirmation: &str,
        client: Client,
    ) -> Result<VerifyWithPasswordOutcome, ServiceError> {
        if !self.admit(ThrottleScope::TokenPagePerIp, Subject::Ip(client.ip)) {
            return Ok(VerifyWithPasswordOutcome::Throttled);
        }
        let Some(token) = SecretToken::parse(token_text) else {
            return Ok(VerifyWithPasswordOutcome::InvalidLink);
        };
        if new_password != confirmation {
            return Ok(VerifyWithPasswordOutcome::Mismatch);
        }
        // The address is not known before the link is resolved; the other rules apply.
        if let Err(refusal) = check_new_password(new_password, "", self.is_common()) {
            return Ok(VerifyWithPasswordOutcome::Refused(refusal));
        }
        let Some(target) = read(
            self.shared
                .store
                .verification_target(token.digest(), self.now_ms()),
        )
        .await?
        else {
            return Ok(VerifyWithPasswordOutcome::InvalidLink);
        };
        if target.already_verified {
            return Ok(VerifyWithPasswordOutcome::AlreadyVerified);
        }
        let replacement = match self.provider().cloned() {
            Some(provider) => {
                if !self
                    .provider_replace_password(provider.as_ref(), target.account, new_password)
                    .await?
                {
                    return Ok(VerifyWithPasswordOutcome::ProviderRejectedPassword);
                }
                None
            }
            None => Some(self.shared.hasher.hash(new_password).await?),
        };
        Ok(match self.finish_verification(token, replacement).await? {
            VerifyEmailOutcome::Verified => {
                let Some(account) = read(self.shared.store.account(target.account)).await? else {
                    return Err(ServiceError::InvalidRecord);
                };
                let issued = self
                    .issue_session(&account, client, Some(EventKind::LoggedIn))
                    .await?;
                VerifyWithPasswordOutcome::Verified(issued)
            }
            VerifyEmailOutcome::AlreadyVerified => VerifyWithPasswordOutcome::AlreadyVerified,
            VerifyEmailOutcome::Invalid => VerifyWithPasswordOutcome::InvalidLink,
            VerifyEmailOutcome::Throttled => VerifyWithPasswordOutcome::Throttled,
        })
    }

    async fn finish_verification(
        &self,
        token: SecretToken,
        replacement: Option<crate::tokens::PasswordHash>,
    ) -> Result<VerifyEmailOutcome, ServiceError> {
        let outcome = write(self.shared.store.verify_email(
            token.digest(),
            self.now_ms(),
            replacement,
        ))
        .await?;
        Ok(match outcome {
            VerifyOutcome::Verified(account) => {
                audit(
                    "account.verify",
                    format!("ACCOUNT verify account={}", account_label(account)),
                );
                self.provider_note_verified(account).await;
                VerifyEmailOutcome::Verified
            }
            VerifyOutcome::AlreadyVerified(_) => VerifyEmailOutcome::AlreadyVerified,
            VerifyOutcome::Invalid => VerifyEmailOutcome::Invalid,
        })
    }

    pub async fn resend_verification(
        &self,
        caller: &Authenticated,
    ) -> Result<ResendOutcome, ServiceError> {
        if caller.account.is_verified {
            return Ok(ResendOutcome::AlreadyVerified);
        }
        if !self.admit(
            ThrottleScope::VerificationResendPerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(ResendOutcome::Throttled);
        }
        let Some(email) = caller.account.email.as_ref() else {
            return Ok(ResendOutcome::AlreadyVerified);
        };
        let permit = self.reserve_mail()?;
        let token = SecretToken::generate()?;
        let now_ms = self.now_ms();
        write(self.shared.store.issue_token(NewToken {
            id: new_record_id()?,
            digest: token.digest(),
            account: caller.account.id,
            purpose: TokenPurpose::VerifyEmail,
            email_change: None,
            credential_generation: caller.account.credential_generation,
            created_at_ms: now_ms,
            expires_at_ms: now_ms + VERIFICATION_TOKEN_LIFETIME_MS,
            attempt_limit: VERIFICATION_ATTEMPT_LIMIT,
        }))
        .await?;
        permit.send(MailJob::Deliver(
            self.shared.messages.verify_email(email.display(), &token),
        ));
        Ok(ResendOutcome::Sent)
    }

    pub async fn login(
        &self,
        email_text: &str,
        password: &str,
        client: Client,
    ) -> Result<LoginOutcome, ServiceError> {
        if !self.admit(ThrottleScope::LoginPerIp, Subject::Ip(client.ip)) {
            return Ok(LoginOutcome::Throttled);
        }
        let started = Instant::now();
        let provider = self.provider().cloned();
        let Ok(email) = EmailAddress::parse(email_text) else {
            if provider.is_none() {
                self.shared.hasher.verify_dummy(password).await?;
            }
            self.pad_sign_in_failure(started).await;
            return Ok(LoginOutcome::Refused);
        };
        let failures = Subject::Email(email.normalized());
        if !self.allowed(ThrottleScope::LoginFailuresPerEmail, failures) {
            return Ok(LoginOutcome::Throttled);
        }
        // Failures are counted when a check ends; this keeps a burst of parallel guesses from
        // all running before the first of them is counted.
        let Some(_slot) = self.shared.throttles.enter_sign_in(email.normalized()) else {
            return Ok(LoginOutcome::Throttled);
        };
        if let Some(provider) = provider {
            return self
                .provider_login(provider.as_ref(), &email, password, client, started)
                .await;
        }
        let account = read(self.shared.store.account_by_email(email.normalized())).await?;
        let Some((account, hash)) = account.and_then(|account| {
            let hash = account.password_hash.clone()?;
            Some((account, hash))
        }) else {
            self.shared.hasher.verify_dummy(password).await?;
            self.count_failure(ThrottleScope::LoginFailuresPerEmail, failures);
            self.queue_login_refusal(
                email.normalized(),
                AccountEvent {
                    kind: EventKind::LoginRefused,
                    at_ms: self.now_ms(),
                    device: client.device,
                },
            );
            audit(
                "account.login",
                format!(
                    "ACCOUNT login_refused account=none reason=credentials ms={}",
                    started.elapsed().as_millis()
                ),
            );
            self.pad_sign_in_failure(started).await;
            return Ok(LoginOutcome::Refused);
        };
        let verification = self.shared.hasher.verify(password, &hash).await?;
        if !verification.is_match {
            self.count_failure(ThrottleScope::LoginFailuresPerEmail, failures);
            self.queue_login_refusal(
                email.normalized(),
                AccountEvent {
                    kind: EventKind::LoginRefused,
                    at_ms: self.now_ms(),
                    device: client.device,
                },
            );
            audit(
                "account.login",
                format!(
                    "ACCOUNT login_refused account={} reason=credentials ms={}",
                    account_label(account.id),
                    started.elapsed().as_millis()
                ),
            );
            self.pad_sign_in_failure(started).await;
            return Ok(LoginOutcome::Refused);
        }
        if account.status == AccountStatus::Closing && !self.reopen(account.id).await? {
            // Past its window (or not closing any more): no session is issued.
            return Ok(LoginOutcome::Refused);
        }
        if verification.needs_rehash {
            let rehashed = self.shared.hasher.hash(password).await?;
            write(self.shared.store.rehash_password(
                account.id,
                account.credential_generation,
                rehashed,
            ))
            .await?;
        }
        let issued = self
            .issue_session(&account, client, Some(EventKind::LoggedIn))
            .await?;
        audit(
            "account.login",
            format!(
                "ACCOUNT login account={} ms={}",
                account_label(account.id),
                started.elapsed().as_millis()
            ),
        );
        Ok(LoginOutcome::SignedIn(issued))
    }

    /// Reopens a closing account whose window has not passed; `false` when nothing changed.
    pub(crate) async fn reopen(&self, account: df_types::AccountId) -> Result<bool, ServiceError> {
        let reopened = write(self.shared.store.reopen_account(account, self.now_ms())).await?;
        if reopened {
            audit(
                "account.reopened",
                format!("ACCOUNT reopened account={}", account_label(account)),
            );
        }
        Ok(reopened)
    }

    /// Inserts a new session for `account` at its current generation.
    pub(crate) async fn issue_session(
        &self,
        account: &AccountRecord,
        client: Client,
        event: Option<EventKind>,
    ) -> Result<IssuedSession, ServiceError> {
        let (session, token) =
            self.new_session(account.id, account.credential_generation, client)?;
        let event = event.map(|kind| AccountEvent {
            kind,
            at_ms: session.created_at_ms,
            device: client.device,
        });
        write(self.shared.store.create_session(session, event)).await?;
        Ok(IssuedSession {
            token,
            max_age_s: self.shared.tunables.sessions.absolute.as_secs(),
        })
    }

    pub(crate) fn new_session(
        &self,
        account: df_types::AccountId,
        credential_generation: u64,
        client: Client,
    ) -> Result<(NewSession, SecretToken), ServiceError> {
        let token = SecretToken::generate()?;
        let now_ms = self.now_ms();
        let absolute_ms =
            u64::try_from(self.shared.tunables.sessions.absolute.as_millis()).unwrap_or(u64::MAX);
        Ok((
            NewSession {
                id: new_session_id()?,
                digest: token.digest(),
                account,
                credential_generation,
                created_at_ms: now_ms,
                absolute_expires_at_ms: now_ms.saturating_add(absolute_ms),
                csrf_secret: CsrfSecret::generate()?,
                device: client.device,
            },
            token,
        ))
    }

    /// Resolves a session cookie to its caller, or `None` for anything not currently valid.
    pub async fn authenticate(
        &self,
        token_text: &str,
    ) -> Result<Option<Authenticated>, ServiceError> {
        let Some(token) = SecretToken::parse(token_text) else {
            return Ok(None);
        };
        let Some((session, account)) =
            read(self.shared.store.session_by_digest(token.digest())).await?
        else {
            return Ok(None);
        };
        if account.status != AccountStatus::Active {
            return Ok(None);
        }
        let now_ms = self.now_ms();
        let check = check_session(
            StoredSession {
                created_at_ms: session.created_at_ms,
                last_seen_at_ms: session.last_seen_at_ms,
                is_revoked: session.is_revoked,
                credential_generation: session.credential_generation,
            },
            account.credential_generation,
            self.shared.tunables.sessions,
            now_ms,
        );
        match check {
            SessionCheck::Active { needs_touch } => {
                if needs_touch {
                    write(self.shared.store.touch_session(session.id, now_ms)).await?;
                }
                Ok(Some(Authenticated {
                    session: session.id,
                    signed_in_at_ms: session.created_at_ms,
                    account,
                    csrf: session.csrf_secret,
                }))
            }
            SessionCheck::Revoked
            | SessionCheck::StaleGeneration
            | SessionCheck::IdleExpired
            | SessionCheck::AbsoluteExpired => Ok(None),
        }
    }

    pub async fn logout(&self, caller: &Authenticated, client: Client) -> Result<(), ServiceError> {
        self.revoke(
            caller,
            RevokeScope::One(caller.session),
            RevokeReason::Logout,
            EventKind::LoggedOut,
            client,
        )
        .await
        .map(|_| ())
    }

    /// Revokes the caller's own sessions. Another account's session id revokes nothing.
    pub async fn revoke_sessions(
        &self,
        caller: &Authenticated,
        scope: RevokeScope,
        client: Client,
    ) -> Result<u64, ServiceError> {
        let reason = match scope {
            RevokeScope::One(_) => RevokeReason::RevokedByUser,
            RevokeScope::AllExcept(_) | RevokeScope::All => RevokeReason::SignOutEverywhere,
        };
        self.revoke(caller, scope, reason, EventKind::SessionsRevoked, client)
            .await
    }

    async fn revoke(
        &self,
        caller: &Authenticated,
        scope: RevokeScope,
        reason: RevokeReason,
        kind: EventKind,
        client: Client,
    ) -> Result<u64, ServiceError> {
        let now_ms = self.now_ms();
        let event = AccountEvent {
            kind,
            at_ms: now_ms,
            device: client.device,
        };
        let revoked = write(self.shared.store.revoke_sessions(
            caller.account.id,
            scope,
            reason,
            now_ms,
            Some(event),
        ))
        .await?;
        audit(
            "account.sessions_revoked",
            format!(
                "ACCOUNT sessions_revoked account={} reason={} count={revoked}",
                account_label(caller.account.id),
                reason.label()
            ),
        );
        Ok(revoked)
    }
}
