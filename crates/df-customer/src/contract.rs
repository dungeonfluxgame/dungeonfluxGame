//! The store contract: scenarios every `AccountStore` must pass, driven through the real service
//! with the fake clock and the recording mail sender. `store_contract_tests!` turns them into
//! tests for one store; this crate runs them on the memory store, `df-persistence` on PostgreSQL.

use std::{future::Future, net::IpAddr, sync::Arc, time::Duration};

use df_types::OperationId;

use crate::{
    AccountStore, Authenticated, BlockReason, BuiltInCommonPasswords, Client, CustomerConfig,
    CustomerService, DeviceLabel, EmailChangeDecision, EmailChangeResult, EmailConfirmResult,
    EventKind, ExportResult, HashingParameters, HostConfig, IdentitySource, LoginOutcome,
    MailPurpose, PasswordChangeResult, ResetConfirmOutcome, RevokeScope, ServiceTasks, SignupForm,
    SignupOutcome, Tunables, VerifyWithPasswordOutcome,
    clock::FakeClock,
    fakes::{CampaignSeed, CampaignSeeding, FakeIdentityProvider, RecordingMail},
    lifecycle::CloseResult,
};

/// A store under test and how to dispose of it.
pub trait Fixture: Sized {
    type Store: AccountStore + Clone;
    fn store(&self) -> Self::Store;
    fn finish(self) -> impl Future<Output = ()>;
}

/// A start time well inside the operation-id lifetime.
pub const START_MS: u64 = 1_800_000_000_000;
pub const PASSWORD: &str = "lantern under the old mill";
pub const NEW_PASSWORD: &str = "the miller keeps a second key";
const MINUTE_MS: u64 = 60_000;

/// The service with fakes around one store.
pub struct Harness<S> {
    pub service: CustomerService<S, RecordingMail>,
    pub mail: RecordingMail,
    pub clock: FakeClock,
    _tasks: ServiceTasks,
}

pub fn client(last_octet: u8) -> Client {
    Client {
        ip: Some(IpAddr::from([192, 0, 2, last_octet])),
        device: DeviceLabel::Desktop,
    }
}

impl<S: AccountStore + Clone> Harness<S> {
    pub async fn start(store: S) -> Self {
        let hosts = HostConfig::local("http://127.0.0.1:8099").expect("hosts");
        Self::start_with(store, IdentitySource::local_passwords(), hosts).await
    }

    pub async fn start_with(store: S, identity: IdentitySource, hosts: HostConfig) -> Self {
        Self::start_configured(store, identity, hosts, Duration::ZERO)
            .await
            .expect("service starts")
    }

    /// Like `start_with`, with the configured auth timing floor and the start error visible.
    pub async fn start_configured(
        store: S,
        identity: IdentitySource,
        hosts: HostConfig,
        provider_signup_floor: Duration,
    ) -> Result<Self, crate::StartError> {
        let clock = FakeClock::at(START_MS);
        let mail = RecordingMail::default();
        let mut tunables = Tunables::default();
        tunables.sign_in_failure_floor = Duration::ZERO;
        let (service, tasks) = CustomerService::start(
            CustomerConfig {
                hosts,
                identity,
                tunables,
                hashing: HashingParameters::TEST,
                maximum_concurrent_hashes: 4,
                hash_permit_wait: Duration::from_secs(5),
                provider_signup_floor,
                dev_allow_local_passwords_on_https: false,
            },
            store,
            mail.clone(),
            Arc::new(clock.clone()),
            Arc::new(BuiltInCommonPasswords),
        )
        .await?;
        Ok(Self {
            service,
            mail,
            clock,
            _tasks: tasks,
        })
    }

    pub fn operation(&self) -> OperationId {
        self.service.new_operation().expect("operation id")
    }

    pub async fn sign_up(&self, email: &str) -> SignupOutcome {
        self.service
            .signup(
                SignupForm {
                    email,
                    password: PASSWORD,
                    operation: self.operation(),
                    adult_attested: true,
                    terms_version: "terms-2026-10".to_owned(),
                },
                client(1),
            )
            .await
            .expect("signup runs")
    }

    /// Signs up, verifies through the emailed link and logs in.
    pub async fn verified_account(&self, email: &str) -> (Authenticated, String) {
        let sent = self.mail.messages().len();
        assert_eq!(self.sign_up(email).await, SignupOutcome::CheckYourEmail);
        self.mail.wait_for(sent + 1).await;
        let token = self
            .mail
            .last_token(email, MailPurpose::VerifyEmail)
            .expect("verification link");
        assert_eq!(
            self.service
                .verify_email_and_set_password(&token, PASSWORD, PASSWORD, client(1))
                .await
                .unwrap(),
            VerifyWithPasswordOutcome::Verified
        );
        self.log_in(email, PASSWORD).await
    }

    pub async fn log_in(&self, email: &str, password: &str) -> (Authenticated, String) {
        let LoginOutcome::SignedIn(issued) = self
            .service
            .login(email, password, client(2))
            .await
            .expect("login runs")
        else {
            panic!("login refused for {email}");
        };
        let text = issued.token.text();
        let caller = self
            .service
            .authenticate(&text)
            .await
            .unwrap()
            .expect("fresh session authenticates");
        (caller, text)
    }

    /// Verifies through the newest emailed link, then logs in.
    pub async fn verify_and_log_in(&self, email: &str) -> (Authenticated, String) {
        self.mail.wait_for(1).await;
        let token = self
            .mail
            .last_token(email, MailPurpose::VerifyEmail)
            .expect("verification link");
        assert_eq!(
            self.service
                .verify_email_and_set_password(&token, PASSWORD, PASSWORD, client(1))
                .await
                .unwrap(),
            VerifyWithPasswordOutcome::Verified
        );
        self.log_in(email, PASSWORD).await
    }

