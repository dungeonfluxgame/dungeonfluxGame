//! The mail worker: the only caller of `EmailSender`. One owned task, fed by the bounded queue.
//! Reset requests are resolved here so the web request never learns whether the address exists.
//! With a provider, resolving a reset calls it, which can be slow: those run in a bounded set
//! of tasks the worker owns (dropped, and so aborted, with the worker), so one slow provider call
//! does not hold up the verification emails behind it.

use std::{sync::Arc, time::Instant};

use tokio::{sync::mpsc, task::JoinSet};

use crate::{
    email::Message,
    ports::{AccountStore, EmailError, EmailSender},
    provider::{ErasedProvider, ResetStart},
    provider_flows::{is_plausible_provider_token, provider_token_digest},
    records::{AccountStatus, EmailAddress, NewToken, TokenPurpose},
    service::{
        LOGIN_EVENT_JOB_BUDGET, LoginRefusalJob, MAIL_DEADLINE, MailJob, PROVIDER_DEADLINE,
        STORE_DEADLINE, Shared, account_label, audit,
    },
    tokens::{SecretToken, new_record_id},
};

/// A reset link's lifetime (`service-operations.md` challenge-token class).
pub const RESET_TOKEN_LIFETIME_MS: u64 = 15 * 60 * 1000;
/// Submissions allowed per reset link.
pub const RESET_ATTEMPT_LIMIT: u32 = 5;
/// Reset requests being resolved at once; a full set holds the queue until one finishes.
pub const MAXIMUM_CONCURRENT_RESETS: usize = 8;
/// Refusal event jobs run away from the request path and remain bounded.
pub const MAXIMUM_CONCURRENT_LOGIN_EVENTS: usize = 8;

pub(crate) async fn run<S: AccountStore, M: EmailSender>(
    shared: Arc<Shared<S, M>>,
    mut jobs: mpsc::Receiver<MailJob>,
) {
    let mut resets: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            job = jobs.recv() => match job {
                None => break,
                Some(MailJob::Deliver(message)) => deliver(&shared, message).await,
                Some(MailJob::PasswordReset { normalized_email }) => {
                    if resets.len() >= MAXIMUM_CONCURRENT_RESETS {
                        report(resets.join_next().await);
                    }
                    let shared = Arc::clone(&shared);
                    resets.spawn(async move {
                        if let Some(message) = reset_message(&shared, &normalized_email).await {
                            deliver(&shared, message).await;
                        }
                    });
                }
            },
            Some(finished) = resets.join_next(), if !resets.is_empty() => report(Some(finished)),
        }
    }
    resets.shutdown().await;
}

pub(crate) async fn run_login_events<S: AccountStore, M: EmailSender>(
    shared: Arc<Shared<S, M>>,
    mut jobs: mpsc::Receiver<LoginRefusalJob>,
) {
    let mut workers: JoinSet<()> = JoinSet::new();
    loop {
        tokio::select! {
            job = jobs.recv() => match job {
                None => break,
                Some(job) => {
                    if workers.len() >= MAXIMUM_CONCURRENT_LOGIN_EVENTS {
                        report_login_event(workers.join_next().await);
                    }
                    let shared = Arc::clone(&shared);
                    workers.spawn(async move {
                        record_login_refusal(&shared, job).await;
                    });
                }
            },
            Some(finished) = workers.join_next(), if !workers.is_empty() => report_login_event(Some(finished)),
        }
    }
    workers.shutdown().await;
}

fn report_login_event(finished: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(error)) = finished
        && error.is_panic()
    {
        audit(
            "account.login_event_worker_failed",
            "ACCOUNT login_event_worker_failed".to_owned(),
        );
    }
}

