//! Customer accounts: signup, email verification, login and sessions, password change and
//! reset, email change, profile, data export and closure (planning/account-service-research.md).
//! The credentials live either in the account store (development and self-hosting) or at an
//! identity provider (`IdentitySource`; hosted production).
//!
//! Owns the account rules' native side: hashing, tokens, throttles, mail and retention. It does
//! not own storage (the `AccountStore` port, implemented in `df-persistence`), HTTP (`df-portal`)
//! or any game state; it never runs on the game owner thread. Store and mail failures are typed
//! `ServiceError`s; an uncertain mutation is `Unknown`, never reported as failed or done.
#![forbid(unsafe_code)]
#![cfg(not(target_arch = "wasm32"))]

mod access;
mod clock;
mod closure;
mod common_passwords;
mod credentials;
mod dev_outbox;
mod email;
mod hosts;
mod lifecycle;
mod mail_worker;
mod oauth;
mod oauth_provider;
mod oauth_store;
pub mod operator_contract;
pub mod operator_ports;
mod operators;
mod password_hashing;
mod ports;
mod provider;
mod provider_flows;
mod records;
mod reset_authority;
mod retention;
mod service;
mod throttle;
mod tokens;
mod tunables;

#[cfg(any(test, feature = "test-fakes"))]
pub mod fakes;

#[cfg(any(test, feature = "test-fakes"))]
pub mod contract;

#[cfg(any(test, feature = "test-fakes"))]
pub mod oauth_contract;

#[cfg(any(test, feature = "test-fakes"))]
pub mod tenant_contract;

pub use access::{
    Authenticated, IssuedSession, LoginOutcome, ResendOutcome, SignupForm, SignupOutcome,
    SignupRefusal, VerifyEmailOutcome, VerifyWithPasswordOutcome,
};
#[cfg(any(test, feature = "test-fakes"))]
pub use clock::FakeClock;
pub use clock::{Clock, SystemClock};
pub use closure::{
    BlockReason, BlockingCampaign, CloseOutcome, ClosureCheck, CompletionOutcome, LeaveChunk,
    OwnedArchivedCampaign, ScheduledDeletion,
};
pub use common_passwords::{BuiltInCommonPasswords, CommonPasswords};
pub use credentials::{
    EmailChangeResult, EmailConfirmResult, PasswordChangeResult, ResetConfirmOutcome,
    ResetRequestOutcome,
};
pub use dev_outbox::{DevOutbox, MAXIMUM_OUTBOX_MESSAGES, OutboxEntry};
pub use df_auth::password_policy::PasswordRefusal;
pub use email::{LinkStyle, MailPurpose, Message, utc_date};
pub use hosts::{
    HOSTS_SCHEMA, HexId, HostConfig, HostsRefusal, MailIdentity, OriginRefusal, PublicOrigin,
};
pub use lifecycle::{
    CLOSURE_REAUTHENTICATION_MS, CloseResult, ExportResult, ProfileResult, RECENT_SIGN_IN_MS,
    Reauthentication, SecurityOverview,
};
pub use oauth::{OAuthOutcome, OAuthStart, OAuthStartForm};
pub use oauth_provider::{
    OAuthAuthorize, OAuthCall, OAuthExchange, OAuthFailure, OAuthIdentity, OAuthProvider,
    OAuthSecret, pkce_challenge,
};
pub use oauth_store::{
    GoogleResolution, GoogleResolved, GoogleSession, GoogleStore, GoogleStoreCall, OAuthAttempt,
    OAuthIntent, OAuthRefusal,
};
pub use operator_ports::OperatorStore;
pub use operators::OperatorService;
pub use password_hashing::HashingParameters;
pub use ports::{AccountStore, EmailError, EmailSender, ResetPolicy, StoreError, StoreResult};
pub use provider::{
    Authentication, CallerContext, CreateUser, IdentityFailure, IdentityProvider,
    IdentityProviderKind, IdentitySource, ProviderUser, ResetFinish, ResetStart, SetPassword,
};
pub use records::{
    AccountEvent, AccountRecord, AccountStatus, AccountsSummary, ConfirmChangeOutcome,
    CreateOutcome, DeviceLabel, DisplayName, DisplayNameRefusal, EmailAddress, EmailChangeDecision,
    EmailChangeOutcome, EmailChangeRequest, EmailRefusal, EventKind, NewAccount,
    NewIdentityAccount, NewSession, NewToken, OperationClaim, OperationKey, OperationNamespace,
    PasswordChange, PasswordChangeOutcome, PurgeCounts, ReceiptOutcome, RecentAccount,
    ResetOutcome, ResetSnapshot, RetentionCutoffs, RevokeReason, RevokeScope, SessionRecord,
    SessionSummary, TokenPurpose, VerificationTarget, VerifyOutcome,
};
pub use service::{
    Client, CustomerConfig, CustomerService, ServiceError, ServiceTasks, StartError,
};
pub use tokens::{CsrfSecret, PasswordHash, SecretToken, TokenDigest};
pub use tunables::{PORTAL_SCHEMA, Tunables};

