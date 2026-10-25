//! The account's own pages: profile, security history, data export and closure
//! (commerce-service "Account and tenant lifecycle"; plan 7.5).

use serde_json::{Value, json};

use crate::{
    access::Authenticated,
    closure::{CloseOutcome, ClosureCheck, ScheduledDeletion},
    ports::{AccountStore, EmailSender},
    records::{
        AccountEvent, AccountRecord, AccountsSummary, DisplayName, DisplayNameRefusal, EventKind,
        SessionSummary,
    },
    retention::idle_before_ms,
    service::{Client, CustomerService, MailJob, ServiceError, account_label, audit, read, write},
    throttle::{Subject, ThrottleScope},
};

/// A sign-in at most this old counts as recent authentication for an export.
pub const RECENT_SIGN_IN_MS: u64 = 10 * 60 * 1000;
/// Closing an account needs a fresher proof: the password typed again, or a sign-in (the
/// provider's, redone) no older than this.
pub const CLOSURE_REAUTHENTICATION_MS: u64 = 5 * 60 * 1000;

/// How a person re-proves who they are before a sensitive action (export, closure).
#[derive(Clone, Copy, Debug)]
pub enum Reauthentication<'a> {
    /// The current password, typed again (checked locally, or by the identity provider).
    Password(&'a str),
    /// A sign-in no older than the action's window: `RECENT_SIGN_IN_MS` for an export,
    /// `CLOSURE_REAUTHENTICATION_MS` for closing an account.
    RecentSignIn,
}

/// Security events shown on the security page.
pub const SHOWN_EVENTS: u32 = 20;
/// The export never exceeds this many bytes (plan 7.5).
pub const MAXIMUM_EXPORT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProfileResult {
    Saved,
    Refused(DisplayNameRefusal),
    Throttled,
}

pub enum ExportResult {
    /// The JSON document, ready to send as an attachment.
    Document(String),
    /// The password was wrong, or the sign-in is not recent enough.
    NotReauthenticated,
    Throttled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloseResult {
    /// Closed; logging in before `delete_after_ms` reopens it.
    Closed {
        delete_after_ms: u64,
    },
    /// Not closed, nothing changed: the account owns or hosts these active campaigns (transfer
    /// or archive them), and owns these archived ones (transfer or delete them, or let their
    /// deletion be scheduled).
    Blocked(ClosureCheck),
    /// The password was wrong, or the sign-in is not recent enough.
    NotReauthenticated,
    Throttled,
}

/// What the security page shows.
pub struct SecurityOverview {
    pub events: Vec<AccountEvent>,
    pub sessions: Vec<SessionSummary>,
}

fn event_json(event: &AccountEvent) -> Value {
    json!({
        "event": event.kind.label(),
        "at_ms": event.at_ms,
        "device": event.device.label(),
    })
}

impl<S: AccountStore, M: EmailSender> CustomerService<S, M> {
    /// A password proof checks the password now. A sign-in proof must be at most `window_ms` old.
    async fn is_reauthenticated(
        &self,
        caller: &Authenticated,
        proof: Reauthentication<'_>,
        window_ms: u64,
    ) -> Result<bool, ServiceError> {
        match proof {
            Reauthentication::Password(password) => self.password_matches(caller, password).await,
            Reauthentication::RecentSignIn => {
                Ok(self.now_ms().saturating_sub(caller.signed_in_at_ms) <= window_ms)
            }
        }
    }

    pub async fn update_profile(
        &self,
        caller: &Authenticated,
        display_name: &str,
    ) -> Result<ProfileResult, ServiceError> {
        if !self.admit(
            ThrottleScope::ProfilePerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(ProfileResult::Throttled);
        }
        let name = match DisplayName::parse(display_name) {
            Ok(name) => name,
            Err(refusal) => return Ok(ProfileResult::Refused(refusal)),
        };
        write(self.shared.store.update_profile(caller.account.id, name)).await?;
        Ok(ProfileResult::Saved)
    }

    pub async fn security_overview(
        &self,
        caller: &Authenticated,
    ) -> Result<SecurityOverview, ServiceError> {
        let now_ms = self.now_ms();
        let idle_before_ms = idle_before_ms(now_ms, self.shared.tunables.sessions.idle);
        Ok(SecurityOverview {
            events: read(self.shared.store.events(caller.account.id, SHOWN_EVENTS)).await?,
            sessions: read(
                self.shared
                    .store
                    .sessions(caller.account.id, now_ms, idle_before_ms),
            )
            .await?,
        })
    }

