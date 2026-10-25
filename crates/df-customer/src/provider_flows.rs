//! The account flows when the credentials live at an identity provider (decision 28). The service
//! entry points (`signup`, `login`, `verify_email_and_set_password`, password reset,
//! re-authentication) call these when `IdentitySource` is a provider; the person-visible answers
//! are the same as with local passwords: a new and a registered address look alike, a wrong
//! password and an unknown address look alike, and every email is ours.
//!
//! Division of labor: the provider keeps the password and answers "is this password right?"; we
//! keep the account row, the sessions, the verified flag (set only by OUR emailed link; the
//! provider's own email verification is off), the throttles and every message. The provider's own
//! session is ended the moment it is created, because our cookie is the only session there is.

use std::{sync::Arc, time::Instant};

use df_types::AccountId;

use crate::{
    access::{Authenticated, LoginOutcome},
    credentials::ResetConfirmOutcome,
    ports::{AccountStore, EmailSender},
    provider::{
        Authentication, CallerContext, CreateUser, ErasedProvider, IdentityProviderKind,
        MAXIMUM_PROVIDER_TOKEN_BYTES, MAXIMUM_SUBJECT_BYTES, ProviderUser, ResetFinish,
        SetPassword,
    },
    records::{
        AccountEvent, AccountRecord, AccountStatus, CreateOutcome, EmailAddress, EventKind,
        NewIdentityAccount, NewToken, ResetOutcome, TokenPurpose,
    },
    reset_authority::apply_reset_policy,
    service::{
        Client, CustomerService, MailJob, PROVIDER_DEADLINE, ServiceError, account_label, audit,
        provider_call, read, write,
    },
    throttle::{Subject, ThrottleScope},
    tokens::{SecretToken, TokenDigest, new_account_id, new_record_id, tagged_digest},
};

/// A verification link's lifetime (the same as local passwords).
use crate::access::VERIFICATION_TOKEN_LIFETIME_MS;

/// What a provider password check meant for the person.
pub(crate) enum PasswordCheck {
    Verified(ProviderUser),
    /// Wrong password, unknown address, or an address the provider will not authenticate
    /// because it has not verified it (our verification is the only one in use): one answer.
    Invalid,
}

/// How a sign-up at the provider ended.
pub(crate) enum ProviderSignup {
    Created,
    PasswordRejected,
}

/// A provider reset token's shape: opaque and URL-safe, 16 to 512 bytes. Anything else is not
/// worth a store lookup or a provider call.
pub(crate) fn is_plausible_provider_token(text: &str) -> bool {
    (16..=MAXIMUM_PROVIDER_TOKEN_BYTES).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~'))
}