    pub async fn is_signed_in(&self, cookie: &str) -> bool {
        self.service.authenticate(cookie).await.unwrap().is_some()
    }
}

pub async fn signup_verify_login_and_logout_work_end_to_end<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    assert!(caller.account.is_verified);
    harness.service.logout(&caller, client(2)).await.unwrap();
    assert!(!harness.is_signed_in(&cookie).await);
}

pub async fn a_double_submitted_signup_creates_one_account_and_one_message<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let operation = harness.operation();
    for _ in 0..2 {
        let outcome = harness
            .service
            .signup(
                SignupForm {
                    email: "robin@example.test",
                    password: PASSWORD,
                    operation,
                    adult_attested: true,
                    terms_version: "terms-2026-10".to_owned(),
                },
                client(1),
            )
            .await
            .unwrap();
        assert_eq!(outcome, SignupOutcome::CheckYourEmail);
    }
    harness.mail.wait_for(1).await;
    let summary = harness.service.dev_summary().await.unwrap();
    assert_eq!(summary.active, 1);
    assert_eq!(harness.mail.messages().len(), 1);
    assert_eq!(harness.mail.messages()[0].purpose, MailPurpose::VerifyEmail);
}

pub async fn signing_up_with_a_registered_address_looks_the_same_and_warns_its_owner<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let first = harness.sign_up("robin@example.test").await;
    let second = harness.sign_up("Robin@Example.test").await;
    assert_eq!(first, second);
    harness.mail.wait_for(2).await;
    let purposes: Vec<MailPurpose> = harness
        .mail
        .messages()
        .iter()
        .map(|message| message.purpose)
        .collect();
    assert_eq!(
        purposes,
        [MailPurpose::VerifyEmail, MailPurpose::SignupAttempt]
    );
    assert_eq!(harness.service.dev_summary().await.unwrap().active, 1);
}

pub async fn a_password_change_signs_out_other_devices_and_rotates_this_one<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    let (_other, other_cookie) = harness.log_in("robin@example.test", PASSWORD).await;
    let PasswordChangeResult::Changed(issued) = harness
        .service
        .change_password(&caller, PASSWORD, NEW_PASSWORD, NEW_PASSWORD, client(2))
        .await
        .unwrap()
    else {
        panic!("password change refused");
    };
    assert!(!harness.is_signed_in(&cookie).await);
    assert!(!harness.is_signed_in(&other_cookie).await);
    assert!(harness.is_signed_in(&issued.token.text()).await);
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(3))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    harness.log_in("robin@example.test", NEW_PASSWORD).await;
}

pub async fn a_reset_link_works_once_within_fifteen_minutes_and_signs_out_everywhere<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (_caller, cookie) = harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    harness
        .service
        .request_password_reset("robin@example.test", client(4))
        .await
        .unwrap();
    harness.mail.wait_for(sent + 1).await;
    let token = harness
        .mail
        .last_token("robin@example.test", MailPurpose::PasswordReset)
        .expect("reset link");
    harness.clock.advance_ms(14 * MINUTE_MS);
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, NEW_PASSWORD, NEW_PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::Reset
    );
    assert!(!harness.is_signed_in(&cookie).await);
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, PASSWORD, PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::InvalidLink,
        "a used link is refused"
    );
    harness.log_in("robin@example.test", NEW_PASSWORD).await;

    let sent = harness.mail.messages().len();
    harness
        .service
        .request_password_reset("robin@example.test", client(5))
        .await
        .unwrap();
    harness.mail.wait_for(sent + 1).await;
    let late = harness
        .mail
        .last_token("robin@example.test", MailPurpose::PasswordReset)
        .unwrap();
    harness.clock.advance_ms(15 * MINUTE_MS);
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&late, PASSWORD, PASSWORD, client(5))
            .await
            .unwrap(),
        ResetConfirmOutcome::InvalidLink,
        "a link is refused at fifteen minutes"
    );
}

pub async fn a_reset_for_an_unknown_address_sends_nothing<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    harness
        .service
        .request_password_reset("nobody@example.test", client(6))
        .await
        .unwrap();
    // A known address afterwards proves the worker has drained the unknown one first.
    harness
        .service
        .request_password_reset("robin@example.test", client(6))
        .await
        .unwrap();
    harness.mail.wait_for(sent + 1).await;
    let messages = harness.mail.messages();
    assert_eq!(messages.len(), sent + 1);
    assert_eq!(messages[sent].to, "robin@example.test");
}

pub async fn an_email_change_needs_both_addresses_to_confirm<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    assert_eq!(
        harness
            .service
            .request_email_change(&caller, PASSWORD, "hood@example.test", harness.operation())
            .await
            .unwrap(),
        EmailChangeResult::ConfirmationSent
    );
    harness.mail.wait_for(sent + 2).await;
    let old = harness
        .mail
        .last_token("robin@example.test", MailPurpose::EmailChangeOld)
        .unwrap();
    let new = harness
        .mail
        .last_token("hood@example.test", MailPurpose::EmailChangeNew)
        .unwrap();
    assert_eq!(
        harness
            .service
            .confirm_email_change(&new, EmailChangeDecision::Confirm, client(7))
            .await
            .unwrap(),
        EmailConfirmResult::AwaitingOtherAddress
    );
    assert!(
        harness.is_signed_in(&cookie).await,
        "one confirmation changes nothing"
    );
    assert_eq!(
        harness
            .service
            .confirm_email_change(&old, EmailChangeDecision::Confirm, client(7))
            .await
            .unwrap(),
        EmailConfirmResult::Changed
    );
    assert!(!harness.is_signed_in(&cookie).await);
    harness.log_in("hood@example.test", PASSWORD).await;
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(8))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
}