    /// The person's own account data. Never game content, other people's data, hashes or tokens.
    pub async fn export(
        &self,
        caller: &Authenticated,
        proof: Reauthentication<'_>,
        client: Client,
    ) -> Result<ExportResult, ServiceError> {
        if !self.admit(
            ThrottleScope::DataPerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(ExportResult::Throttled);
        }
        if !self
            .is_reauthenticated(caller, proof, RECENT_SIGN_IN_MS)
            .await?
        {
            return Ok(ExportResult::NotReauthenticated);
        }
        let overview = self.security_overview(caller).await?;
        let account = &caller.account;
        let document = json!({
            "schema": "dungeonflux.account-export/1",
            "account": {
                "id": account_label(account.id),
                "email": account.email.as_ref().map(|email| email.display()),
                "email_verified": account.is_verified,
                "display_name": account.display_name.as_ref().map(DisplayName::as_str),
                "status": account.status.label(),
                "created_at_ms": account.created_at_ms,
            },
            "sessions": overview.sessions.iter().map(|session| json!({
                "device": session.device.label(),
                "created_at_ms": session.created_at_ms,
                "last_seen_at_ms": session.last_seen_at_ms,
            })).collect::<Vec<_>>(),
            "security_events": overview.events.iter().map(event_json).collect::<Vec<_>>(),
        });
        let text =
            serde_json::to_string_pretty(&document).map_err(|_| ServiceError::Unavailable)?;
        if text.len() > MAXIMUM_EXPORT_BYTES {
            return Err(ServiceError::Unavailable);
        }
        let event = AccountEvent {
            kind: EventKind::DataExported,
            at_ms: self.now_ms(),
            device: client.device,
        };
        write(self.shared.store.record_event(account.id, event)).await?;
        audit(
            "account.export",
            format!(
                "ACCOUNT export account={} bytes={}",
                account_label(account.id),
                text.len()
            ),
        );
        Ok(ExportResult::Document(text))
    }

    pub async fn close(
        &self,
        caller: &Authenticated,
        proof: Reauthentication<'_>,
    ) -> Result<CloseResult, ServiceError> {
        if !self.admit(
            ThrottleScope::DataPerAccount,
            Subject::Account(caller.account.id),
        ) {
            return Ok(CloseResult::Throttled);
        }
        if !self
            .is_reauthenticated(caller, proof, CLOSURE_REAUTHENTICATION_MS)
            .await?
        {
            return Ok(CloseResult::NotReauthenticated);
        }
        let now_ms = self.now_ms();
        let window_ms =
            u64::try_from(self.shared.tunables.closure_window.as_millis()).unwrap_or(u64::MAX);
        let delete_after_ms = now_ms.saturating_add(window_ms);
        let outcome = write(self.shared.store.close_account(
            caller.account.id,
            now_ms,
            delete_after_ms,
        ))
        .await?;
        match outcome {
            CloseOutcome::Blocked(check) => {
                audit(
                    "account.closure_blocked",
                    format!(
                        "ACCOUNT closure_blocked account={} blocking={} owned_archived={}",
                        account_label(caller.account.id),
                        check.blocking.len(),
                        check.owned_archived.len()
                    ),
                );
                Ok(CloseResult::Blocked(check))
            }
            CloseOutcome::AlreadyClosing { delete_after_ms } => {
                Ok(CloseResult::Closed { delete_after_ms })
            }
            CloseOutcome::Closed {
                delete_after_ms,
                scheduled,
            } => {
                audit(
                    "account.closing",
                    format!(
                        "ACCOUNT closing account={} delete_after_ms={delete_after_ms}                          scheduled_deletions={}",
                        account_label(caller.account.id),
                        scheduled.len()
                    ),
                );
                self.notify_scheduled_deletions(&scheduled, delete_after_ms)
                    .await;
                Ok(CloseResult::Closed { delete_after_ms })
            }
        }
    }

    /// Tells every member of each campaign scheduled for deletion. Best effort after the commit:
    /// a member without a deliverable address or a full mail queue is skipped and audited.
    async fn notify_scheduled_deletions(&self, scheduled: &[ScheduledDeletion], at_ms: u64) {
        let date = crate::email::utc_date(at_ms);
        for deletion in scheduled {
            for member in &deletion.members {
                let email = read(self.shared.store.account(*member))
                    .await
                    .ok()
                    .flatten()
                    .and_then(|account| account.email);
                let sent = match (email, self.reserve_mail()) {
                    (Some(email), Ok(permit)) => {
                        permit.send(MailJob::Deliver(
                            self.shared.messages.campaign_deletion_scheduled(
                                email.display(),
                                &deletion.name,
                                &date,
                            ),
                        ));
                        true
                    }
                    _ => false,
                };
                if !sent {
                    audit(
                        "email.discarded",
                        "EMAIL discarded purpose=campaign_deletion_scheduled reason=no_delivery"
                            .to_owned(),
                    );
                }
            }
        }
    }

    /// Exact-match lookup for the operator console: a full address or a 32-character account id,
    /// never a prefix or a pattern. Whether the caller may look is the calling layer's check
    /// (the operator capability); this only finds. A closed account has no address to match.
    pub async fn find_account(&self, query: &str) -> Result<Option<AccountRecord>, ServiceError> {
        let query = query.trim();
        if let Ok(id) = df_types::AccountId::from_hex(query) {
            return read(self.shared.store.account(id)).await;
        }
        match crate::records::EmailAddress::parse(query) {
            Ok(email) => read(self.shared.store.account_by_email(email.normalized())).await,
            Err(_) => Ok(None),
        }
    }

    /// Counts and recent accounts for `dev accounts`.
    pub async fn dev_summary(&self) -> Result<AccountsSummary, ServiceError> {
        read(self.shared.store.summary(20)).await
    }
}
