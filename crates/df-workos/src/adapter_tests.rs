//! The adapter against the in-memory WorkOS: every port call over real request and answer
//! bodies, and the failure classes.

use df_customer::{
    Authentication, CallerContext, CreateUser, EmailAddress, HostConfig, IdentityFailure,
    IdentityProvider, ResetFinish, ResetStart, SetPassword,
};

use crate::{
    ConfigRefusal, Method, TransportError, WorkOsConfig, WorkOsIdentity, fake::FakeWorkOsServer,
};

const KEY: &str = "sk_test_fake_key";
const RESET_URL: &str = "https://staging.dungeonfluxdnd.com/app/reset-password";
const PASSWORD: &str = "lantern under the old mill";

fn hosts() -> HostConfig {
    HostConfig::parse(
        r#"{"schema": "dungeonflux.hosts/1", "origin": "https://staging.dungeonfluxdnd.com",
            "mail": {"from_name": "DungeonFlux", "from_address": "accounts@mail.dungeonfluxdnd.com",
                     "reply_to": "support@dungeonfluxdnd.com"}}"#,
    )
    .unwrap()
}

fn adapter_with(
    key: &str,
    reset_url: &str,
) -> (WorkOsIdentity<FakeWorkOsServer>, FakeWorkOsServer) {
    let server = FakeWorkOsServer::new(KEY, reset_url);
    let config = WorkOsConfig::new("client_fake", key.to_owned(), &hosts()).unwrap();
    (WorkOsIdentity::new(server.clone(), config), server)
}

fn adapter() -> (WorkOsIdentity<FakeWorkOsServer>, FakeWorkOsServer) {
    adapter_with(KEY, RESET_URL)
}

fn email(text: &str) -> EmailAddress {
    EmailAddress::parse(text).unwrap()
}

#[tokio::test]
async fn a_new_user_is_unverified_and_a_taken_or_weak_one_is_refused() {
    let (adapter, server) = adapter();
    let CreateUser::Created(user) = adapter
        .create_user(
            &email("Robin@Example.test"),
            PASSWORD,
            CallerContext::default(),
        )
        .await
        .unwrap()
    else {
        panic!("not created");
    };
    assert!(!user.email_verified);
    assert_eq!(user.email.normalized(), "robin@example.test");
    assert_eq!(
        adapter
            .create_user(
                &email("robin@example.test"),
                PASSWORD,
                CallerContext::default()
            )
            .await
            .unwrap(),
        CreateUser::EmailNotAvailable
    );
    server.reject_password("a password workos dislikes");
    assert_eq!(
        adapter
            .create_user(
                &email("marian@example.test"),
                "a password workos dislikes",
                CallerContext::default()
            )
            .await
            .unwrap(),
        CreateUser::PasswordRejected
    );
    assert!(!server.has_user("marian@example.test"));
}

#[tokio::test]
async fn a_wrong_password_and_an_unknown_address_look_alike_and_an_unverified_one_is_named() {
    let (adapter, server) = adapter();
    server.require_verified(true);
    server.add_user("robin@example.test", PASSWORD, false);
    let caller = CallerContext::default();
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), "not the password!", caller)
            .await
            .unwrap(),
        Authentication::InvalidCredentials
    );
    assert_eq!(
        adapter
            .authenticate(&email("nobody@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        Authentication::InvalidCredentials
    );
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        Authentication::EmailVerificationRequired
    );
    assert_eq!(server.open_sessions(), 0);
}

#[tokio::test]
async fn a_sign_in_names_its_provider_session_and_ending_it_leaves_none_open() {
    let (adapter, server) = adapter();
    let subject = server.add_user("robin@example.test", PASSWORD, true);
    let Authentication::Authenticated {
        user,
        provider_session,
    } = adapter
        .authenticate(
            &email("robin@example.test"),
            PASSWORD,
            CallerContext::default(),
        )
        .await
        .unwrap()
    else {
        panic!("not authenticated");
    };
    assert_eq!(user.subject, subject);
    assert_eq!(server.open_sessions(), 1);
    adapter
        .end_session(&provider_session.expect("sid claim"))
        .await
        .unwrap();
    assert_eq!(server.open_sessions(), 0);
}

