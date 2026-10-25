use std::collections::HashMap;

use super::*;

fn environment() -> HashMap<String, String> {
    [
        ("DF_PROFILE", "development"),
        ("DF_GAMEPLAY_PUBLIC_URL", "http://127.0.0.1:8099"),
        (
            "DF_PORTAL_DATABASE_URL",
            "host=127.0.0.1 port=55911 user=postgres dbname=hostedstack_config",
        ),
        ("DF_GAMEPLAY_DATABASE_TLS", "loopback-plaintext"),
        ("DF_PORTAL_BUNDLE_DIR", "artifacts/build/account-app-web"),
        (
            "DF_TRIAL_EMAIL_HMAC_KEY",
            "3333333333333333333333333333333333333333333333333333333333333333",
        ),
        (
            "DF_CAMPAIGN_IP_KEY",
            "1111111111111111111111111111111111111111111111111111111111111111",
        ),
        (
            "DF_CAMPAIGN_CODE_KEY",
            "2222222222222222222222222222222222222222222222222222222222222222",
        ),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect()
}

fn parse(values: &HashMap<String, String>) -> Result<Config, ConfigError> {
    Config::parse(|key| values.get(key).cloned())
}

#[test]
fn development_defaults_to_local_identity_and_outbox() {
    let parsed = parse(&environment()).expect("development config");
    assert!(matches!(parsed.identity, Identity::Local));
    assert!(matches!(parsed.mail, Mail::Outbox(_)));
}

#[test]
fn demo_refuses_missing_real_mail_and_partial_workos_settings() {
    let mut values = environment();
    values.insert("DF_PROFILE".to_owned(), "demo".to_owned());
    values.insert(
        "DF_GAMEPLAY_PUBLIC_URL".to_owned(),
        "https://dungeonfluxdnd.com".to_owned(),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Missing(
            "DF_WORKOS_CLIENT_ID/DF_WORKOS_API_KEY"
        ))
    ));
    values.insert(
        "DF_WORKOS_CLIENT_ID".to_owned(),
        "client_test_fixture".to_owned(),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid(
            "DF_WORKOS_CLIENT_ID/DF_WORKOS_API_KEY"
        ))
    ));
    values.insert("DF_WORKOS_API_KEY".to_owned(), "fixture-only".to_owned());
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Missing("DF_RESEND_API_KEY"))
    ));
}

#[test]
fn nondevelopment_profiles_refuse_fake_ports_and_development_controls() {
    for profile in ["demo", "production"] {
        for name in [
            "DF_PORTAL_FAKE_PORTS",
            "DF_PORTAL_FAKE_CHECKOUT",
            "DF_DEV_ACCOUNTS",
            "DF_GAMEPLAY_DEV_CONTROL",
            "DF_PORTAL_OUTBOX",
        ] {
            let mut values = environment();
            values.insert("DF_PROFILE".to_owned(), profile.to_owned());
            values.insert(name.to_owned(), "on".to_owned());
            assert!(
                matches!(parse(&values), Err(ConfigError::DevelopmentOnly(found)) if found == name)
            );
        }
    }
}

#[test]
fn remote_plaintext_and_public_listeners_are_refused() {
    let mut values = environment();
    values.insert(
        "DF_PORTAL_DATABASE_URL".to_owned(),
        "host=db.example.test user=portal dbname=portal".to_owned(),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid("DF_GAMEPLAY_DATABASE_TLS"))
    ));
    let mut values = environment();
    for address in ["0.0.0.0:8099", "127.0.0.1:63309", "127.0.0.1:0"] {
        values.insert("DF_PORTAL_LISTEN".to_owned(), address.to_owned());
        assert!(matches!(
            parse(&values),
            Err(ConfigError::Invalid("DF_PORTAL_LISTEN"))
        ));
    }
}

