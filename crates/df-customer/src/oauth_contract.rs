//! Shared Google account/attempt scenarios for memory and PostgreSQL, with no provider calls.

use crate::{
    AccountStore, Authenticated, CloseResult, EmailAddress, GoogleStore, OAuthAuthorize, OAuthCall,
    OAuthExchange, OAuthIdentity, OAuthIntent, OAuthOutcome, OAuthProvider, OAuthRefusal,
    OAuthSecret, OAuthStart, OAuthStartForm, Reauthentication,
    contract::{Harness, PASSWORD, client},
};

struct Code {
    challenge: [u8; 32],
    nonce: String,
    require_fresh: bool,
    expires_at_ms: u64,
}

struct Provider {
    email: &'static str,
    clock: crate::FakeClock,
    codes: std::sync::Mutex<std::collections::HashMap<String, Code>>,
}

impl OAuthProvider for Provider {
    fn redirect_uri(&self) -> &str {
        "http://127.0.0.1:8099/app/api/oauth/google/callback"
    }
    fn authorize(&self, request: OAuthAuthorize<'_>) -> Result<OAuthSecret, crate::OAuthFailure> {
        use crate::Clock;
        let now_ms = self.clock.now_ms();
        let mut codes = self.codes.lock().unwrap();
        codes.retain(|_, code| code.expires_at_ms > now_ms);
        if codes.len() >= 128 {
            return Err(crate::OAuthFailure::Unavailable);
        }
        codes.insert(
            request.state.expose().to_owned(),
            Code {
                challenge: request.challenge,
                nonce: request.nonce.expose().to_owned(),
                require_fresh: request.require_fresh,
                expires_at_ms: now_ms + 600_000,
            },
        );
        Ok(OAuthSecret::new(request.state.expose().to_owned()))
    }
    fn exchange<'a>(&'a self, request: OAuthExchange<'a>) -> OAuthCall<'a> {
        Box::pin(async move {
            let code = self
                .codes
                .lock()
                .unwrap()
                .remove(request.code.expose())
                .ok_or(crate::OAuthFailure::Rejected)?;
            if code.challenge != crate::pkce_challenge(request.verifier)
                || code.nonce != request.nonce.expose()
                || code.require_fresh != request.require_fresh
                || code.expires_at_ms <= request.now_s.saturating_mul(1000)
            {
                return Err(crate::OAuthFailure::Rejected);
            }
            Ok(OAuthIdentity {
                subject: format!("subject-{}", self.email),
                email: EmailAddress::parse(self.email).unwrap(),
                authenticated_at_s: request.require_fresh.then_some(request.now_s),
            })
        })
    }
}

async fn harness<S: AccountStore + GoogleStore + Clone>(
    store: S,
    email: &'static str,
) -> Harness<S> {
    let mut harness = Harness::start(store.clone()).await;
    let provider = Provider {
        email,
        clock: harness.clock.clone(),
        codes: std::sync::Mutex::default(),
    };
    harness.service = harness.service.with_google(provider, store);
    harness
}

async fn begin<S: AccountStore + GoogleStore + Clone>(
    harness: &Harness<S>,
    intent: OAuthIntent,
    caller: Option<&Authenticated>,
) -> OAuthStart {
    harness
        .service
        .start_google(
            OAuthStartForm {
                intent,
                adult_attested: true,
                terms_version: harness.service.current_terms_version(),
                current_password: PASSWORD,
            },
            caller,
            client(12),
        )
        .await
        .unwrap()
        .unwrap_or_else(|reason| panic!("start refused: {reason:?}"))
}

async fn finish<S: AccountStore + GoogleStore + Clone>(
    harness: &Harness<S>,
    start: &OAuthStart,
    caller: Option<&Authenticated>,
) -> OAuthOutcome {
    harness
        .service
        .finish_google(
            start.authorization_url.expose(),
            &start.cookie.text(),
            Some(start.authorization_url.expose()),
            false,
            caller,
            client(12),
        )
        .await
        .unwrap()
}

