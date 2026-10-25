//! The account service over the WorkOS adapter and the in-memory WorkOS: the whole person-level
//! flows, end to end, with the memory account store and recorded mail.

use std::{sync::Arc, time::Duration};

use df_customer::{
    BuiltInCommonPasswords, Client, CustomerConfig, CustomerService, DeviceLabel, FakeClock,
    HashingParameters, HostConfig, IdentitySource, LoginOutcome, MailPurpose, Reauthentication,
    ResetConfirmOutcome, ServiceError, ServiceTasks, SignupForm, SignupOutcome, Tunables,
    VerifyWithPasswordOutcome,
    contract::{NEW_PASSWORD, PASSWORD, START_MS},
    fakes::{MemoryAccountStore, RecordingMail},
};

use crate::{WorkOsConfig, WorkOsIdentity, fake::FakeWorkOsServer};

const KEY: &str = "sk_test_fake_key";
const EMAIL: &str = "robin@example.test";

struct Harness {
    service: CustomerService<MemoryAccountStore, RecordingMail>,
    mail: RecordingMail,
    clock: FakeClock,
    workos: FakeWorkOsServer,
    _tasks: ServiceTasks,
}

fn client() -> Client {
    Client {
        ip: Some([192, 0, 2, 7].into()),
        device: DeviceLabel::Desktop,
    }
}

async fn harness() -> Harness {
    let hosts = HostConfig::parse(
        r#"{"schema": "dungeonflux.hosts/1", "origin": "https://staging.dungeonfluxdnd.com",
            "mail": {"from_name": "DungeonFlux", "from_address": "accounts@mail.dungeonfluxdnd.com",
                     "reply_to": "support@dungeonfluxdnd.com"}}"#,
    )
    .unwrap();
    let workos =
        FakeWorkOsServer::new(KEY, "https://staging.dungeonfluxdnd.com/app/reset-password");
    let config = WorkOsConfig::new("client_fake", KEY.to_owned(), &hosts).unwrap();
    let clock = FakeClock::at(START_MS);
    let mail = RecordingMail::default();
    let mut tunables = Tunables::default();
    tunables.sign_in_failure_floor = Duration::ZERO;
    let (service, tasks) = CustomerService::start(
        CustomerConfig {
            hosts,
            identity: IdentitySource::provider(WorkOsIdentity::new(workos.clone(), config)),
            tunables,
            hashing: HashingParameters::TEST,
            maximum_concurrent_hashes: 2,
            hash_permit_wait: Duration::from_secs(5),
            provider_signup_floor: Duration::ZERO,
            dev_allow_local_passwords_on_https: false,
        },
        MemoryAccountStore::default(),
        mail.clone(),
        Arc::new(clock.clone()),
        Arc::new(BuiltInCommonPasswords),
    )
    .await
    .unwrap();
    Harness {
        service,
        mail,
        clock,
        workos,
        _tasks: tasks,
    }
}

impl Harness {
    async fn sign_up(&self, email: &str) -> Result<SignupOutcome, ServiceError> {
        self.service
            .signup(
                SignupForm {
                    email,
                    password: PASSWORD,
                    operation: self.service.new_operation().unwrap(),
                    adult_attested: true,
                    terms_version: self.service.current_terms_version().to_owned(),
                },
                client(),
            )
            .await
    }