#[test]
fn production_requires_verified_tls_and_provider_selections_must_agree() {
    let mut values = environment();
    values.extend(
        [
            ("DF_PROFILE", "production"),
            ("DF_GAMEPLAY_PUBLIC_URL", "https://dungeonfluxdnd.com"),
            ("DF_WORKOS_CLIENT_ID", "client_fixture"),
            ("DF_WORKOS_API_KEY", "fixture-only"),
            ("DF_RESEND_API_KEY", "fixture-only"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned())),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid("DF_GAMEPLAY_DATABASE_TLS"))
    ));
    values.insert(
        "DF_GAMEPLAY_DATABASE_TLS".to_owned(),
        "verify-full".to_owned(),
    );
    values.insert(
        "DF_PORTAL_DATABASE_CA_FILE".to_owned(),
        "artifacts/tmp/test-ca.pem".to_owned(),
    );
    assert!(parse(&values).is_ok());
    values.insert("DF_GAMEPLAY_IDENTITY".to_owned(), "local".to_owned());
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid("DF_GAMEPLAY_IDENTITY"))
    ));
}

#[test]
fn live_payment_keys_fail_closed_and_paired_test_keys_are_accepted() {
    let mut values = environment();
    values.insert(
        "DF_STRIPE_SECRET_KEY".to_owned(),
        "sk_live_fixture_only".to_owned(),
    );
    values.insert(
        "DF_STRIPE_WEBHOOK_SECRET".to_owned(),
        "whsec_fixture_only".to_owned(),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid("DF_STRIPE_SECRET_KEY"))
    ));
    values.insert(
        "DF_STRIPE_SECRET_KEY".to_owned(),
        "sk_test_fixture_only".to_owned(),
    );
    assert!(parse(&values).is_ok());
}

#[test]
fn demo_accepts_test_checkout_settings_with_real_identity_and_mail_configuration() {
    let mut values = environment();
    for (key, value) in [
        ("DF_PROFILE", "demo"),
        ("DF_GAMEPLAY_PUBLIC_URL", "https://dungeonfluxdnd.com"),
        ("DF_WORKOS_CLIENT_ID", "client_fixture"),
        ("DF_WORKOS_API_KEY", "fixture_only"),
        ("DF_RESEND_API_KEY", "fixture_only"),
        ("DF_STRIPE_SECRET_KEY", "sk_test_fixture_only"),
        ("DF_STRIPE_WEBHOOK_SECRET", "whsec_fixture_only"),
    ] {
        values.insert(key.to_owned(), value.to_owned());
    }
    assert!(parse(&values).unwrap().stripe.is_some());
}

#[test]
fn production_refuses_fixture_builds_and_demo_accepts_only_its_explicit_exception() {
    let mut values = environment();
    values.insert("DF_PROFILE".to_owned(), "production".to_owned());
    for fixture in ["on", "demo-exception:df-transport-fixture --gameplay-demo"] {
        values.insert("DF_FIXTURE_BUILD".to_owned(), fixture.to_owned());
        assert!(matches!(
            parse(&values),
            Err(ConfigError::DevelopmentOnly("DF_FIXTURE_BUILD"))
        ));
    }
    values.insert("DF_PROFILE".to_owned(), "demo".to_owned());
    values.insert("DF_FIXTURE_BUILD".to_owned(), "on".to_owned());
    assert!(matches!(
        parse(&values),
        Err(ConfigError::DevelopmentOnly("DF_FIXTURE_BUILD"))
    ));
    values.insert(
        "DF_FIXTURE_BUILD".to_owned(),
        "demo-exception:df-transport-fixture --gameplay-demo".to_owned(),
    );
    assert!(!matches!(
        parse(&values),
        Err(ConfigError::DevelopmentOnly("DF_FIXTURE_BUILD"))
    ));
}

#[test]
fn proxy_network_configuration_is_validated_before_serving() {
    let mut values = environment();
    values.insert(
        "DF_PORTAL_TRUSTED_PROXIES".to_owned(),
        "127.0.0.1/99".to_owned(),
    );
    assert!(matches!(
        parse(&values),
        Err(ConfigError::Invalid("DF_PORTAL_TRUSTED_PROXIES"))
    ));
    values.insert(
        "DF_PORTAL_TRUSTED_PROXIES".to_owned(),
        "127.0.0.1/32,::1/128".to_owned(),
    );
    assert!(parse(&values).is_ok());
}

#[test]
fn development_profile_uses_the_existing_identity_and_mail_spellings() {
    let mut values = environment();
    values.insert(
        "DF_GAMEPLAY_IDENTITY".to_owned(),
        "local-password".to_owned(),
    );
    values.insert("DF_GAMEPLAY_MAIL_MODE".to_owned(), "dev-outbox".to_owned());
    assert!(parse(&values).is_ok());
}
