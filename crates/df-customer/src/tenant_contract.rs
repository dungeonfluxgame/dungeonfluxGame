//! Personal tenant lifecycle scenarios shared by the memory and PostgreSQL account stores.

use crate::{
    AccountStore, CreateOutcome, EmailAddress, IdentityProviderKind, MailPurpose,
    NewIdentityAccount, ResetConfirmOutcome, SignupOutcome, VerifyWithPasswordOutcome,
    contract::{Harness, NEW_PASSWORD, PASSWORD, START_MS, client},
};
use df_types::AccountId;

pub async fn verification_issues_a_distinct_stable_personal_tenant<S: AccountStore + Clone>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    assert_eq!(
        harness.sign_up("tenant@example.test").await,
        SignupOutcome::CheckYourEmail
    );
    let account = store
        .account_by_email("tenant@example.test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(harness.service.tenant_for(account.id).await.unwrap(), None);
    harness.mail.wait_for(1).await;
    let token = harness
        .mail
        .last_token("tenant@example.test", MailPurpose::VerifyEmail)
        .unwrap();
    assert!(matches!(
        harness
            .service
            .verify_email_and_set_password(&token, PASSWORD, PASSWORD, client(1))
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::Verified(_)
    ));
    let tenant = harness
        .service
        .tenant_for(account.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(tenant.as_bytes(), account.id.as_bytes());
    assert!(matches!(
        harness
            .service
            .verify_email_and_set_password(&token, PASSWORD, PASSWORD, client(1))
            .await
            .unwrap(),
        VerifyWithPasswordOutcome::AlreadyVerified
    ));
    assert_eq!(store.tenant_for(account.id).await.unwrap(), Some(tenant));
    let (caller, _) = harness.log_in("tenant@example.test", PASSWORD).await;
    assert!(matches!(
        harness
            .service
            .close(&caller, crate::Reauthentication::Password(PASSWORD))
            .await
            .unwrap(),
        crate::CloseResult::Closed { .. }
    ));
    assert_eq!(store.tenant_for(account.id).await.unwrap(), None);
    harness.log_in("tenant@example.test", PASSWORD).await;
    assert_eq!(store.tenant_for(account.id).await.unwrap(), Some(tenant));
}

pub async fn identity_signup_issues_a_tenant_only_for_verified_email<S: AccountStore + Clone>(
    store: S,
) {
    for (byte, is_verified) in [(71, false), (72, true)] {
        let account = AccountId::from_bytes(&[byte; 16]).unwrap();
        let input = NewIdentityAccount {
            id: account,
            email: EmailAddress::parse(&format!("identity{byte}@example.test")).unwrap(),
            is_verified,
            provider: IdentityProviderKind::WorkOs,
            subject: format!("identity{byte}"),
            created_at_ms: START_MS,
            adult_attested_at: Some(START_MS),
            terms_version: Some("terms-2026-10".to_owned()),
            terms_accepted_at: Some(START_MS),
            verification: None,
        };
        assert_eq!(
            store.create_identity_account(input.clone()).await.unwrap(),
            CreateOutcome::Created
        );
        let tenant = store.tenant_for(account).await.unwrap();
        assert_eq!(tenant.is_some(), is_verified);
        if let Some(tenant) = tenant {
            assert_ne!(tenant.as_bytes(), account.as_bytes());
        }
        assert_eq!(
            store.create_identity_account(input).await.unwrap(),
            CreateOutcome::EmailTaken
        );
        assert_eq!(store.tenant_for(account).await.unwrap(), tenant);
    }
}

pub async fn password_reset_proves_an_unverified_email_and_issues_one_tenant<
    S: AccountStore + Clone,
>(
    store: S,
) {
    let harness = Harness::start(store.clone()).await;
    harness.sign_up("reset-tenant@example.test").await;
    let account = store
        .account_by_email("reset-tenant@example.test")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(store.tenant_for(account.id).await.unwrap(), None);
    harness.mail.wait_for(1).await;
    harness
        .service
        .request_password_reset("reset-tenant@example.test", client(4))
        .await
        .unwrap();
    harness.mail.wait_for(2).await;
    let token = harness
        .mail
        .last_token("reset-tenant@example.test", MailPurpose::PasswordReset)
        .unwrap();
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, NEW_PASSWORD, NEW_PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::Reset
    );
    let tenant = store.tenant_for(account.id).await.unwrap().unwrap();
    assert_ne!(tenant.as_bytes(), account.id.as_bytes());
    assert_eq!(
        harness
            .service
            .confirm_password_reset(&token, PASSWORD, PASSWORD, client(4))
            .await
            .unwrap(),
        ResetConfirmOutcome::InvalidLink
    );
    assert_eq!(store.tenant_for(account.id).await.unwrap(), Some(tenant));
}

#[cfg(test)]
mod tests {
    use crate::fakes::MemoryAccountStore;

    #[tokio::test]
    async fn verification_issues_a_distinct_stable_personal_tenant() {
        super::verification_issues_a_distinct_stable_personal_tenant(MemoryAccountStore::default())
            .await;
    }

    #[tokio::test]
    async fn identity_signup_issues_a_tenant_only_for_verified_email() {
        super::identity_signup_issues_a_tenant_only_for_verified_email(
            MemoryAccountStore::default(),
        )
        .await;
    }

    #[tokio::test]
    async fn password_reset_proves_an_unverified_email_and_issues_one_tenant() {
        super::password_reset_proves_an_unverified_email_and_issues_one_tenant(
            MemoryAccountStore::default(),
        )
        .await;
    }
}