pub async fn new_returning_and_single_use<S: AccountStore + GoogleStore + Clone>(store: S) {
    let harness = harness(store.clone(), "google-new@example.test").await;
    let unknown = begin(&harness, OAuthIntent::SignIn, None).await;
    assert!(matches!(
        finish(&harness, &unknown, None).await,
        OAuthOutcome::Refused(OAuthRefusal::NoAccount)
    ));
    let start = begin(&harness, OAuthIntent::SignUp, None).await;
    assert!(matches!(
        harness
            .service
            .finish_google(
                start.authorization_url.expose(),
                &crate::SecretToken::generate().unwrap().text(),
                Some("code"),
                false,
                None,
                client(12)
            )
            .await
            .unwrap(),
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
    let OAuthOutcome::SignedUp(session) = finish(&harness, &start, None).await else {
        panic!("Google signup must sign in");
    };
    let caller = harness
        .service
        .authenticate(&session.token.text())
        .await
        .unwrap()
        .unwrap();
    assert!(caller.account.is_verified);
    assert!(harness.mail.messages().is_empty());
    assert!(store.is_linked(caller.account.id).await.unwrap());
    assert!(matches!(
        finish(&harness, &start, None).await,
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
    let replay = begin(&harness, OAuthIntent::SignIn, None).await;
    let sessions_before = store
        .sessions(caller.account.id, crate::contract::START_MS, 0)
        .await
        .unwrap()
        .len();
    assert!(matches!(
        harness
            .service
            .finish_google(
                replay.authorization_url.expose(),
                &replay.cookie.text(),
                Some(start.authorization_url.expose()),
                false,
                None,
                client(12)
            )
            .await
            .unwrap(),
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
    assert_eq!(
        store
            .sessions(caller.account.id, crate::contract::START_MS, 0)
            .await
            .unwrap()
            .len(),
        sessions_before
    );
    assert!(matches!(
        finish(&harness, &replay, None).await,
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
    let returning = begin(&harness, OAuthIntent::SignIn, None).await;
    let OAuthOutcome::SignedIn(returned) = finish(&harness, &returning, None).await else {
        panic!("returning Google signin");
    };
    let returned = harness
        .service
        .authenticate(&returned.token.text())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.account.id, caller.account.id);
    assert_eq!(
        harness
            .service
            .unlink_google(&caller, PASSWORD)
            .await
            .unwrap(),
        Err(OAuthRefusal::LastSignInMethod)
    );
    assert_eq!(
        harness
            .service
            .close(&caller, Reauthentication::RecentSignIn)
            .await
            .unwrap(),
        CloseResult::NotReauthenticated
    );
    let fresh = begin(&harness, OAuthIntent::Reauthenticate, Some(&caller)).await;
    let OAuthOutcome::SignedIn(fresh) = finish(&harness, &fresh, Some(&caller)).await else {
        panic!("fresh Google authentication");
    };
    let fresh = harness
        .service
        .authenticate(&fresh.token.text())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        harness
            .service
            .export(&fresh, Reauthentication::RecentSignIn, client(12))
            .await
            .unwrap(),
        crate::ExportResult::Document(_)
    ));
    let expired = begin(&harness, OAuthIntent::SignIn, None).await;
    harness.clock.advance_ms(600_000);
    assert!(matches!(
        finish(&harness, &expired, None).await,
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
}

pub async fn email_collision_link_and_unlink<S: AccountStore + GoogleStore + Clone>(store: S) {
    let harness = harness(store.clone(), "google-password@example.test").await;
    let (caller, _) = harness
        .verified_account("google-password@example.test")
        .await;
    let collision = begin(&harness, OAuthIntent::SignUp, None).await;
    assert!(matches!(
        finish(&harness, &collision, None).await,
        OAuthOutcome::Refused(OAuthRefusal::PasswordSignInRequired)
    ));
    assert!(!store.is_linked(caller.account.id).await.unwrap());
    let link = begin(&harness, OAuthIntent::Link, Some(&caller)).await;
    assert!(matches!(
        finish(&harness, &link, Some(&caller)).await,
        OAuthOutcome::Linked
    ));
    assert!(store.is_linked(caller.account.id).await.unwrap());
    assert!(matches!(
        finish(&harness, &link, Some(&caller)).await,
        OAuthOutcome::Refused(OAuthRefusal::Expired)
    ));
    assert_eq!(
        harness
            .service
            .unlink_google(&caller, PASSWORD)
            .await
            .unwrap(),
        Ok(())
    );
    assert!(!store.is_linked(caller.account.id).await.unwrap());
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn an_attempt_cannot_exchange_after_the_configured_callback_origin_changes() {
        struct OtherOrigin;
        impl crate::OAuthProvider for OtherOrigin {
            fn redirect_uri(&self) -> &str {
                "http://127.0.0.1:8100/app/api/oauth/google/callback"
            }
            fn authorize(
                &self,
                _request: crate::OAuthAuthorize<'_>,
            ) -> Result<crate::OAuthSecret, crate::OAuthFailure> {
                Err(crate::OAuthFailure::Unavailable)
            }
            fn exchange<'a>(&'a self, _request: crate::OAuthExchange<'a>) -> crate::OAuthCall<'a> {
                Box::pin(async { Err(crate::OAuthFailure::Unavailable) })
            }
        }
        let store = crate::fakes::MemoryAccountStore::default();
        let mut harness = super::harness(store.clone(), "origin@example.test").await;
        let start = super::begin(&harness, crate::OAuthIntent::SignUp, None).await;
        harness.service = harness.service.with_google(OtherOrigin, store);
        assert!(matches!(
            super::finish(&harness, &start, None).await,
            crate::OAuthOutcome::Refused(crate::OAuthRefusal::Expired)
        ));
        assert_eq!(harness.service.dev_summary().await.unwrap().active, 0);
    }

    #[tokio::test]
    async fn callback_throttle_preserves_the_attempt_and_allows_a_changed_client_network() {
        let store = crate::fakes::MemoryAccountStore::default();
        let harness = super::harness(store, "throttle@example.test").await;
        let start = super::begin(&harness, crate::OAuthIntent::SignUp, None).await;
        let mut throttled = false;
        for _ in 0..1024 {
            let outcome = harness
                .service
                .finish_google(
                    "invalid",
                    "",
                    None,
                    false,
                    None,
                    crate::contract::client(13),
                )
                .await
                .unwrap();
            if matches!(
                outcome,
                crate::OAuthOutcome::Refused(crate::OAuthRefusal::Throttled)
            ) {
                throttled = true;
                break;
            }
        }
        assert!(throttled, "callback IP bucket must be bounded");
        assert!(matches!(
            harness
                .service
                .finish_google(
                    start.authorization_url.expose(),
                    &start.cookie.text(),
                    Some(start.authorization_url.expose()),
                    false,
                    None,
                    crate::contract::client(13)
                )
                .await
                .unwrap(),
            crate::OAuthOutcome::Refused(crate::OAuthRefusal::Throttled)
        ));
        assert!(matches!(
            harness
                .service
                .finish_google(
                    start.authorization_url.expose(),
                    &start.cookie.text(),
                    Some(start.authorization_url.expose()),
                    false,
                    None,
                    crate::contract::client(14)
                )
                .await
                .unwrap(),
            crate::OAuthOutcome::SignedUp(_)
        ));
    }

    #[tokio::test]
    async fn a_nearly_expired_google_authentication_does_not_gain_a_new_five_minute_window() {
        use crate::{CloseResult, OAuthIntent, OAuthOutcome, OAuthProvider, Reauthentication};
        struct Aged;
        impl OAuthProvider for Aged {
            fn redirect_uri(&self) -> &str {
                "http://127.0.0.1:8099/app/api/oauth/google/callback"
            }
            fn authorize(
                &self,
                request: crate::OAuthAuthorize<'_>,
            ) -> Result<crate::OAuthSecret, crate::OAuthFailure> {
                Ok(crate::OAuthSecret::new(request.state.expose().to_owned()))
            }
            fn exchange<'a>(&'a self, request: crate::OAuthExchange<'a>) -> crate::OAuthCall<'a> {
                Box::pin(async move {
                    Ok(crate::OAuthIdentity {
                        subject: "subject-expiry@example.test".to_owned(),
                        email: crate::EmailAddress::parse("expiry@example.test").unwrap(),
                        authenticated_at_s: Some(request.now_s - 299),
                    })
                })
            }
        }
        let store = crate::fakes::MemoryAccountStore::default();
        let mut harness = super::harness(store.clone(), "expiry@example.test").await;
        let signup = super::begin(&harness, OAuthIntent::SignUp, None).await;
        let OAuthOutcome::SignedUp(issued) = super::finish(&harness, &signup, None).await else {
            panic!("signup");
        };
        let caller = harness
            .service
            .authenticate(&issued.token.text())
            .await
            .unwrap()
            .unwrap();
        let disabled = harness.service.clone().with_google_store(store.clone());
        assert!(!disabled.google_enabled());
        assert_eq!(
            disabled
                .close(&caller, Reauthentication::RecentSignIn)
                .await
                .unwrap(),
            CloseResult::NotReauthenticated
        );
        harness.service = harness.service.with_google(Aged, store);
        let start = super::begin(&harness, OAuthIntent::Reauthenticate, Some(&caller)).await;
        let OAuthOutcome::SignedIn(fresh) = super::finish(&harness, &start, Some(&caller)).await
        else {
            panic!("reauthenticate");
        };
        let fresh = harness
            .service
            .authenticate(&fresh.token.text())
            .await
            .unwrap()
            .unwrap();
        assert!(
            harness
                .service
                .has_fresh_google_proof(&fresh)
                .await
                .unwrap()
        );
        harness.clock.advance_ms(1001);
        assert!(
            !harness
                .service
                .has_fresh_google_proof(&fresh)
                .await
                .unwrap()
        );
        assert_eq!(
            harness
                .service
                .close(&fresh, Reauthentication::RecentSignIn)
                .await
                .unwrap(),
            CloseResult::NotReauthenticated
        );
    }

    #[tokio::test]
    async fn new_google_signup_returning_user_replay_expiry_and_fresh_proof() {
        use crate::AccountStore;
        let store = crate::fakes::MemoryAccountStore::default();
        super::new_returning_and_single_use(store.clone()).await;
        let account = store
            .account_by_email("google-new@example.test")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            store.google_consent(account.id),
            Some((
                crate::contract::START_MS,
                "terms-2026-10".to_owned(),
                crate::contract::START_MS
            ))
        );
    }
    #[tokio::test]
    async fn password_email_collision_requires_explicit_link_and_allows_unlink() {
        super::email_collision_link_and_unlink(crate::fakes::MemoryAccountStore::default()).await;
    }

    #[tokio::test]
    async fn signup_refuses_missing_adult_or_outdated_terms_without_an_account() {
        let store = crate::fakes::MemoryAccountStore::default();
        let harness = super::harness(store, "consent@example.test").await;
        for (adult, terms) in [(false, "terms-2026-10"), (true, "old")] {
            let outcome = harness
                .service
                .start_google(
                    crate::OAuthStartForm {
                        intent: crate::OAuthIntent::SignUp,
                        adult_attested: adult,
                        terms_version: terms,
                        current_password: "",
                    },
                    None,
                    crate::contract::client(1),
                )
                .await
                .unwrap();
            assert!(matches!(outcome, Err(crate::OAuthRefusal::ConsentRequired)));
        }
        assert_eq!(harness.service.dev_summary().await.unwrap().active, 0);
    }

    #[tokio::test]
    async fn reauthentication_refuses_a_session_revoked_while_google_is_responding() {
        use crate::{AccountStore, OAuthProvider};
        struct Blocked {
            started: std::sync::Arc<tokio::sync::Notify>,
            release: std::sync::Arc<tokio::sync::Notify>,
        }
        impl OAuthProvider for Blocked {
            fn redirect_uri(&self) -> &str {
                "http://127.0.0.1:8099/app/api/oauth/google/callback"
            }
            fn authorize(
                &self,
                request: crate::OAuthAuthorize<'_>,
            ) -> Result<crate::OAuthSecret, crate::OAuthFailure> {
                Ok(crate::OAuthSecret::new(request.state.expose().to_owned()))
            }
            fn exchange<'a>(&'a self, request: crate::OAuthExchange<'a>) -> crate::OAuthCall<'a> {
                Box::pin(async move {
                    self.started.notify_one();
                    self.release.notified().await;
                    Ok(crate::OAuthIdentity {
                        subject: "subject-revoked@example.test".to_owned(),
                        email: crate::EmailAddress::parse("revoked@example.test").unwrap(),
                        authenticated_at_s: Some(request.now_s),
                    })
                })
            }
        }
        let store = crate::fakes::MemoryAccountStore::default();
        let mut harness = super::harness(store.clone(), "revoked@example.test").await;
        let signup = super::begin(&harness, crate::OAuthIntent::SignUp, None).await;
        let crate::OAuthOutcome::SignedUp(issued) = super::finish(&harness, &signup, None).await
        else {
            panic!("signup");
        };
        let caller = harness
            .service
            .authenticate(&issued.token.text())
            .await
            .unwrap()
            .unwrap();
        let started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        harness.service = harness.service.with_google(
            Blocked {
                started: started.clone(),
                release: release.clone(),
            },
            store.clone(),
        );
        let start = super::begin(&harness, crate::OAuthIntent::Reauthenticate, Some(&caller)).await;
        let service = harness.service.clone();
        let caller_for_request = caller.clone();
        let task = tokio::spawn(async move {
            service
                .finish_google(
                    start.authorization_url.expose(),
                    &start.cookie.text(),
                    Some("code"),
                    false,
                    Some(&caller_for_request),
                    crate::contract::client(1),
                )
                .await
        });
        started.notified().await;
        store
            .revoke_sessions(
                caller.account.id,
                crate::RevokeScope::One(caller.session),
                crate::RevokeReason::Logout,
                crate::contract::START_MS,
                None,
            )
            .await
            .unwrap();
        release.notify_one();
        assert!(matches!(
            task.await.unwrap().unwrap(),
            crate::OAuthOutcome::Refused(crate::OAuthRefusal::ReauthenticationRequired)
        ));
    }
}