/// The digest a provider reset token is stored under (the token itself is never stored).
pub(crate) fn provider_token_digest(token: &str) -> TokenDigest {
    TokenDigest::from_bytes(tagged_digest("df-provider-reset", token.as_bytes()))
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    pub(crate) fn provider(&self) -> Option<&Arc<dyn ErasedProvider>> {
        self.shared.identity.erased()
    }

    /// Ends a provider-side session we never use. A failure is audited, not shown: the session
    /// holds no capability here and expires at the provider.
    async fn end_provider_session(&self, provider: &dyn ErasedProvider, session: Option<String>) {
        let Some(session) = session else { return };
        let ended = tokio::time::timeout(PROVIDER_DEADLINE, provider.end_session(&session)).await;
        if !matches!(ended, Ok(Ok(()))) {
            audit(
                "account.provider_session_left",
                format!(
                    "ACCOUNT provider_session_left provider={}",
                    provider.kind().label()
                ),
            );
        }
    }

    /// Creates the user at the provider and the account here. A registered address sends the
    /// owner a notice and nothing else; an unverified provider user with no account here (the
    /// leftover of a sign-up that failed between the two writes) is replaced.
    pub(crate) async fn provider_create_account(
        &self,
        provider: &dyn ErasedProvider,
        email: &EmailAddress,
        password: &str,
        terms_version: &str,
        client: Client,
    ) -> Result<ProviderSignup, ServiceError> {
        let permit = self.reserve_mail()?;
        let caller = CallerContext { ip: client.ip };
        let mut created =
            provider_call(true, provider.create_user(email, password, caller)).await?;
        if created == CreateUser::EmailNotAvailable
            && read(self.shared.store.account_by_email(email.normalized()))
                .await?
                .is_none()
        {
            let found = provider_call(false, provider.find_user(email)).await?;
            if let Some(leftover) = found.filter(|user| !user.email_verified) {
                provider_call(true, provider.delete_user(&leftover.subject)).await?;
                audit(
                    "account.orphan_reclaimed",
                    format!(
                        "ACCOUNT orphan_reclaimed provider={}",
                        provider.kind().label()
                    ),
                );
                created =
                    provider_call(true, provider.create_user(email, password, caller)).await?;
            }
        }
        match created {
            CreateUser::PasswordRejected => Ok(ProviderSignup::PasswordRejected),
            CreateUser::EmailNotAvailable => {
                self.warn_registered_owner(email, permit).await?;
                Ok(ProviderSignup::Created)
            }
            CreateUser::Created(user) => {
                let token = SecretToken::generate()?;
                let now_ms = self.now_ms();
                let account_id = new_account_id()?;
                let outcome = write(self.shared.store.create_identity_account(
                    NewIdentityAccount {
                        id: account_id,
                        email: email.clone(),
                        is_verified: false,
                        provider: provider.kind(),
                        subject: user.subject.clone(),
                        created_at_ms: now_ms,
                        adult_attested_at: Some(now_ms),
                        terms_version: Some(terms_version.to_owned()),
                        terms_accepted_at: Some(now_ms),
                        verification: Some(NewToken {
                            id: new_record_id()?,
                            digest: token.digest(),
                            account: account_id,
                            purpose: TokenPurpose::VerifyEmail,
                            email_change: None,
                            credential_generation: 1,
                            created_at_ms: now_ms,
                            expires_at_ms: now_ms + VERIFICATION_TOKEN_LIFETIME_MS,
                            attempt_limit: 1,
                        }),
                    },
                ))
                .await?;
                match outcome {
                    CreateOutcome::Created => {
                        audit(
                            "account.signup",
                            format!(
                                "ACCOUNT signup account={} provider={}",
                                account_label(account_id),
                                provider.kind().label()
                            ),
                        );
                        permit.send(MailJob::Deliver(
                            self.shared.messages.verify_email(email.display(), &token),
                        ));
                    }
                    CreateOutcome::EmailTaken => {
                        // An account here already uses the address (a race, or an account made
                        // before the provider): the user just created has no account. Remove it.
                        if provider_call(true, provider.delete_user(&user.subject))
                            .await
                            .is_err()
                        {
                            audit(
                                "account.orphan_left",
                                format!("ACCOUNT orphan_left provider={}", provider.kind().label()),
                            );
                        }
                        self.warn_registered_owner(email, permit).await?;
                    }
                }
                Ok(ProviderSignup::Created)
            }
        }
    }

    /// Tells the owner of a registered address that someone tried to sign up with it.
    async fn warn_registered_owner(
        &self,
        email: &EmailAddress,
        permit: tokio::sync::mpsc::Permit<'_, MailJob>,
    ) -> Result<(), ServiceError> {
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
        Ok(())
    }

    /// Asks the provider whether `password` is right for `email`. An address the provider will
    /// not authenticate for want of its own verification (which is switched off for us) is a
    /// refusal like any other.
    pub(crate) async fn provider_check_password(
        &self,
        provider: &dyn ErasedProvider,
        email: &EmailAddress,
        password: &str,
        caller: CallerContext,
    ) -> Result<PasswordCheck, ServiceError> {
        match provider_call(false, provider.authenticate(email, password, caller)).await? {
            Authentication::Authenticated {
                user,
                provider_session,
            } => {
                self.end_provider_session(provider, provider_session).await;
                Ok(PasswordCheck::Verified(user))
            }
            Authentication::InvalidCredentials | Authentication::EmailVerificationRequired => {
                Ok(PasswordCheck::Invalid)
            }
        }
    }

    pub(crate) async fn provider_login(
        &self,
        provider: &dyn ErasedProvider,
        email: &EmailAddress,
        password: &str,
        client: Client,
        started: Instant,
    ) -> Result<LoginOutcome, ServiceError> {
        let caller = CallerContext { ip: client.ip };
        let failures = Subject::Email(email.normalized());
        let user = match self
            .provider_check_password(provider, email, password, caller)
            .await?
        {
            PasswordCheck::Verified(user) => user,
            PasswordCheck::Invalid => {
                self.count_failure(ThrottleScope::LoginFailuresPerEmail, failures);
                let known = read(self.shared.store.account_by_email(email.normalized())).await?;
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
                        known
                            .map_or_else(|| "none".to_owned(), |account| account_label(account.id)),
                        started.elapsed().as_millis()
                    ),
                );
                self.pad_sign_in_failure(started).await;
                return Ok(LoginOutcome::Refused);
            }
        };
        let Some(account) = self
            .account_for_provider_user(provider.kind(), user)
            .await?
        else {
            self.count_failure(ThrottleScope::LoginFailuresPerEmail, failures);
            return Ok(LoginOutcome::Refused);
        };
        if account.status == AccountStatus::Closing && !self.reopen(account.id).await? {
            // Past its window: no session is issued.
            return Ok(LoginOutcome::Refused);
        }
        let issued = self
            .issue_session(&account, client, Some(EventKind::LoggedIn))
            .await?;
        audit(
            "account.login",
            format!(
                "ACCOUNT login account={} provider={} ms={}",
                account_label(account.id),
                provider.kind().label(),
                started.elapsed().as_millis()
            ),
        );
        Ok(LoginOutcome::SignedIn(issued))
    }

    /// The account a provider user is. An existing link wins. An account found only by address is
    /// linked only when BOTH sides say the address is verified AND the account has no link to
    /// this provider yet (a second provider user with the same address must not take over an
    /// account already tied to another). A user the provider has and we do not (made by an
    /// operator at the provider) gets an account. `None` is a refusal.
    async fn account_for_provider_user(
        &self,
        kind: IdentityProviderKind,
        user: ProviderUser,
    ) -> Result<Option<AccountRecord>, ServiceError> {
        let store = &self.shared.store;
        let now_ms = self.now_ms();
        if user.subject.is_empty() || user.subject.len() > MAXIMUM_SUBJECT_BYTES {
            return Ok(None);
        }
        let account = match read(store.account_for_identity(kind, &user.subject)).await? {
            Some(account) => account,
            None => match read(store.account_by_email(user.email.normalized())).await? {
                Some(existing) => {
                    let already_linked = read(store.identity_for_account(existing.id, kind))
                        .await?
                        .is_some();
                    if already_linked || !(existing.is_verified && user.email_verified) {
                        audit(
                            "account.sign_in_refused",
                            format!(
                                "ACCOUNT sign_in_refused provider={} reason={}",
                                kind.label(),
                                if already_linked {
                                    "account_already_linked"
                                } else {
                                    "unverified_link"
                                }
                            ),
                        );
                        return Ok(None);
                    }
                    write(store.link_identity(existing.id, kind, &user.subject, now_ms)).await?;
                    audit(
                        "account.identity_linked",
                        format!(
                            "ACCOUNT identity_linked account={} provider={}",
                            account_label(existing.id),
                            kind.label()
                        ),
                    );
                    existing
                }
                None => {
                    let id = new_account_id()?;
                    let created = write(store.create_identity_account(NewIdentityAccount {
                        id,
                        email: user.email,
                        is_verified: user.email_verified,
                        provider: kind,
                        subject: user.subject,
                        created_at_ms: now_ms,
                        adult_attested_at: None,
                        terms_version: None,
                        terms_accepted_at: None,
                        verification: None,
                    }))
                    .await?;
                    if created == CreateOutcome::EmailTaken {
                        return Ok(None);
                    }
                    audit(
                        "account.signup",
                        format!(
                            "ACCOUNT signup account={} provider={}",
                            account_label(id),
                            kind.label()
                        ),
                    );
                    read(store.account(id))
                        .await?
                        .ok_or(ServiceError::InvalidRecord)?
                }
            },
        };
        Ok((account.status != AccountStatus::Closed).then_some(account))
    }

    /// After our emailed link verified `account`, tells the provider (best effort: the provider's
    /// own flag gates nothing, because its email verification is off).
    pub(crate) async fn provider_note_verified(&self, account: AccountId) {
        let Some(provider) = self.provider() else {
            return;
        };
        let subject = read(
            self.shared
                .store
                .identity_for_account(account, provider.kind()),
        )
        .await;
        let result = match subject {
            Ok(Some(subject)) => provider_call(true, provider.mark_email_verified(&subject)).await,
            Ok(None) => return,
            Err(error) => Err(error),
        };
        if result.is_err() {
            audit(
                "account.provider_verify_pending",
                format!(
                    "ACCOUNT provider_verify_pending account={}",
                    account_label(account)
                ),
            );
        }
    }

    /// Replaces the provider's password for `account` (the verify-and-set-password step).
    /// `false` when the provider's own policy refused the password.
    pub(crate) async fn provider_replace_password(
        &self,
        provider: &dyn ErasedProvider,
        account: AccountId,
        password: &str,
    ) -> Result<bool, ServiceError> {
        let subject = read(
            self.shared
                .store
                .identity_for_account(account, provider.kind()),
        )
        .await?
        .ok_or(ServiceError::InvalidRecord)?;
        Ok(matches!(
            provider_call(true, provider.set_password(&subject, password)).await?,
            SetPassword::Set
        ))
    }

    /// Re-authentication for a sensitive action: the provider checks the password, and the user
    /// it answers with must be this account's own.
    pub(crate) async fn provider_password_matches(
        &self,
        provider: &dyn ErasedProvider,
        caller: &Authenticated,
        password: &str,
    ) -> Result<bool, ServiceError> {
        let Some(email) = caller.account.email.as_ref() else {
            return Ok(false);
        };
        let _slot = self
            .shared
            .throttles
            .enter_sign_in(email.normalized())
            .ok_or(ServiceError::Busy)?;
        let check = self
            .provider_check_password(provider, email, password, CallerContext::default())
            .await?;
        let PasswordCheck::Verified(user) = check else {
            return Ok(false);
        };
        let linked = read(
            self.shared
                .store
                .identity_for_account(caller.account.id, provider.kind()),
        )
        .await?;
        Ok(linked.as_deref() == Some(user.subject.as_str()))
    }

    /// Finishes a reset with the provider's token. The token was stored by digest when the email
    /// was queued, so it is spent here first: that bumps the credential generation and revokes
    /// every session of ours before the provider is asked to change anything. If revoking fails,
    /// the provider is never called; if the provider then fails, our sessions are already gone
    /// and the password is unchanged. Either way no old session outlives a reset.
    pub(crate) async fn provider_finish_reset(
        &self,
        provider: &dyn ErasedProvider,
        token: &str,
        new_password: &str,
    ) -> Result<ResetConfirmOutcome, ServiceError> {
        let permit = self.reserve_mail()?;
        let digest = provider_token_digest(token);
        let now_ms = self.now_ms();
        let operation = self.new_operation()?;
        let policy = Box::new(move |snapshot: &mut crate::records::ResetSnapshot| {
            apply_reset_policy(snapshot, digest, operation, now_ms)
        });
        let account = match write(
            self.shared
                .store
                .consume_reset_token(digest, now_ms, policy),
        )
        .await?
        {
            ResetOutcome::Reset(account) => account,
            ResetOutcome::Refused(_) | ResetOutcome::Invalid => {
                return Ok(ResetConfirmOutcome::InvalidLink);
            }
        };
        let finished =
            provider_call(true, provider.finish_password_reset(token, new_password)).await?;
        match finished {
            ResetFinish::Reset(_) => {}
            ResetFinish::InvalidToken => return Ok(ResetConfirmOutcome::InvalidLink),
            ResetFinish::PasswordRejected => {
                return Ok(ResetConfirmOutcome::ProviderRejectedPassword);
            }
        }
        audit(
            "account.reset_completed",
            format!("ACCOUNT reset_completed account={}", account_label(account)),
        );
        if let Some(email) = read(self.shared.store.account(account))
            .await?
            .and_then(|account| account.email)
        {
            permit.send(MailJob::Deliver(
                self.shared.messages.password_changed(email.display()),
            ));
        }
        Ok(ResetConfirmOutcome::Reset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_token_has_a_bounded_url_safe_shape() {
        assert!(is_plausible_provider_token("reset_token_000042_abcdef"));
        for bad in [
            "",
            "short",
            "has space in it which is long",
            "unicode\u{e9}unicode\u{e9}unicode",
            &"a".repeat(513),
        ] {
            assert!(!is_plausible_provider_token(bad), "{bad}");
        }
    }

    #[test]
    fn the_stored_digest_is_domain_separated_from_our_own_tokens() {
        let provider = provider_token_digest("0123456789abcdef");
        let ours = TokenDigest::from_bytes(tagged_digest("df-token", b"0123456789abcdef"));
        assert!(provider != ours);
    }
}