#[tokio::test]
async fn marking_verified_lets_an_unverified_user_in_and_deleting_is_repeatable() {
    let (adapter, server) = adapter();
    let subject = server.add_user("robin@example.test", PASSWORD, false);
    adapter.mark_email_verified(&subject).await.unwrap();
    assert!(server.is_verified("robin@example.test"));
    let found = adapter
        .find_user(&email("robin@example.test"))
        .await
        .unwrap()
        .unwrap();
    assert!(found.email_verified);
    assert!(server.requests().iter().any(|(method, path)| {
        *method == Method::Get && path.ends_with("email=robin%40example.test")
    }));
    adapter.delete_user(&subject).await.unwrap();
    adapter.delete_user(&subject).await.unwrap();
    assert!(!server.has_user("robin@example.test"));
    assert!(
        adapter
            .find_user(&email("robin@example.test"))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_reset_token_works_once_ends_sessions_and_a_weak_password_keeps_it_usable() {
    let (adapter, server) = adapter();
    server.add_user("robin@example.test", PASSWORD, true);
    let caller = CallerContext::default();
    adapter
        .authenticate(&email("robin@example.test"), PASSWORD, caller)
        .await
        .unwrap();
    assert_eq!(server.open_sessions(), 1);
    let ResetStart::Started { token } = adapter
        .begin_password_reset(&email("robin@example.test"))
        .await
        .unwrap()
    else {
        panic!("no token");
    };
    server.reject_password("too weak for workos");
    assert_eq!(
        adapter
            .finish_password_reset(&token, "too weak for workos")
            .await
            .unwrap(),
        ResetFinish::PasswordRejected
    );
    let ResetFinish::Reset(user) = adapter
        .finish_password_reset(&token, "the miller keeps a second key")
        .await
        .unwrap()
    else {
        panic!("reset refused");
    };
    assert_eq!(user.email.normalized(), "robin@example.test");
    assert_eq!(
        server.open_sessions(),
        0,
        "a reset ends the user's sessions"
    );
    assert_eq!(
        adapter
            .finish_password_reset(&token, "the miller keeps a second key")
            .await
            .unwrap(),
        ResetFinish::InvalidToken
    );
    assert_eq!(
        adapter
            .begin_password_reset(&email("nobody@example.test"))
            .await
            .unwrap(),
        ResetStart::NoSuchUser
    );
}

#[tokio::test]
async fn a_reset_that_points_outside_the_allowlist_is_not_used() {
    let (adapter, server) = adapter_with(KEY, "https://elsewhere.example/reset");
    server.add_user("robin@example.test", PASSWORD, true);
    assert_eq!(
        adapter
            .begin_password_reset(&email("robin@example.test"))
            .await,
        Err(IdentityFailure::Malformed)
    );
}

#[tokio::test]
async fn a_wrong_api_key_is_an_outage_that_changes_nothing() {
    let (adapter, server) = adapter_with("sk_test_wrong_key", RESET_URL);
    assert_eq!(
        adapter
            .create_user(
                &email("robin@example.test"),
                PASSWORD,
                CallerContext::default()
            )
            .await,
        Err(IdentityFailure::Unavailable)
    );
    assert!(!server.has_user("robin@example.test"));
}

#[tokio::test]
async fn transport_failures_are_unavailable_unless_a_mutation_may_have_happened() {
    let (adapter, server) = adapter();
    let caller = CallerContext::default();
    server.fail_next(TransportError::NotSent);
    assert_eq!(
        adapter
            .create_user(&email("robin@example.test"), PASSWORD, caller)
            .await,
        Err(IdentityFailure::Unavailable)
    );
    // The answer is lost after WorkOS acted: unknown, and the user exists.
    server.lose_next_response();
    assert_eq!(
        adapter
            .create_user(&email("robin@example.test"), PASSWORD, caller)
            .await,
        Err(IdentityFailure::Unknown)
    );
    assert!(server.has_user("robin@example.test"));
    // A lost read changed nothing.
    server.fail_next(TransportError::Unknown);
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), PASSWORD, caller)
            .await,
        Err(IdentityFailure::Unavailable)
    );
}

