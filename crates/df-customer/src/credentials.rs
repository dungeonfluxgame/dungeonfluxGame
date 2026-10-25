//! Changing a password, resetting a forgotten one, and changing the email address.
//!
//! Each success bumps the credential generation and revokes the other sessions (G04-D01).
//! Reset requests are only queued here; the mail worker resolves the address (plan 6.3).

use df_auth::{
    password_policy::{PasswordRefusal, check_new_password},
    recovery::AccountRefusal,
};
use df_types::OperationId;

use crate::{
    access::{Authenticated, IssuedSession},
    ports::{AccountStore, EmailSender},
    records::{
        ConfirmChangeOutcome, EmailAddress, EmailChangeDecision, EmailChangeOutcome,
        EmailChangeRequest, EmailRefusal, NewToken, OperationClaim, OperationKey,
        OperationNamespace, PasswordChange, PasswordChangeOutcome, ReceiptOutcome, ResetOutcome,
        TokenPurpose,
    },
    reset_authority::apply_reset_policy,
    service::{Client, CustomerService, MailJob, ServiceError, account_label, audit, read, write},
    throttle::{Subject, ThrottleScope},
    tokens::{SecretToken, new_record_id, tagged_digest},
};

/// An email change must be confirmed by both addresses within this time.
pub const EMAIL_CHANGE_LIFETIME_MS: u64 = 24 * 60 * 60 * 1000;
const EMAIL_CHANGE_ATTEMPT_LIMIT: u32 = 1;

