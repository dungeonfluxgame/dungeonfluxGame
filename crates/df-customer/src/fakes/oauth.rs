//! OAuth fake uses the same lock as account mutations, preserving transactional identity checks.

use super::*;
use crate::{
    GoogleResolution, GoogleResolved, GoogleStore, GoogleStoreCall, OAuthAttempt, OAuthIntent,
    OAuthRefusal,
};

impl MemoryAccountStore {
    pub fn google_consent(&self, account: AccountId) -> Option<(u64, String, u64)> {
        self.state().google_consents.get(&account).cloned()
    }
}

impl GoogleStore for MemoryAccountStore {
    fn save_attempt(&self, attempt: OAuthAttempt) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            let mut state = self.state();
            state
                .oauth_attempts
                .retain(|_, row| row.expires_at_ms > attempt.created_at_ms);
            if state.oauth_attempts.len() >= 10_000
                || state
                    .oauth_attempts
                    .values()
                    .filter(|row| row.client_digest == attempt.client_digest)
                    .count()
                    >= 8
            {
                return Ok(false);
            }
            state.oauth_attempts.insert(attempt.state_digest, attempt);
            Ok(true)
        })
    }

    fn claim_attempt(
        &self,
        digest: TokenDigest,
        cookie: TokenDigest,
        now_ms: u64,
    ) -> GoogleStoreCall<'_, Option<OAuthAttempt>> {
        Box::pin(async move {
            let mut state = self.state();
            if !state
                .oauth_attempts
                .get(&digest)
                .is_some_and(|row| row.cookie_digest == cookie && row.expires_at_ms > now_ms)
            {
                return Ok(None);
            }
            Ok(state.oauth_attempts.remove(&digest))
        })
    }

    fn resolve(&self, request: GoogleResolution) -> GoogleStoreCall<'_, GoogleResolved> {
        Box::pin(async move {
            let tenant = random_tenant()?;
            let mut state = self.state();
            if let Some(origin) = request.originating_session
                && !state.sessions.get(&origin).is_some_and(|row| {
                    !row.record.is_revoked
                        && Some(row.record.account) == request.account
                        && Some(row.record.credential_generation) == request.credential_generation
                        && row.absolute_expires_at_ms > request.now_ms
                        && row.record.last_seen_at_ms >= request.idle_before_ms
                })
            {
                return Ok(GoogleResolved::Refused(
                    OAuthRefusal::ReauthenticationRequired,
                ));
            }
            let linked = state.google_links.get(&request.subject).copied();
            if let Some(account) = linked {
                if request.intent.requires_fresh() && request.account != Some(account) {
                    return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
                }
                let account = state
                    .accounts
                    .get(&account)
                    .ok_or(crate::StoreError::InvalidRecord)?;
                let may_reopen = !request.intent.requires_fresh()
                    && account.status == AccountStatus::Closing
                    && account
                        .delete_after_ms
                        .is_some_and(|end| request.now_ms < end);
                if (account.status != AccountStatus::Active && !may_reopen) || !account.is_verified
                {
                    return Ok(GoogleResolved::Refused(OAuthRefusal::AccountUnavailable));
                }
                if account.email.as_ref().map(EmailAddress::normalized)
                    != Some(request.email.normalized())
                {
                    return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
                }
                if request.intent.requires_fresh()
                    && request.credential_generation != Some(account.credential_generation)
                {
                    return Ok(GoogleResolved::Refused(
                        OAuthRefusal::ReauthenticationRequired,
                    ));
                }
                return Ok(GoogleResolved::Account(account.clone()));
            }
            match request.intent {
                OAuthIntent::SignIn | OAuthIntent::Reauthenticate => {
                    Ok(GoogleResolved::Refused(OAuthRefusal::NoAccount))
                }
                OAuthIntent::Link => {
                    let Some(id) = request.account else {
                        return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
                    };
                    let Some(account) = state.accounts.get(&id).cloned() else {
                        return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
                    };
                    if account.status != AccountStatus::Active
                        || !account.is_verified
                        || Some(account.credential_generation) != request.credential_generation
                        || account.email.as_ref().map(EmailAddress::normalized)
                            != Some(request.email.normalized())
                        || state.google_links.values().any(|linked| *linked == id)
                    {
                        return Ok(GoogleResolved::Refused(OAuthRefusal::Conflict));
                    }
                    state.google_links.insert(request.subject, id);
                    Ok(GoogleResolved::Account(account))
                }
                OAuthIntent::SignUp => {
                    if state.by_email.contains_key(request.email.normalized()) {
                        return Ok(GoogleResolved::Refused(
                            OAuthRefusal::PasswordSignInRequired,
                        ));
                    }
                    let terms = request
                        .terms_version
                        .ok_or(crate::StoreError::InvalidRecord)?;
                    state.google_consents.insert(
                        request.new_account,
                        (request.accepted_at_ms, terms, request.accepted_at_ms),
                    );
                    let account = AccountRecord {
                        id: request.new_account,
                        email: Some(request.email),
                        is_verified: true,
                        display_name: None,
                        status: AccountStatus::Active,
                        credential_generation: 1,
                        password_hash: None,
                        created_at_ms: request.now_ms,
                        delete_after_ms: None,
                    };
                    let email = account
                        .email
                        .as_ref()
                        .ok_or(crate::StoreError::InvalidRecord)?;
                    state
                        .by_email
                        .insert(email.normalized().to_owned(), account.id);
                    state.google_links.insert(request.subject, account.id);
                    state.accounts.insert(account.id, account.clone());
                    state.issue_tenant(account.id, tenant);
                    state.event(
                        account.id,
                        EventKind::SignedUp,
                        request.now_ms,
                        DeviceLabel::Unknown,
                    );
                    Ok(GoogleResolved::Created(account))
                }
            }
        })
    }

    fn is_linked(&self, account: AccountId) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            Ok(self
                .state()
                .google_links
                .values()
                .any(|linked| *linked == account))
        })
    }

    fn unlink(
        &self,
        account: AccountId,
        generation: u64,
    ) -> GoogleStoreCall<'_, Result<(), OAuthRefusal>> {
        Box::pin(async move {
            let mut state = self.state();
            let Some(row) = state.accounts.get(&account) else {
                return Ok(Err(OAuthRefusal::Conflict));
            };
            if row.status != AccountStatus::Active || row.credential_generation != generation {
                return Ok(Err(OAuthRefusal::Conflict));
            }
            if row.password_hash.is_none()
                && !state.identities.values().any(|linked| *linked == account)
            {
                return Ok(Err(OAuthRefusal::LastSignInMethod));
            }
            state.google_links.retain(|_, linked| *linked != account);
            Ok(Ok(()))
        })
    }

    fn create_google_session(&self, request: crate::GoogleSession) -> GoogleStoreCall<'_, bool> {
        Box::pin(async move {
            let session = request.session;
            let id = session.id;
            let account = session.account;
            let mut state = self.state();
            if state.google_links.get(&request.subject) != Some(&account) {
                return Ok(false);
            }
            if let Some(origin) = request.originating_session
                && !state.sessions.get(&origin).is_some_and(|row| {
                    !row.record.is_revoked
                        && row.record.account == session.account
                        && row.record.credential_generation == session.credential_generation
                        && row.absolute_expires_at_ms > session.created_at_ms
                        && row.record.last_seen_at_ms >= request.idle_before_ms
                })
            {
                return Ok(false);
            }
            if !state.accounts.get(&account).is_some_and(|row| {
                row.status == AccountStatus::Active
                    && row.credential_generation == session.credential_generation
            }) {
                return Ok(false);
            }
            state.insert_session(session);
            state
                .google_proofs
                .insert(id, request.reauthentication_expires_at_ms);
            state.events.entry(account).or_default().push(request.event);
            Ok(true)
        })
    }

    fn session_proof(&self, session: WebSessionId) -> GoogleStoreCall<'_, Option<Option<u64>>> {
        Box::pin(async move { Ok(self.state().google_proofs.get(&session).copied()) })
    }
}