#[test]
fn the_config_refuses_empty_credentials_hides_the_key_and_lists_the_redirects_to_register() {
    assert_eq!(
        WorkOsConfig::new("", KEY.to_owned(), &hosts()).unwrap_err(),
        ConfigRefusal::ClientId
    );
    assert_eq!(
        WorkOsConfig::new("client_x", " ".to_owned(), &hosts()).unwrap_err(),
        ConfigRefusal::ApiKey
    );
    let config = WorkOsConfig::new("client_x", KEY.to_owned(), &hosts()).unwrap();
    assert!(!format!("{config:?}").contains(KEY));
    assert!(config.redirect_allowlist().contains(&RESET_URL.to_owned()));
}

#[tokio::test]
async fn an_unrecognized_refusal_at_sign_in_is_a_refused_credential_and_a_named_step_is_an_outage()
{
    let (adapter, server) = adapter();
    server.add_user("robin@example.test", PASSWORD, true);
    let caller = CallerContext::default();
    server.answer_next(422, "something_new_we_never_saw");
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        Authentication::InvalidCredentials,
        "fail closed: an unknown refusal never signs anyone in"
    );
    for step in [
        "mfa_challenge",
        "sso_required",
        "organization_selection_required",
    ] {
        server.answer_next(403, step);
        assert_eq!(
            adapter
                .authenticate(&email("robin@example.test"), PASSWORD, caller)
                .await,
            Err(IdentityFailure::Unavailable),
            "{step}"
        );
    }
    server.answer_next(429, "rate_limit_exceeded");
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), PASSWORD, caller)
            .await,
        Err(IdentityFailure::Unavailable)
    );
}

#[tokio::test]
async fn an_unrecognized_refusal_at_sign_up_asks_whether_the_address_is_taken() {
    let (adapter, server) = adapter();
    let caller = CallerContext::default();
    server.add_user("robin@example.test", PASSWORD, true);
    server.answer_next(422, "something_new_we_never_saw");
    assert_eq!(
        adapter
            .create_user(&email("robin@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        CreateUser::EmailNotAvailable,
        "a taken address reads as taken whatever the error looked like"
    );
    server.answer_next(422, "something_new_we_never_saw");
    assert_eq!(
        adapter
            .create_user(&email("marian@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        CreateUser::PasswordRejected
    );
    assert!(!server.has_user("marian@example.test"));
}

#[tokio::test]
async fn an_unrecognized_refusal_at_a_reset_is_a_spent_token() {
    let (adapter, server) = adapter();
    server.add_user("robin@example.test", PASSWORD, true);
    server.answer_next(422, "something_new_we_never_saw");
    assert_eq!(
        adapter
            .finish_password_reset("reset_token_000042_abcdef", "the miller keeps a second key")
            .await
            .unwrap(),
        ResetFinish::InvalidToken
    );
}

#[tokio::test]
async fn set_password_replaces_the_password_ends_sessions_and_a_weak_one_is_rejected() {
    let (adapter, server) = adapter();
    let subject = server.add_user("robin@example.test", PASSWORD, true);
    let caller = CallerContext::default();
    adapter
        .authenticate(&email("robin@example.test"), PASSWORD, caller)
        .await
        .unwrap();
    server.reject_password("a password workos dislikes");
    assert_eq!(
        adapter
            .set_password(&subject, "a password workos dislikes")
            .await
            .unwrap(),
        SetPassword::Rejected
    );
    assert_eq!(
        adapter
            .set_password(&subject, "the miller keeps a second key")
            .await
            .unwrap(),
        SetPassword::Set
    );
    assert_eq!(server.open_sessions(), 0);
    assert_eq!(
        adapter
            .authenticate(&email("robin@example.test"), PASSWORD, caller)
            .await
            .unwrap(),
        Authentication::InvalidCredentials
    );
    assert!(matches!(
        adapter
            .authenticate(
                &email("robin@example.test"),
                "the miller keeps a second key",
                caller
            )
            .await
            .unwrap(),
        Authentication::Authenticated { .. }
    ));
}