pub async fn the_old_address_can_cancel_an_email_change<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    let (caller, _cookie) = harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    harness
        .service
        .request_email_change(&caller, PASSWORD, "hood@example.test", harness.operation())
        .await
        .unwrap();
    harness.mail.wait_for(sent + 2).await;
    let old = harness
        .mail
        .last_token("robin@example.test", MailPurpose::EmailChangeOld)
        .unwrap();
    let new = harness
        .mail
        .last_token("hood@example.test", MailPurpose::EmailChangeNew)
        .unwrap();
    assert_eq!(
        harness
            .service
            .confirm_email_change(&old, EmailChangeDecision::Cancel, client(9))
            .await
            .unwrap(),
        EmailConfirmResult::Cancelled
    );
    assert_eq!(
        harness
            .service
            .confirm_email_change(&new, EmailChangeDecision::Confirm, client(9))
            .await
            .unwrap(),
        EmailConfirmResult::InvalidLink
    );
    harness.log_in("robin@example.test", PASSWORD).await;
}

pub async fn signing_out_other_devices_keeps_this_one<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    let (_, other) = harness.log_in("robin@example.test", PASSWORD).await;
    let revoked = harness
        .service
        .revoke_sessions(&caller, RevokeScope::AllExcept(caller.session), client(2))
        .await
        .unwrap();
    assert_eq!(revoked, 1);
    assert!(harness.is_signed_in(&cookie).await);
    assert!(!harness.is_signed_in(&other).await);
    harness
        .service
        .revoke_sessions(&caller, RevokeScope::All, client(2))
        .await
        .unwrap();
    assert!(!harness.is_signed_in(&cookie).await);
}

pub async fn one_account_cannot_revoke_or_see_another_accounts_sessions<S: AccountStore + Clone>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (robin, _) = harness.verified_account("robin@example.test").await;
    let (marian, marian_cookie) = harness.verified_account("marian@example.test").await;
    let revoked = harness
        .service
        .revoke_sessions(&robin, RevokeScope::One(marian.session), client(2))
        .await
        .unwrap();
    assert_eq!(revoked, 0);
    assert!(harness.is_signed_in(&marian_cookie).await);
    let overview = harness.service.security_overview(&robin).await.unwrap();
    assert!(
        overview
            .sessions
            .iter()
            .all(|session| session.id != marian.session)
    );
}

pub async fn closing_an_account_signs_out_and_logging_in_within_the_window_reopens_it<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    let CloseResult::Closed { delete_after_ms } = harness
        .service
        .close(&caller, crate::Reauthentication::Password(PASSWORD))
        .await
        .unwrap()
    else {
        panic!("closure refused");
    };
    assert_eq!(delete_after_ms, START_MS + 30 * 24 * 60 * MINUTE_MS);
    assert!(!harness.is_signed_in(&cookie).await);
    let (reopened, _) = harness.log_in("robin@example.test", PASSWORD).await;
    assert_eq!(reopened.account.status, crate::AccountStatus::Active);
}

pub async fn retention_deletes_a_closed_accounts_personal_data_after_the_window<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    let (caller, _) = harness.verified_account("robin@example.test").await;
    harness
        .service
        .close(&caller, crate::Reauthentication::Password(PASSWORD))
        .await
        .unwrap();
    harness.clock.advance_ms(30 * 24 * 60 * MINUTE_MS);
    harness.service.run_retention_pass().await;
    let account = store.account(caller.account.id).await.unwrap().unwrap();
    assert!(account.email.is_none() && account.password_hash.is_none());
    assert_eq!(account.status, crate::AccountStatus::Closed);
    assert!(
        store
            .events(caller.account.id, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(10))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    assert_eq!(
        harness.sign_up("robin@example.test").await,
        SignupOutcome::CheckYourEmail,
        "the address can sign up again"
    );
}

pub async fn an_export_holds_the_account_and_its_security_history_and_no_secrets<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    let ExportResult::Document(document) = harness
        .service
        .export(
            &caller,
            crate::Reauthentication::Password(PASSWORD),
            client(2),
        )
        .await
        .unwrap()
    else {
        panic!("export refused");
    };
    assert!(document.contains("\"email\": \"robin@example.test\""));
    assert!(document.contains(EventKind::LoggedIn.label()));
    assert!(!document.contains(&cookie), "no session token");
    assert!(!document.contains("argon2"), "no password hash");
    assert!(!document.contains(PASSWORD));
    assert!(matches!(
        harness
            .service
            .export(
                &caller,
                crate::Reauthentication::Password("wrong password here"),
                client(2)
            )
            .await
            .unwrap(),
        ExportResult::NotReauthenticated
    ));
}

// ---- Identity provider mode (decision 28) ----

/// A harness whose credentials live at the fake provider; links point at the account app.
pub async fn provider_harness<S: AccountStore + Clone>(
    store: S,
) -> (Harness<S>, FakeIdentityProvider) {
    let provider = FakeIdentityProvider::default();
    let hosts = HostConfig::parse(
        r#"{"schema": "dungeonflux.hosts/1", "origin": "http://127.0.0.1:8099",
            "mail": {"from_name": "DungeonFlux", "from_address": "accounts@mail.example.test",
                     "reply_to": "support@example.test"}}"#,
    )
    .expect("hosts");
    let harness =
        Harness::start_with(store, IdentitySource::provider(provider.clone()), hosts).await;
    (harness, provider)
}

pub async fn provider_signup_verify_login_and_logout_work_end_to_end<S: AccountStore + Clone>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    let (caller, cookie) = harness.verified_account("robin@example.test").await;
    assert!(caller.account.is_verified);
    assert!(
        caller.account.password_hash.is_none(),
        "the provider holds the password"
    );
    assert!(provider.is_verified("robin@example.test"));
    assert_eq!(
        provider.open_sessions(),
        0,
        "no provider session outlives its sign-in"
    );
    let message = &harness.mail.messages()[0];
    assert!(message.body.contains("/app/verify-email#token="));
    assert_eq!(message.from, "DungeonFlux <accounts@mail.example.test>");
    assert_eq!(message.reply_to, "support@example.test");
    harness.service.logout(&caller, client(2)).await.unwrap();
    assert!(!harness.is_signed_in(&cookie).await);
}