    async fn verify(&self, email: &str, expected_mail: usize) {
        self.mail.wait_for(expected_mail).await;
        let token = self
            .mail
            .last_token(email, MailPurpose::VerifyEmail)
            .unwrap();
        assert!(matches!(
            self.service
                .verify_email_and_set_password(&token, PASSWORD, PASSWORD, client())
                .await
                .unwrap(),
            VerifyWithPasswordOutcome::Verified(_)
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sign_up_verify_sign_in_close_and_retention_delete_the_workos_user() {
    let h = harness().await;
    assert_eq!(
        h.sign_up(EMAIL).await.unwrap(),
        SignupOutcome::CheckYourEmail
    );
    assert!(h.workos.has_user(EMAIL));
    assert!(
        !h.workos.is_verified(EMAIL),
        "only our emailed link verifies"
    );
    h.mail.wait_for(1).await;
    let message = h.mail.messages().remove(0);
    assert!(
        message
            .body
            .contains("https://staging.dungeonfluxdnd.com/app/verify-email#token=")
    );
    assert_eq!(message.reply_to, "support@dungeonfluxdnd.com");
    h.verify(EMAIL, 1).await;
    assert!(h.workos.is_verified(EMAIL));
    let LoginOutcome::SignedIn(issued) = h.service.login(EMAIL, PASSWORD, client()).await.unwrap()
    else {
        panic!("sign-in refused");
    };
    assert_eq!(
        h.workos.open_sessions(),
        0,
        "the provider session is ended at once"
    );
    let caller = h
        .service
        .authenticate(&issued.token.text())
        .await
        .unwrap()
        .unwrap();
    h.service
        .close(&caller, Reauthentication::Password(PASSWORD))
        .await
        .unwrap();
    h.clock.advance_ms(31 * 24 * 60 * 60 * 1000);
    h.service.run_retention_pass().await;
    assert!(!h.workos.has_user(EMAIL));
    assert_eq!(h.service.dev_summary().await.unwrap().closed, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reset_goes_through_workos_with_our_email_and_signs_out_everywhere() {
    let h = harness().await;
    h.sign_up(EMAIL).await.unwrap();
    h.verify(EMAIL, 1).await;
    let LoginOutcome::SignedIn(issued) = h.service.login(EMAIL, PASSWORD, client()).await.unwrap()
    else {
        panic!("sign-in refused");
    };
    h.service
        .request_password_reset(EMAIL, client())
        .await
        .unwrap();
    h.mail.wait_for(2).await;
    let message = h.mail.messages().remove(1);
    assert_eq!(message.purpose, MailPurpose::PasswordReset);
    assert!(
        message
            .body
            .contains("https://staging.dungeonfluxdnd.com/app/reset-password#token=reset_token_")
    );
    let token = h
        .mail
        .last_token(EMAIL, MailPurpose::PasswordReset)
        .unwrap();
    assert_eq!(
        h.service
            .confirm_password_reset(&token, NEW_PASSWORD, NEW_PASSWORD, client())
            .await
            .unwrap(),
        ResetConfirmOutcome::Reset
    );
    assert!(
        h.service
            .authenticate(&issued.token.text())
            .await
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        h.service.login(EMAIL, PASSWORD, client()).await.unwrap(),
        LoginOutcome::Refused
    ));
    assert!(matches!(
        h.service
            .login(EMAIL, NEW_PASSWORD, client())
            .await
            .unwrap(),
        LoginOutcome::SignedIn(_)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sign_up_whose_answer_was_lost_leaves_a_user_the_next_attempt_replaces() {
    let h = harness().await;
    h.workos.lose_next_response();
    assert_eq!(h.sign_up(EMAIL).await, Err(ServiceError::Unknown));
    assert!(
        h.workos.has_user(EMAIL),
        "WorkOS acted before the answer was lost"
    );
    assert_eq!(h.service.dev_summary().await.unwrap().active, 0);
    assert_eq!(
        h.sign_up(EMAIL).await.unwrap(),
        SignupOutcome::CheckYourEmail
    );
    h.verify(EMAIL, 1).await;
    assert!(matches!(
        h.service.login(EMAIL, PASSWORD, client()).await.unwrap(),
        LoginOutcome::SignedIn(_)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workos_outage_is_unavailable() {
    let h = harness().await;
    h.workos.fail_next(crate::TransportError::NotSent);
    assert_eq!(
        h.service.login(EMAIL, PASSWORD, client()).await.err(),
        Some(ServiceError::Unavailable)
    );
}