#[cfg(test)]
mod memory_contract {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use crate::{
        AccountStore, BuiltInCommonPasswords, CustomerConfig, CustomerService, EventKind,
        HashingParameters, HostConfig, IdentitySource, LoginOutcome, SignupForm, SignupOutcome,
        Tunables,
        clock::FakeClock,
        contract::{Fixture, START_MS, client},
        fakes::{FakeIdentityProvider, MemoryAccountStore, RecordingMail},
    };

    struct MemoryFixture(MemoryAccountStore);

    impl Fixture for MemoryFixture {
        type Store = MemoryAccountStore;

        fn store(&self) -> MemoryAccountStore {
            self.0.clone()
        }

        async fn finish(self) {}
    }

    async fn memory_fixture() -> MemoryFixture {
        MemoryFixture(MemoryAccountStore::default())
    }

    crate::store_contract_tests!(memory_fixture);
    crate::closure_contract_tests!(memory_fixture);

    #[test]
    fn mistyped_password_refusals_finish_near_the_short_floor_for_known_and_unknown_addresses() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            for identity in [
                IdentitySource::local_passwords(),
                IdentitySource::provider(FakeIdentityProvider::default()),
            ] {
                let store = MemoryAccountStore::default();
                let clock = FakeClock::at(START_MS);
                let mail = RecordingMail::default();
                let floor = Duration::from_millis(750);
                let mut tunables = Tunables::default();
                tunables.sign_in_failure_floor = floor;
                let (service, _tasks) = CustomerService::start(
                    CustomerConfig {
                        hosts: HostConfig::local("http://127.0.0.1:8099").expect("hosts"),
                        identity,
                        tunables,
                        hashing: HashingParameters::TEST,
                        maximum_concurrent_hashes: 2,
                        hash_permit_wait: Duration::from_secs(5),
                        provider_signup_floor: floor,
                        dev_allow_local_passwords_on_https: false,
                    },
                    store.clone(),
                    mail,
                    Arc::new(clock),
                    Arc::new(BuiltInCommonPasswords),
                )
                .await
                .expect("service starts");
                let operation = service.new_operation().expect("operation");
                assert_eq!(
                    service
                        .signup(
                            SignupForm {
                                email: "known@example.test",
                                password: "lantern under the old mill",
                                operation,
                                adult_attested: true,
                                terms_version: service.current_terms_version().to_owned(),
                            },
                            client(1),
                        )
                        .await
                        .expect("signup"),
                    SignupOutcome::CheckYourEmail
                );
                store.set_event_write_delay(Duration::from_millis(1_200));

                let known_started = Instant::now();
                let known = service
                    .login("known@example.test", "wrong password", client(2))
                    .await
                    .expect("known failure");
                let known_elapsed = known_started.elapsed();
                let unknown_started = Instant::now();
                let unknown = service
                    .login("unknown@example.test", "wrong password", client(3))
                    .await
                    .expect("unknown failure");
                let unknown_elapsed = unknown_started.elapsed();

                assert!(matches!(known, LoginOutcome::Refused));
                assert!(matches!(unknown, LoginOutcome::Refused));
                let maximum_response = floor + Duration::from_millis(250);
                assert!(known_elapsed <= maximum_response, "known={known_elapsed:?}");
                assert!(
                    unknown_elapsed <= maximum_response,
                    "unknown={unknown_elapsed:?}"
                );
                assert!(known_elapsed < Duration::from_secs(2));
                assert!(unknown_elapsed < Duration::from_secs(2));
                assert!(
                    known_elapsed.abs_diff(unknown_elapsed) <= Duration::from_millis(100),
                    "known={known_elapsed:?}, unknown={unknown_elapsed:?}"
                );
                tokio::time::timeout(Duration::from_secs(3), async {
                    loop {
                        let account = store
                            .account_by_email("known@example.test")
                            .await
                            .expect("account lookup")
                            .expect("known account");
                        let events = store.events(account.id, 10).await.expect("events");
                        if events
                            .iter()
                            .any(|event| event.kind == EventKind::LoginRefused)
                        {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("owned worker records the known account refusal");
            }
        });
    }

    #[test]
    fn signup_requires_current_terms_and_adult_attestation() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let store = MemoryAccountStore::default();
            let harness = crate::contract::Harness::start(store.clone()).await;
            let operation = harness.service.new_operation().expect("operation");
            let missing_adult = harness
                .service
                .signup(
                    SignupForm {
                        email: "first@example.test",
                        password: "lantern under the old mill",
                        operation,
                        adult_attested: false,
                        terms_version: harness.service.current_terms_version().to_owned(),
                    },
                    client(1),
                )
                .await
                .expect("missing adult attestation");
            assert_eq!(
                missing_adult,
                SignupOutcome::Refused(crate::SignupRefusal::AdultAttestationRequired)
            );
            let outdated = harness
                .service
                .signup(
                    SignupForm {
                        email: "second@example.test",
                        password: "lantern under the old mill",
                        operation: harness.service.new_operation().expect("operation"),
                        adult_attested: true,
                        terms_version: "terms-old".to_owned(),
                    },
                    client(1),
                )
                .await
                .expect("outdated terms");
            assert_eq!(
                outdated,
                SignupOutcome::Refused(crate::SignupRefusal::TermsVersionChanged)
            );
            assert_eq!(
                harness.service.dev_summary().await.expect("summary").active,
                0
            );
        });
    }
}