pub async fn a_registered_address_signs_up_like_a_new_one_and_warns_its_owner_at_the_provider<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    harness.verified_account("robin@example.test").await;
    let before = harness.mail.messages().len();
    assert_eq!(
        harness.sign_up("Robin@Example.test").await,
        SignupOutcome::CheckYourEmail
    );
    harness.mail.wait_for(before + 1).await;
    let messages = harness.mail.messages();
    assert_eq!(messages[before].purpose, MailPurpose::SignupAttempt);
    assert_eq!(messages[before].to, "robin@example.test");
    assert_eq!(harness.service.dev_summary().await.unwrap().active, 1);
    assert!(provider.has_user("robin@example.test"));
}

pub async fn a_wrong_password_and_an_unknown_address_look_alike_at_the_provider<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, _provider) = provider_harness(store).await;
    harness.verified_account("robin@example.test").await;
    let wrong = harness
        .service
        .login("robin@example.test", "not the right words here", client(3))
        .await
        .unwrap();
    let unknown = harness
        .service
        .login("nobody@example.test", PASSWORD, client(3))
        .await
        .unwrap();
    assert!(matches!(wrong, LoginOutcome::Refused));
    assert!(matches!(unknown, LoginOutcome::Refused));
    let events = harness
        .service
        .security_overview(&harness.log_in("robin@example.test", PASSWORD).await.0)
        .await
        .unwrap()
        .events;
    assert!(
        events
            .iter()
            .any(|event| event.kind == EventKind::LoginRefused)
    );
}

pub async fn a_provider_that_requires_its_own_verification_is_a_refused_sign_in_like_any_other<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    provider.require_verified(true);
    assert_eq!(
        harness.sign_up("robin@example.test").await,
        SignupOutcome::CheckYourEmail
    );
    // The provider refuses an address it has not verified: the same refusal as a wrong password,
    // counted as a failure, with no session and nothing that says the account exists.
    let refused = harness
        .service
        .login("robin@example.test", PASSWORD, client(2))
        .await
        .unwrap();
    let unknown = harness
        .service
        .login("nobody@example.test", PASSWORD, client(2))
        .await
        .unwrap();
    assert!(matches!(refused, LoginOutcome::Refused));
    assert!(matches!(unknown, LoginOutcome::Refused));
    for _ in 0..5 {
        let attempt = harness
            .service
            .login("robin@example.test", PASSWORD, client(2))
            .await
            .unwrap();
        assert!(!matches!(attempt, LoginOutcome::SignedIn(_)));
    }
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(2))
            .await
            .unwrap(),
        LoginOutcome::Throttled
    ));
}

pub async fn a_provider_that_does_not_require_verification_signs_an_unverified_person_in<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    provider.require_verified(false);
    harness.sign_up("robin@example.test").await;
    let (caller, _) = harness.log_in("robin@example.test", PASSWORD).await;
    assert!(
        !caller.account.is_verified,
        "only our emailed link verifies an address"
    );
}

pub async fn a_provider_reset_works_once_and_signs_out_everywhere<S: AccountStore + Clone>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    let (_caller, cookie) = harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    harness
        .service
        .request_password_reset("robin@example.test", client(4))
        .await
        .unwrap();
    harness.mail.wait_for(sent + 1).await;
    let message = harness.mail.messages().pop().unwrap();
    assert_eq!(message.purpose, MailPurpose::PasswordReset);
    assert!(message.body.contains("/app/reset-password#token="));
    let token = harness
        .mail
        .last_token("robin@example.test", MailPurpose::PasswordReset)
        .unwrap();
    // The provider's own policy refuses this password. The link is spent either way (sessions
    // are revoked before the provider is asked), so the person asks for a new one.
    provider.reject_password("a password the provider dislikes");
    assert_eq!(
        harness
            .service
            .confirm_password_reset(
                &token,
                "a password the provider dislikes",
                "a password the provider dislikes",
                client(4)
            )
            .await
            .unwrap(),
        ResetConfirmOutcome::ProviderRejectedPassword
    );
    assert!(
        !harness.is_signed_in(&cookie).await,
        "the sessions were revoked before the provider was asked"
    );
    let sent = harness.mail.messages().len();
    harness
        .service
        .request_password_reset("robin@example.test", client(4))
        .await
        .unwrap();
    harness.mail.wait_for(sent + 1).await;
    let token = harness
        .mail
        .last_token("robin@example.test", MailPurpose::PasswordReset)
        .unwrap();
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, NEW_PASSWORD, NEW_PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::Reset
    );
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, NEW_PASSWORD, NEW_PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::InvalidLink,
        "a reset token works once"
    );
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(2))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    harness.log_in("robin@example.test", NEW_PASSWORD).await;
}

pub async fn a_provider_reset_for_an_unknown_address_sends_nothing<S: AccountStore + Clone>(
    store: S,
) {
    let (harness, _provider) = provider_harness(store).await;
    harness
        .service
        .request_password_reset("nobody@example.test", client(4))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(harness.mail.messages().is_empty());
}