async fn record_login_refusal<S: AccountStore, M>(shared: &Shared<S, M>, job: LoginRefusalJob) {
    let deadline = tokio::time::Instant::now() + LOGIN_EVENT_JOB_BUDGET;
    let work = async {
        let found = tokio::time::timeout_at(
            deadline,
            shared.store.account_by_email(&job.normalized_email),
        )
        .await;
        let account = match found {
            Ok(Ok(Some(account))) => account,
            Ok(Ok(None)) => return,
            Ok(Err(_)) | Err(_) => {
                audit(
                    "account.login_event_unavailable",
                    "ACCOUNT login_event_unavailable operation=lookup".to_owned(),
                );
                return;
            }
        };
        if !matches!(
            tokio::time::timeout_at(deadline, shared.store.record_event(account.id, job.event),)
                .await,
            Ok(Ok(()))
        ) {
            audit(
                "account.login_event_unavailable",
                format!(
                    "ACCOUNT login_event_unavailable account={}",
                    account_label(account.id)
                ),
            );
        }
    };
    tokio::select! {
        _ = work => {},
        _ = tokio::time::sleep_until(deadline) => {},
    }
    tokio::time::sleep_until(deadline).await;
}

/// A reset task that panicked is audited; it never stops the worker.
fn report(finished: Option<Result<(), tokio::task::JoinError>>) {
    if let Some(Err(error)) = finished
        && error.is_panic()
    {
        audit(
            "email.worker_task_failed",
            "EMAIL worker_task_failed purpose=password_reset".to_owned(),
        );
    }
}

async fn deliver<S, M: EmailSender>(shared: &Shared<S, M>, message: Message) {
    let purpose = message.purpose.label();
    let started = Instant::now();
    let outcome = match tokio::time::timeout(MAIL_DEADLINE, shared.mail.send(message)).await {
        Ok(Ok(())) => "written",
        Ok(Err(EmailError::Failed)) => "failed",
        Ok(Err(EmailError::Unknown)) | Err(_) => "unknown",
    };
    audit(
        "email.delivery",
        format!(
            "EMAIL {outcome} purpose={purpose} ms={}",
            started.elapsed().as_millis()
        ),
    );
}

fn discarded(reason: &str) {
    audit(
        "email.discarded",
        format!("EMAIL discarded purpose=password_reset reason={reason}"),
    );
}

/// Issues a reset token for an active account at this address, or `None` (the job is discarded:
/// unknown address, closed account, or a store failure, each recorded once).
async fn reset_message<S: AccountStore, M>(
    shared: &Shared<S, M>,
    normalized_email: &str,
) -> Option<Message> {
    let found = tokio::time::timeout(
        STORE_DEADLINE,
        shared.store.account_by_email(normalized_email),
    )
    .await;
    let account = match found {
        Ok(Ok(Some(account))) if account.status == AccountStatus::Active => account,
        Ok(Ok(_)) => {
            discarded("no_active_account");
            return None;
        }
        Ok(Err(_)) | Err(_) => {
            discarded("store_unavailable");
            return None;
        }
    };
    // An active account always has an address; only closed accounts lose it.
    let email = account.email.clone()?;
    let (token_text, message_for) = match shared.identity.erased() {
        Some(provider) => {
            let token = provider_reset_token(shared, provider.as_ref(), account.id, &email).await?;
            (token, None)
        }
        None => {
            let Ok(token) = SecretToken::generate() else {
                discarded("random_unavailable");
                return None;
            };
            (token.text(), Some(token))
        }
    };
    let digest = match &message_for {
        Some(token) => token.digest(),
        None => provider_token_digest(&token_text),
    };
    let Ok(id) = new_record_id() else {
        discarded("random_unavailable");
        return None;
    };
    let now_ms = shared.clock.now_ms();
    let issued = tokio::time::timeout(
        STORE_DEADLINE,
        shared.store.issue_token(NewToken {
            id,
            digest,
            account: account.id,
            purpose: TokenPurpose::PasswordReset,
            email_change: None,
            credential_generation: account.credential_generation,
            created_at_ms: now_ms,
            expires_at_ms: now_ms + RESET_TOKEN_LIFETIME_MS,
            attempt_limit: RESET_ATTEMPT_LIMIT,
        }),
    )
    .await;
    if !matches!(issued, Ok(Ok(()))) {
        discarded("token_not_stored");
        return None;
    }
    audit(
        "account.reset_requested",
        format!(
            "ACCOUNT reset_requested account={}",
            account_label(account.id)
        ),
    );
    Some(shared.messages.password_reset(email.display(), &token_text))
}

