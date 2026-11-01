use std::collections::HashMap;

use super::*;

fn environment() -> HashMap<String, String> {
    [
        ("DF_DRAFT_DELIVERY_ENABLED", "true".to_owned()),
        (
            "DF_MEDIA_OBJECT_ROOT",
            std::env::temp_dir().display().to_string(),
        ),
        (
            "DF_MEDIA_CATALOG_DATABASE_URL",
            "host=127.0.0.1 port=55471 dbname=draft_config user=catalog_reader".to_owned(),
        ),
        (
            "DF_DRAFT_DISCLOSURE_DATABASE_URL",
            "host=127.0.0.1 port=55471 dbname=draft_config user=disclosure_reader".to_owned(),
        ),
        ("DF_DRAFT_MAX_IMAGE_BYTES", "1024".to_owned()),
        ("DF_DRAFT_MAX_DELIVERY_BYTES", "4096".to_owned()),
        ("DF_DRAFT_OPERATION_TIMEOUT_MS", "2500".to_owned()),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value))
    .collect()
}

fn parse(values: &HashMap<String, String>) -> Result<Settings, ConfigError> {
    let accounts = "host=127.0.0.1 port=55471 dbname=draft_config user=accounts"
        .parse()
        .unwrap();
    Settings::parse(
        &|key| values.get(key).cloned(),
        &accounts,
        &PostgresTransport::PlaintextLoopbackOnly,
    )
}

#[test]
fn delivery_is_disabled_until_explicitly_configured() {
    assert!(matches!(parse(&HashMap::new()), Ok(Settings::Disabled)));
    let mut values = environment();
    values.insert("DF_DRAFT_DELIVERY_ENABLED".into(), "false".into());
    values.remove("DF_MEDIA_OBJECT_ROOT");
    assert!(matches!(parse(&values), Ok(Settings::Disabled)));
}

#[test]
fn enabled_delivery_requires_all_settings_and_finite_nonzero_bounds() {
    let values = environment();
    assert!(matches!(parse(&values), Ok(Settings::Enabled(_))));
    for key in values
        .keys()
        .filter(|key| key.as_str() != "DF_DRAFT_DELIVERY_ENABLED")
    {
        let mut incomplete = values.clone();
        incomplete.remove(key);
        assert!(matches!(parse(&incomplete), Err(ConfigError::Missing(found)) if found == key));
    }
    for key in [
        "DF_DRAFT_MAX_IMAGE_BYTES",
        "DF_DRAFT_MAX_DELIVERY_BYTES",
        "DF_DRAFT_OPERATION_TIMEOUT_MS",
    ] {
        for value in ["0", "-1", "bad"] {
            let mut invalid = values.clone();
            invalid.insert(key.into(), value.into());
            assert!(matches!(parse(&invalid), Err(ConfigError::Invalid(_))));
        }
    }
    let mut invalid = values.clone();
    invalid.insert("DF_DRAFT_MAX_DELIVERY_BYTES".into(), "100".into());
    assert!(matches!(
        parse(&invalid),
        Err(ConfigError::Invalid("DF_DRAFT_MAX_DELIVERY_BYTES"))
    ));
}

#[test]
fn delivery_requires_one_database_and_separate_disclosure_credentials() {
    for url in [
        "host=127.0.0.1 port=55471 dbname=other user=reader",
        "host=127.0.0.1 port=55472 dbname=draft_config user=reader",
        "host=localhost port=55471 dbname=draft_config user=reader",
        "host=127.0.0.1 port=55471 dbname=draft_config user=catalog_reader",
    ] {
        let mut invalid = environment();
        invalid.insert("DF_DRAFT_DISCLOSURE_DATABASE_URL".into(), url.into());
        assert!(matches!(
            parse(&invalid),
            Err(ConfigError::Invalid("DF_DRAFT_DISCLOSURE_DATABASE_URL"))
        ));
    }
    let mut invalid = environment();
    invalid.insert("DF_MEDIA_OBJECT_ROOT".into(), "relative/path".into());
    assert!(matches!(
        parse(&invalid),
        Err(ConfigError::Invalid("DF_MEDIA_OBJECT_ROOT"))
    ));
}