pub async fn a_leftover_provider_user_is_replaced_and_a_verified_one_is_not<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    // A sign-up that failed between the provider and us left an unverified user behind.
    provider.add_user("robin@example.test", "someone elses old password", false);
    assert_eq!(
        harness.sign_up("robin@example.test").await,
        SignupOutcome::CheckYourEmail
    );
    harness.mail.wait_for(1).await;
    assert_eq!(
        harness.mail.messages()[0].purpose,
        MailPurpose::VerifyEmail,
        "the person who owns the mailbox gets a working sign-up"
    );
    harness.verify_and_log_in("robin@example.test").await;
    // A verified user an operator made is not taken over: its owner is told, and signing in
    // with its password creates the account.
    provider.add_user("marian@example.test", "marian has a long password", true);
    let sent = harness.mail.messages().len();
    harness.sign_up("marian@example.test").await;
    harness.mail.wait_for(sent + 1).await;
    assert_eq!(
        harness.mail.messages()[sent].purpose,
        MailPurpose::SignupAttempt
    );
    let LoginOutcome::SignedIn(_) = harness
        .service
        .login(
            "marian@example.test",
            "marian has a long password",
            client(2),
        )
        .await
        .unwrap()
    else {
        panic!("operator-made user refused");
    };
}

pub async fn a_provider_outage_is_unavailable_and_a_lost_sign_up_answer_is_unknown<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    provider.fail_next(crate::IdentityFailure::Unavailable);
    assert_eq!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(2))
            .await
            .err(),
        Some(crate::ServiceError::Unavailable)
    );
    provider.fail_next(crate::IdentityFailure::Unknown);
    assert_eq!(
        harness
            .service
            .signup(
                SignupForm {
                    email: "robin@example.test",
                    password: PASSWORD,
                    operation: harness.operation(),
                    adult_attested: true,
                    terms_version: "terms-2026-10".to_owned(),
                },
                client(1)
            )
            .await
            .err(),
        Some(crate::ServiceError::Unknown)
    );
}

pub async fn the_provider_checks_the_password_for_sensitive_actions_and_closing_deletes_its_user<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    let (caller, _) = harness.verified_account("robin@example.test").await;
    assert!(matches!(
        harness
            .service
            .close(
                &caller,
                crate::Reauthentication::Password("wrong words here!")
            )
            .await
            .unwrap(),
        CloseResult::NotReauthenticated
    ));
    assert!(matches!(
        harness
            .service
            .close(&caller, crate::Reauthentication::Password(PASSWORD))
            .await
            .unwrap(),
        CloseResult::Closed { .. }
    ));
    harness.clock.advance_ms(31 * 24 * 60 * MINUTE_MS);
    harness.service.run_retention_pass().await;
    assert!(
        !provider.has_user("robin@example.test"),
        "the provider's user is deleted"
    );
    let summary = harness.service.dev_summary().await.unwrap();
    assert_eq!((summary.closing, summary.closed), (0, 1));
}

pub async fn password_and_email_changes_are_not_offered_while_the_provider_holds_them<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, _provider) = provider_harness(store).await;
    let (caller, _) = harness.verified_account("robin@example.test").await;
    assert!(matches!(
        harness
            .service
            .change_password(&caller, PASSWORD, NEW_PASSWORD, NEW_PASSWORD, client(2))
            .await
            .unwrap(),
        PasswordChangeResult::ProviderManaged
    ));
    assert_eq!(
        harness
            .service
            .request_email_change(&caller, PASSWORD, "new@example.test", harness.operation())
            .await
            .unwrap(),
        EmailChangeResult::ProviderManaged
    );
}

pub async fn verifying_replaces_the_sign_up_password_and_ends_earlier_sessions<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    // Someone signs up with an address that is not theirs, and signs in.
    harness.sign_up("victim@example.test").await;
    let (attacker, attacker_cookie) = harness.log_in("victim@example.test", PASSWORD).await;
    assert!(!attacker.account.is_verified);
    // The address's owner opens the emailed link and chooses their own password.
    harness.mail.wait_for(1).await;
    let token = harness
        .mail
        .last_token("victim@example.test", MailPurpose::VerifyEmail)
        .unwrap();
    assert_eq!(
        harness
            .service
            .verify_email_and_set_password(
                &token,
                NEW_PASSWORD,
                "another password entirely",
                client(5)
            )
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::Mismatch
    );
    assert_eq!(
        harness
            .service
            .verify_email_and_set_password(&token, NEW_PASSWORD, NEW_PASSWORD, client(5))
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::Verified
    );
    assert!(
        !harness.is_signed_in(&attacker_cookie).await,
        "every session made before the proof is revoked"
    );
    assert!(matches!(
        harness
            .service
            .login("victim@example.test", PASSWORD, client(6))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    harness.log_in("victim@example.test", NEW_PASSWORD).await;
    assert_eq!(
        harness
            .service
            .verify_email_and_set_password(&token, NEW_PASSWORD, NEW_PASSWORD, client(5))
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::AlreadyVerified,
        "the link works once"
    );
}

pub async fn verifying_at_the_provider_replaces_its_password_and_a_refused_password_leaves_the_link<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    harness.sign_up("victim@example.test").await;
    harness.mail.wait_for(1).await;
    let token = harness
        .mail
        .last_token("victim@example.test", MailPurpose::VerifyEmail)
        .unwrap();
    provider.reject_password("a password the provider dislikes");
    assert_eq!(
        harness
            .service
            .verify_email_and_set_password(
                &token,
                "a password the provider dislikes",
                "a password the provider dislikes",
                client(5)
            )
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::ProviderRejectedPassword
    );
    assert_eq!(
        harness
            .service
            .verify_email_and_set_password(&token, NEW_PASSWORD, NEW_PASSWORD, client(5))
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::Verified,
        "the link was not spent by the refusal"
    );
    // The password the sign-up set at the provider no longer works; the chosen one does.
    assert!(matches!(
        harness
            .service
            .login("victim@example.test", PASSWORD, client(6))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    harness.log_in("victim@example.test", NEW_PASSWORD).await;
}

pub async fn a_login_after_the_closing_window_does_not_reopen_the_account<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (caller, _) = harness.verified_account("robin@example.test").await;
    harness
        .service
        .close(&caller, crate::Reauthentication::Password(PASSWORD))
        .await
        .unwrap();
    harness.clock.advance_ms(30 * 24 * 60 * MINUTE_MS);
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(7))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    assert_eq!(
        harness.service.dev_summary().await.unwrap().closing,
        1,
        "the late login changed nothing"
    );
}