pub enum PasswordChangeResult {
    /// The new session replacing the current one (the old cookie is revoked).
    Changed(IssuedSession),
    /// The identity provider holds the password; the person resets it by email instead.
    ProviderManaged,
    WrongPassword,
    Mismatch,
    Refused(PasswordRefusal),
    Throttled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResetRequestOutcome {
    /// The same page for every address.
    Accepted,
    Throttled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResetConfirmOutcome {
    Reset,
    Mismatch,
    Refused(PasswordRefusal),
    /// Unknown, used, expired or exhausted link: one message for all.
    InvalidLink,
    /// The identity provider's own password policy refused the password.
    ProviderRejectedPassword,
    Throttled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailChangeResult {
    /// The identity provider holds the address; changing it is not offered yet.
    ProviderManaged,
    /// The same page whether or not the new address is free.
    ConfirmationSent,
    WrongPassword,
    Refused(EmailRefusal),
    SameAddress,
    StaleForm,
    Throttled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmailConfirmResult {
    AwaitingOtherAddress,
    Changed,
    Cancelled,
    AddressTaken,
    InvalidLink,
    Throttled,
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    /// Checks the caller's current password (for changes that need it).
    pub(crate) async fn password_matches(
        &self,
        caller: &Authenticated,
        password: &str,
    ) -> Result<bool, ServiceError> {
        if let Some(provider) = self.provider() {
            return self
                .provider_password_matches(provider.as_ref(), caller, password)
                .await;
        }
        let Some(hash) = caller.account.password_hash.as_ref() else {
            return Ok(false);
        };
        Ok(self.shared.hasher.verify(password, hash).await?.is_match)
    }

    pub async fn change_password(
        &self,
        caller: &Authenticated,
        current: &str,
        new: &str,
        confirmation: &str,
        client: Client,
    ) -> Result<PasswordChangeResult, ServiceError> {
        if !self.uses_local_passwords() {
            return Ok(PasswordChangeResult::ProviderManaged);
        }
        if !self.admit(
            ThrottleScope::PasswordChangePerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(PasswordChangeResult::Throttled);
        }
        if new != confirmation {
            return Ok(PasswordChangeResult::Mismatch);
        }
        let email = caller
            .account
            .email
            .as_ref()
            .map_or("", |email| email.display());
        if let Err(refusal) = check_new_password(new, email, self.is_common()) {
            return Ok(PasswordChangeResult::Refused(refusal));
        }
        if !self.password_matches(caller, current).await? {
            return Ok(PasswordChangeResult::WrongPassword);
        }
        let permit = self.reserve_mail()?;
        let new_hash = self.shared.hasher.hash(new).await?;
        let next_generation = caller.account.credential_generation.saturating_add(1);
        let (replacement, token) = self.new_session(caller.account.id, next_generation, client)?;
        let change = PasswordChange {
            account: caller.account.id,
            expected_generation: caller.account.credential_generation,
            new_hash,
            replacement,
            at_ms: self.now_ms(),
        };
        match write(self.shared.store.change_password(change)).await? {
            PasswordChangeOutcome::Changed => {}
            // Another change won the race: this caller's session is already revoked.
            PasswordChangeOutcome::StaleGeneration => {
                return Ok(PasswordChangeResult::WrongPassword);
            }
        }
        audit(
            "account.password_changed",
            format!(
                "ACCOUNT password_changed account={}",
                account_label(caller.account.id)
            ),
        );
        if let Some(email) = caller.account.email.as_ref() {
            permit.send(MailJob::Deliver(
                self.shared.messages.password_changed(email.display()),
            ));
        }
        Ok(PasswordChangeResult::Changed(IssuedSession {
            token,
            max_age_s: self.shared.tunables.sessions.absolute.as_secs(),
        }))
    }

    /// Queues a reset for the worker. The answer never depends on whether the address exists.
    pub async fn request_password_reset(
        &self,
        email_text: &str,
        client: Client,
    ) -> Result<ResetRequestOutcome, ServiceError> {
        if !self.admit(ThrottleScope::ResetPerIp, Subject::Ip(client.ip)) {
            return Ok(ResetRequestOutcome::Throttled);
        }
        let Ok(email) = EmailAddress::parse(email_text) else {
            return Ok(ResetRequestOutcome::Accepted);
        };
        if !self.admit(
            ThrottleScope::ResetPerEmail,
            Subject::Email(email.normalized()),
        ) {
            return Ok(ResetRequestOutcome::Throttled);
        }
        let permit = self.reserve_mail()?;
        permit.send(MailJob::PasswordReset {
            normalized_email: email.normalized().to_owned(),
        });
        Ok(ResetRequestOutcome::Accepted)
    }

    pub async fn confirm_password_reset(
        &self,
        token_text: &str,
        new: &str,
        confirmation: &str,
        client: Client,
    ) -> Result<ResetConfirmOutcome, ServiceError> {
        if !self.admit(ThrottleScope::TokenPagePerIp, Subject::Ip(client.ip)) {
            return Ok(ResetConfirmOutcome::Throttled);
        }
        let provider = self.provider().cloned();
        let token = match provider {
            Some(_) if crate::provider_flows::is_plausible_provider_token(token_text) => None,
            Some(_) => return Ok(ResetConfirmOutcome::InvalidLink),
            None => match SecretToken::parse(token_text) {
                Some(token) => Some(token),
                None => return Ok(ResetConfirmOutcome::InvalidLink),
            },
        };
        if new != confirmation {
            return Ok(ResetConfirmOutcome::Mismatch);
        }
        // The address is not known before the token is resolved; the other rules apply.
        if let Err(refusal) = check_new_password(new, "", self.is_common()) {
            return Ok(ResetConfirmOutcome::Refused(refusal));
        }
        let Some(token) = token else {
            let Some(provider) = provider else {
                return Ok(ResetConfirmOutcome::InvalidLink);
            };
            return self
                .provider_finish_reset(provider.as_ref(), token_text, new)
                .await;
        };
        let permit = self.reserve_mail()?;
        let new_hash = self.shared.hasher.hash(new).await?;
        let digest = token.digest();
        let now_ms = self.now_ms();
        let operation: OperationId = self.new_operation()?;
        let policy = Box::new(move |snapshot: &mut crate::records::ResetSnapshot| {
            apply_reset_policy(snapshot, digest, operation, now_ms)
        });
        let outcome = write(
            self.shared
                .store
                .reset_password(digest, new_hash, now_ms, policy),
        )
        .await?;
        match outcome {
            ResetOutcome::Reset(account) => {
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
            ResetOutcome::Refused(refusal) => {
                audit(
                    "account.reset_refused",
                    format!("ACCOUNT reset_refused reason={}", refusal_label(refusal)),
                );
                Ok(ResetConfirmOutcome::InvalidLink)
            }
            ResetOutcome::Invalid => Ok(ResetConfirmOutcome::InvalidLink),
        }
    }

    pub async fn request_email_change(
        &self,
        caller: &Authenticated,
        password: &str,
        new_email_text: &str,
        operation: OperationId,
    ) -> Result<EmailChangeResult, ServiceError> {
        if !self.uses_local_passwords() {
            return Ok(EmailChangeResult::ProviderManaged);
        }
        if !self.admit(
            ThrottleScope::EmailChangePerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(EmailChangeResult::Throttled);
        }
        let new_email = match EmailAddress::parse(new_email_text) {
            Ok(email) => email,
            Err(refusal) => return Ok(EmailChangeResult::Refused(refusal)),
        };
        let Some(old_email) = caller.account.email.clone() else {
            return Ok(EmailChangeResult::WrongPassword);
        };
        if new_email.normalized() == old_email.normalized() {
            return Ok(EmailChangeResult::SameAddress);
        }
        if !self.password_matches(caller, password).await? {
            return Ok(EmailChangeResult::WrongPassword);
        }
        if !self.operation_is_current(&operation) {
            return Ok(EmailChangeResult::StaleForm);
        }
        let key = OperationKey {
            namespace: OperationNamespace::EmailChange,
            principal: tagged_digest("df-account-principal", caller.account.id.as_bytes()),
            operation,
        };
        let fingerprint = tagged_digest("df-email-change", new_email.normalized().as_bytes());
        match write(
            self.shared
                .store
                .claim_operation(key, fingerprint, self.now_ms()),
        )
        .await?
        {
            OperationClaim::New => {}
            OperationClaim::Replayed(_) | OperationClaim::InProgress => {
                return Ok(EmailChangeResult::ConfirmationSent);
            }
            OperationClaim::Conflict => return Ok(EmailChangeResult::StaleForm),
        }
        let sent = self.send_email_change(caller, &old_email, new_email).await;
        match sent {
            Ok(()) => {
                write(
                    self.shared
                        .store
                        .finish_operation(key, ReceiptOutcome::Done),
                )
                .await?;
                Ok(EmailChangeResult::ConfirmationSent)
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

    async fn send_email_change(
        &self,
        caller: &Authenticated,
        old_email: &EmailAddress,
        new_email: EmailAddress,
    ) -> Result<(), ServiceError> {
        // Both messages or neither: reserve both queue slots first.
        let old_permit = self.reserve_mail()?;
        let new_permit = self.reserve_mail()?;
        let now_ms = self.now_ms();
        let change_id = new_record_id()?;
        let old_token = SecretToken::generate()?;
        let new_token = SecretToken::generate()?;
        let token = |purpose, digest| -> Result<NewToken, ServiceError> {
            Ok(NewToken {
                id: new_record_id()?,
                digest,
                account: caller.account.id,
                purpose,
                email_change: Some(change_id),
                credential_generation: caller.account.credential_generation,
                created_at_ms: now_ms,
                expires_at_ms: now_ms + EMAIL_CHANGE_LIFETIME_MS,
                attempt_limit: EMAIL_CHANGE_ATTEMPT_LIMIT,
            })
        };
        let request = EmailChangeRequest {
            id: change_id,
            account: caller.account.id,
            new_email: new_email.clone(),
            old_address_token: token(TokenPurpose::EmailChangeOld, old_token.digest())?,
            new_address_token: token(TokenPurpose::EmailChangeNew, new_token.digest())?,
            expires_at_ms: now_ms + EMAIL_CHANGE_LIFETIME_MS,
        };
        match write(self.shared.store.request_email_change(request, now_ms)).await? {
            EmailChangeOutcome::Requested => {
                audit(
                    "account.email_change_requested",
                    format!(
                        "ACCOUNT email_change_requested account={}",
                        account_label(caller.account.id)
                    ),
                );
                old_permit.send(MailJob::Deliver(self.shared.messages.email_change_old(
                    old_email.display(),
                    new_email.display(),
                    &old_token,
                )));
                new_permit.send(MailJob::Deliver(
                    self.shared
                        .messages
                        .email_change_new(new_email.display(), &new_token),
                ));
            }
            // Nothing goes to an address that belongs to another account.
            EmailChangeOutcome::AddressTaken => {
                audit(
                    "account.email_change_requested",
                    format!(
                        "ACCOUNT email_change_requested account={} reason=address_registered",
                        account_label(caller.account.id)
                    ),
                );
            }
        }
        Ok(())
    }

    pub async fn confirm_email_change(
        &self,
        token_text: &str,
        decision: EmailChangeDecision,
        client: Client,
    ) -> Result<EmailConfirmResult, ServiceError> {
        if !self.admit(ThrottleScope::TokenPagePerIp, Subject::Ip(client.ip)) {
            return Ok(EmailConfirmResult::Throttled);
        }
        let Some(token) = SecretToken::parse(token_text) else {
            return Ok(EmailConfirmResult::InvalidLink);
        };
        let permits = (self.reserve_mail()?, self.reserve_mail()?);
        let outcome = write(self.shared.store.confirm_email_change(
            token.digest(),
            decision,
            self.now_ms(),
        ))
        .await?;
        Ok(match outcome {
            ConfirmChangeOutcome::AwaitingOtherAddress => EmailConfirmResult::AwaitingOtherAddress,
            ConfirmChangeOutcome::Completed {
                account,
                old_email,
                new_email,
            } => {
                audit(
                    "account.email_changed",
                    format!("ACCOUNT email_changed account={}", account_label(account)),
                );
                let (old_permit, new_permit) = permits;
                for (permit, to) in [(old_permit, &old_email), (new_permit, &new_email)] {
                    permit.send(MailJob::Deliver(
                        self.shared
                            .messages
                            .email_changed(to.display(), new_email.display()),
                    ));
                }
                EmailConfirmResult::Changed
            }
            ConfirmChangeOutcome::Cancelled { account } => {
                audit(
                    "account.email_change_cancelled",
                    format!(
                        "ACCOUNT email_change_cancelled account={}",
                        account_label(account)
                    ),
                );
                EmailConfirmResult::Cancelled
            }
            ConfirmChangeOutcome::AddressTaken => EmailConfirmResult::AddressTaken,
            ConfirmChangeOutcome::Invalid => EmailConfirmResult::InvalidLink,
        })
    }
}

fn refusal_label(refusal: AccountRefusal) -> &'static str {
    match refusal {
        AccountRefusal::Expired => "expired",
        AccountRefusal::ChallengeUsed => "used",
        AccountRefusal::AttemptsExhausted => "attempts_exhausted",
        AccountRefusal::WrongPurpose => "wrong_purpose",
        AccountRefusal::CredentialRevoked => "account_inactive",
        AccountRefusal::GenerationExhausted => "generation_exhausted",
        AccountRefusal::WrongScope
        | AccountRefusal::WrongChallenge
        | AccountRefusal::WrongIdentity
        | AccountRefusal::UnlinkedGuest
        | AccountRefusal::NoRecoveryChannel
        | AccountRefusal::UnverifiedProof
        | AccountRefusal::NotGuest
        | AccountRefusal::UnverifiedAccount
        | AccountRefusal::StaleRevision
        | AccountRefusal::RevisionExhausted
        | AccountRefusal::SelectionTooLarge
        | AccountRefusal::InvalidSelection
        | AccountRefusal::MembershipDenied
        | AccountRefusal::AlreadyLinked
        | AccountRefusal::Conflict
        | AccountRefusal::Retired => "refused",
    }
}