/// The provider issues the reset token (it keeps the password); we store its digest for the
/// link's 15 minutes and five attempts, and our mail carries the token. An account with no link
/// to this provider, a provider that does not know the address, a malformed token or a failure
/// sends nothing, each recorded once.
async fn provider_reset_token<S: AccountStore, M>(
    shared: &Shared<S, M>,
    provider: &dyn ErasedProvider,
    account: df_types::AccountId,
    email: &EmailAddress,
) -> Option<String> {
    let linked = tokio::time::timeout(
        STORE_DEADLINE,
        shared.store.identity_for_account(account, provider.kind()),
    )
    .await;
    if !matches!(linked, Ok(Ok(Some(_)))) {
        discarded("no_linked_identity");
        return None;
    }
    let started =
        tokio::time::timeout(PROVIDER_DEADLINE, provider.begin_password_reset(email)).await;
    match started {
        Ok(Ok(ResetStart::Started { token })) if is_plausible_provider_token(&token) => Some(token),
        Ok(Ok(ResetStart::Started { .. })) => {
            discarded("provider_token_malformed");
            None
        }
        Ok(Ok(ResetStart::NoSuchUser)) => {
            discarded("provider_has_no_user");
            None
        }
        Ok(Err(_)) | Err(_) => {
            discarded("provider_unavailable");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::{AccountEvent, EventKind, fakes::MemoryAccountStore, records::DeviceLabel};

    use super::{MAXIMUM_CONCURRENT_LOGIN_EVENTS, record_login_refusal};
    use crate::service::{LOGIN_EVENT_QUEUE_CAPACITY, LoginRefusalJob};

    fn refusal_event() -> AccountEvent {
        AccountEvent {
            kind: EventKind::LoginRefused,
            at_ms: 1,
            device: DeviceLabel::Unknown,
        }
    }

    #[tokio::test]
    async fn refusal_flood_does_not_make_mail_admission_busy_and_counts_drops() {
        let store = MemoryAccountStore::default();
        store.set_event_write_delay(Duration::from_millis(1_200));
        let harness = crate::contract::Harness::start(store).await;

        for _ in 0..(LOGIN_EVENT_QUEUE_CAPACITY + MAXIMUM_CONCURRENT_LOGIN_EVENTS * 4) {
            harness
                .service
                .queue_login_refusal("known@example.test", refusal_event());
        }

        let permit = harness
            .service
            .reserve_mail()
            .expect("mail capacity is independent");
        drop(permit);
        assert!(harness.service.login_refusal_queue_dropped() > 0);
    }

    #[tokio::test]
    async fn known_and_unknown_refusal_jobs_take_the_same_bounded_time() {
        let store = MemoryAccountStore::default();
        let harness = crate::contract::Harness::start(store.clone()).await;
        harness.sign_up("known@example.test").await;
        store.set_event_write_delay(Duration::from_millis(1_200));
        let event = refusal_event();
        let known_started = Instant::now();
        record_login_refusal(
            &harness.service.shared,
            LoginRefusalJob {
                normalized_email: "known@example.test".to_owned(),
                event,
            },
        )
        .await;
        let known_elapsed = known_started.elapsed();
        let unknown_started = Instant::now();
        record_login_refusal(
            &harness.service.shared,
            LoginRefusalJob {
                normalized_email: "unknown@example.test".to_owned(),
                event,
            },
        )
        .await;
        let unknown_elapsed = unknown_started.elapsed();

        assert!(
            known_elapsed.abs_diff(unknown_elapsed) <= Duration::from_millis(100),
            "known={known_elapsed:?}, unknown={unknown_elapsed:?}"
        );
    }
}