pub async fn closing_needs_a_fresh_proof_within_five_minutes<S: AccountStore + Clone>(store: S) {
    let harness = Harness::start(store).await;
    let (caller, _) = harness.verified_account("robin@example.test").await;
    harness.clock.advance_ms(6 * MINUTE_MS);
    assert!(matches!(
        harness
            .service
            .close(&caller, crate::Reauthentication::RecentSignIn)
            .await
            .unwrap(),
        CloseResult::NotReauthenticated
    ));
    // An export accepts a sign-in up to ten minutes old; closing does not.
    assert!(matches!(
        harness
            .service
            .export(&caller, crate::Reauthentication::RecentSignIn, client(2))
            .await
            .unwrap(),
        ExportResult::Document(_)
    ));
    let (fresh, _) = harness.log_in("robin@example.test", PASSWORD).await;
    assert!(matches!(
        harness
            .service
            .close(&fresh, crate::Reauthentication::RecentSignIn)
            .await
            .unwrap(),
        CloseResult::Closed { .. }
    ));
}

pub async fn two_sign_ins_for_an_address_run_at_once_and_a_third_waits<S: AccountStore + Clone>(
    store: S,
) {
    let harness = Harness::start(store).await;
    harness.verified_account("robin@example.test").await;
    let throttles = &harness.service.shared.throttles;
    let first = throttles.enter_sign_in("robin@example.test").unwrap();
    let _second = throttles.enter_sign_in("robin@example.test").unwrap();
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(8))
            .await
            .unwrap(),
        LoginOutcome::Throttled
    ));
    drop(first);
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", PASSWORD, client(8))
            .await
            .unwrap(),
        LoginOutcome::SignedIn(_)
    ));
}

pub async fn a_pass_closes_out_a_bounded_batch_and_the_next_pass_continues<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let total = 22;
    for index in 0..total {
        if index % 4 == 0 {
            // The per-address-and-IP limits are per hour.
            harness.clock.advance_ms(61 * MINUTE_MS);
        }
        let email = format!("closer{index}@example.test");
        let (caller, _) = harness.verified_account(&email).await;
        harness
            .service
            .close(&caller, crate::Reauthentication::Password(PASSWORD))
            .await
            .unwrap();
    }
    harness.clock.advance_ms(31 * 24 * 60 * MINUTE_MS);
    harness.service.run_retention_pass().await;
    let after_one = harness.service.dev_summary().await.unwrap();
    assert_eq!(after_one.closed, 20, "one pass closes out a bounded batch");
    assert_eq!(after_one.closing, 2);
    harness.service.run_retention_pass().await;
    let after_two = harness.service.dev_summary().await.unwrap();
    assert_eq!((after_two.closing, after_two.closed), (0, 22));
}

pub async fn a_provider_failure_keeps_one_account_and_the_pass_goes_on<S: AccountStore + Clone>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    for email in ["a-first@example.test", "b-second@example.test"] {
        let (caller, _) = harness.verified_account(email).await;
        harness
            .service
            .close(&caller, crate::Reauthentication::Password(PASSWORD))
            .await
            .unwrap();
        harness.clock.advance_ms(61 * MINUTE_MS);
    }
    harness.clock.advance_ms(31 * 24 * 60 * MINUTE_MS);
    // The provider is down for the first delete only.
    provider.fail_next(crate::IdentityFailure::Unavailable);
    harness.service.run_retention_pass().await;
    let after_one = harness.service.dev_summary().await.unwrap();
    assert_eq!(
        (after_one.closing, after_one.closed),
        (1, 1),
        "the failing account is kept, the other one is closed out"
    );
    harness.service.run_retention_pass().await;
    let after_two = harness.service.dev_summary().await.unwrap();
    assert_eq!((after_two.closing, after_two.closed), (0, 2));
    assert!(!provider.has_user("a-first@example.test"));
    assert!(!provider.has_user("b-second@example.test"));
}

pub async fn provider_sign_up_takes_the_same_time_for_a_new_and_a_registered_address<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let provider = FakeIdentityProvider::default();
    let hosts = HostConfig::local("http://127.0.0.1:8099").expect("hosts");
    let floor = Duration::from_millis(400);
    let harness =
        Harness::start_configured(store, IdentitySource::provider(provider), hosts, floor)
            .await
            .expect("service starts");
    let started = std::time::Instant::now();
    harness.sign_up("robin@example.test").await;
    let new_address = started.elapsed();
    harness.mail.wait_for(1).await;
    let started = std::time::Instant::now();
    harness.sign_up("robin@example.test").await;
    let registered = started.elapsed();
    assert!(new_address >= floor && registered >= floor);
    assert!(
        new_address.abs_diff(registered) < Duration::from_millis(300),
        "{new_address:?} against {registered:?}"
    );
}

pub async fn local_passwords_are_refused_on_a_public_https_origin_unless_a_dev_flag_says_otherwise<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let hosts = HostConfig::parse(
        r#"{"schema": "dungeonflux.hosts/1", "origin": "https://dungeonfluxdnd.com",
            "mail": {"from_name": "DungeonFlux", "from_address": "accounts@mail.dungeonfluxdnd.com",
                     "reply_to": "support@dungeonfluxdnd.com"}}"#,
    )
    .expect("hosts");
    let refused = Harness::start_configured(
        store,
        IdentitySource::local_passwords(),
        hosts,
        Duration::ZERO,
    )
    .await;
    assert!(matches!(
        refused.err(),
        Some(crate::StartError::LocalPasswordsOnHostedOrigin)
    ));
}

pub async fn a_second_provider_user_with_the_same_address_cannot_take_over_a_linked_account<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    let (_caller, _) = harness.verified_account("robin@example.test").await;
    // Another provider user with the same, verified, address (made at the provider, outside any
    // sign-up of ours) knows its own password.
    provider.add_user("robin@example.test", "a different long password", true);
    assert!(matches!(
        harness
            .service
            .login("robin@example.test", "a different long password", client(9))
            .await
            .unwrap(),
        LoginOutcome::Refused
    ));
    assert_eq!(harness.service.dev_summary().await.unwrap().active, 1);
}

pub async fn a_slow_provider_reset_does_not_hold_up_the_mail_behind_it<S: AccountStore + Clone>(
    store: S,
) {
    let (harness, provider) = provider_harness(store).await;
    harness.verified_account("robin@example.test").await;
    let sent = harness.mail.messages().len();
    provider.delay_next_reset(Duration::from_millis(600));
    harness
        .service
        .request_password_reset("robin@example.test", client(4))
        .await
        .unwrap();
    harness.sign_up("marian@example.test").await;
    harness.mail.wait_for(sent + 1).await;
    let first = harness.mail.messages().remove(sent);
    assert_eq!(
        first.purpose,
        MailPurpose::VerifyEmail,
        "the verification mail does not wait for the slow reset"
    );
    harness.mail.wait_for(sent + 2).await;
    assert_eq!(
        harness.mail.messages().remove(sent + 1).purpose,
        MailPurpose::PasswordReset
    );
}

pub async fn an_operator_lookup_finds_an_account_by_exact_address_or_id_only<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store).await;
    let (robin, _) = harness.verified_account("robin@example.test").await;
    let by_address = harness
        .service
        .find_account(" Robin@Example.test ")
        .await
        .unwrap()
        .expect("found by address");
    assert_eq!(by_address.id, robin.account.id);
    let id_text = robin
        .account
        .id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let by_id = harness.service.find_account(&id_text).await.unwrap();
    assert_eq!(by_id.map(|account| account.id), Some(robin.account.id));
    for query in ["robin", "robin@example", "rob*", "", "0000"] {
        assert!(
            harness.service.find_account(query).await.unwrap().is_none(),
            "{query}"
        );
    }
}

// ---- Account closure (J14) ----

fn id(byte: u8) -> df_types::CampaignId {
    df_types::CampaignId::from_bytes(&[byte; 16]).expect("campaign id")
}

fn seed(
    campaign: u8,
    name: &str,
    owner: df_types::AccountId,
    host: df_types::AccountId,
    archived: bool,
    members: &[df_types::AccountId],
) -> CampaignSeed {
    CampaignSeed {
        campaign: id(campaign),
        name: name.to_owned(),
        owner,
        host,
        archived,
        members: members.to_vec(),
    }
}

pub async fn closing_is_blocked_while_the_account_owns_or_hosts_an_active_campaign<
    S: AccountStore + Clone + CampaignSeeding,
>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    let (robin, cookie) = harness.verified_account("robin@example.test").await;
    let (marian, _) = harness.verified_account("marian@example.test").await;
    store
        .seed_campaign(seed(
            1,
            "Owned",
            robin.account.id,
            robin.account.id,
            false,
            &[],
        ))
        .await;
    store
        .seed_campaign(seed(
            2,
            "Hosted",
            marian.account.id,
            robin.account.id,
            false,
            &[],
        ))
        .await;
    let CloseResult::Blocked(check) = harness
        .service
        .close(&robin, crate::Reauthentication::Password(PASSWORD))
        .await
        .unwrap()
    else {
        panic!("closure not blocked");
    };
    let mut reasons: Vec<_> = check
        .blocking
        .iter()
        .map(|blocker| (blocker.name.as_str(), blocker.reason))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        [
            ("Hosted", BlockReason::Hosts),
            ("Owned", BlockReason::OwnsAndHosts)
        ]
    );
    assert!(
        harness.is_signed_in(&cookie).await,
        "a blocked closure changes nothing: the person stays signed in"
    );
    let still = harness
        .service
        .authenticate(&cookie)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still.account.status, crate::AccountStatus::Active);
}

pub async fn closing_schedules_archived_deletions_cancels_proposals_and_reopening_undoes_them<
    S: AccountStore + Clone + CampaignSeeding,
>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    let (robin, _) = harness.verified_account("robin@example.test").await;
    let (marian, _) = harness.verified_account("marian@example.test").await;
    store
        .seed_campaign(seed(
            3,
            "Old tales",
            robin.account.id,
            robin.account.id,
            true,
            &[robin.account.id, marian.account.id],
        ))
        .await;
    store.seed_outgoing_proposal(robin.account.id).await;
    let sent = harness.mail.messages().len();
    let CloseResult::Closed { delete_after_ms } = harness
        .service
        .close(&robin, crate::Reauthentication::Password(PASSWORD))
        .await
        .unwrap()
    else {
        panic!("closure refused");
    };
    assert_eq!(store.scheduled_deletion(id(3)).await, Some(delete_after_ms));
    assert_eq!(store.open_proposals_from(robin.account.id).await, 0);
    harness.mail.wait_for(sent + 2).await;
    let notices: Vec<_> = harness
        .mail
        .messages()
        .into_iter()
        .filter(|message| message.purpose == MailPurpose::CampaignDeletionScheduled)
        .collect();
    assert!(
        notices
            .iter()
            .any(|message| message.to == "marian@example.test")
    );
    assert!(notices[0].body.contains("Old tales"));
    assert!(notices[0].body.contains(&crate::utc_date(delete_after_ms)));
    harness.log_in("robin@example.test", PASSWORD).await;
    assert_eq!(
        store.scheduled_deletion(id(3)).await,
        None,
        "reopening cancels the scheduled deletion"
    );
}

pub async fn a_completed_closure_leaves_campaigns_and_new_blockers_keep_the_account_closing<
    S: AccountStore + Clone + CampaignSeeding,
>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    let (robin, _) = harness.verified_account("robin@example.test").await;
    let (marian, _) = harness.verified_account("marian@example.test").await;
    store
        .seed_campaign(seed(
            4,
            "Marian's table",
            marian.account.id,
            marian.account.id,
            false,
            &[robin.account.id, marian.account.id],
        ))
        .await;
    assert!(matches!(
        harness
            .service
            .close(&robin, crate::Reauthentication::Password(PASSWORD))
            .await
            .unwrap(),
        CloseResult::Closed { .. }
    ));
    // Robin becomes host after closing (a transfer the owner made): the window must not
    // complete the closure over it.
    store
        .seed_campaign(seed(
            5,
            "Handed over",
            marian.account.id,
            robin.account.id,
            false,
            &[],
        ))
        .await;
    harness.clock.advance_ms(31 * 24 * 60 * MINUTE_MS);
    harness.service.run_retention_pass().await;
    assert_eq!(harness.service.dev_summary().await.unwrap().closing, 1);
    assert!(store.is_member(id(4), robin.account.id).await);
    store.archive_campaign(id(5)).await;
    harness.service.run_retention_pass().await;
    let summary = harness.service.dev_summary().await.unwrap();
    assert_eq!((summary.closing, summary.closed), (0, 1));
    assert!(
        !store.is_member(id(4), robin.account.id).await,
        "completion leaves the campaigns the account only played in"
    );
}

/// The closure scenarios, for stores that also model the campaign owner's participant
/// (`fakes::CampaignSeeding`).
#[macro_export]
macro_rules! closure_contract_tests {
    ($make:path) => {
        $crate::store_contract_tests!(@each $make;
            closing_is_blocked_while_the_account_owns_or_hosts_an_active_campaign,
            closing_schedules_archived_deletions_cancels_proposals_and_reopening_undoes_them,
            a_completed_closure_leaves_campaigns_and_new_blockers_keep_the_account_closing
        );
    };
}

/// Generates one `#[test]` per contract scenario for the fixture `$make` (an async function
/// returning a `contract::Fixture`).
#[macro_export]
macro_rules! store_contract_tests {
    ($make:path) => {
        $crate::store_contract_tests!(@each $make;
            signup_verify_login_and_logout_work_end_to_end,
            a_double_submitted_signup_creates_one_account_and_one_message,
            signing_up_with_a_registered_address_looks_the_same_and_warns_its_owner,
            a_password_change_signs_out_other_devices_and_rotates_this_one,
            a_reset_link_works_once_within_fifteen_minutes_and_signs_out_everywhere,
            a_reset_for_an_unknown_address_sends_nothing,
            an_email_change_needs_both_addresses_to_confirm,
            the_old_address_can_cancel_an_email_change,
            signing_out_other_devices_keeps_this_one,
            one_account_cannot_revoke_or_see_another_accounts_sessions,
            closing_an_account_signs_out_and_logging_in_within_the_window_reopens_it,
            retention_deletes_a_closed_accounts_personal_data_after_the_window,
            an_export_holds_the_account_and_its_security_history_and_no_secrets,
            provider_signup_verify_login_and_logout_work_end_to_end,
            a_registered_address_signs_up_like_a_new_one_and_warns_its_owner_at_the_provider,
            a_wrong_password_and_an_unknown_address_look_alike_at_the_provider,
            a_provider_that_requires_its_own_verification_is_a_refused_sign_in_like_any_other,
            a_provider_that_does_not_require_verification_signs_an_unverified_person_in,
            a_provider_reset_works_once_and_signs_out_everywhere,
            a_provider_reset_for_an_unknown_address_sends_nothing,
            a_leftover_provider_user_is_replaced_and_a_verified_one_is_not,
            a_provider_outage_is_unavailable_and_a_lost_sign_up_answer_is_unknown,
            the_provider_checks_the_password_for_sensitive_actions_and_closing_deletes_its_user,
            password_and_email_changes_are_not_offered_while_the_provider_holds_them,
            an_operator_lookup_finds_an_account_by_exact_address_or_id_only,
            verifying_replaces_the_sign_up_password_and_ends_earlier_sessions,
            verifying_at_the_provider_replaces_its_password_and_a_refused_password_leaves_the_link,
            a_login_after_the_closing_window_does_not_reopen_the_account,
            closing_needs_a_fresh_proof_within_five_minutes,
            two_sign_ins_for_an_address_run_at_once_and_a_third_waits,
            a_pass_closes_out_a_bounded_batch_and_the_next_pass_continues,
            a_provider_failure_keeps_one_account_and_the_pass_goes_on,
            provider_sign_up_takes_the_same_time_for_a_new_and_a_registered_address,
            local_passwords_are_refused_on_a_public_https_origin_unless_a_dev_flag_says_otherwise,
            a_second_provider_user_with_the_same_address_cannot_take_over_a_linked_account,
            a_slow_provider_reset_does_not_hold_up_the_mail_behind_it
        );
    };
    (@each $make:path; $($scenario:ident),+) => {
        $(
            #[test]
            fn $scenario() {
                let runtime = ::tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("test runtime");
                runtime.block_on(async {
                    let fixture = $make().await;
                    $crate::contract::$scenario(
                        $crate::contract::Fixture::store(&fixture),
                    )
                    .await;
                    $crate::contract::Fixture::finish(fixture).await;
                });
            }
        )+
    };
}
