use std::{collections::BTreeMap, fs, path::PathBuf};

const REGISTRY_SOURCES: [(&str, &str); 13] = [
    (
        "df-hosted database",
        include_str!("../../df-hosted/src/database.rs"),
    ),
    (
        "runtime support fixture",
        include_str!("runtime_support/mod.rs"),
    ),
    (
        "account store",
        include_str!("../src/account_store/pool.rs"),
    ),
    ("save store", include_str!("../src/save_store/postgres.rs")),
    ("media catalog", include_str!("../src/media_catalog.rs")),
    ("story memory", include_str!("../src/story_memory.rs")),
    (
        "local demo scope",
        include_str!("../src/local_demo_scope.rs"),
    ),
    (
        "local demo startup",
        include_str!("../src/local_demo_scope/startup.rs"),
    ),
    (
        "support store fixture",
        include_str!("../src/support_store/tests.rs"),
    ),
    (
        "finance fixture",
        include_str!("../../df-finance/tests/support/mod.rs"),
    ),
    (
        "finance host funding fixture",
        include_str!("../../df-finance/tests/host_funding.rs"),
    ),
    (
        "hosted campaign fixture",
        include_str!("../../df-hosted/src/campaign_sessions.rs"),
    ),
    (
        "campaign postgres fixtures",
        include_str!("../../df-campaign/tests/postgres/fixture.rs"),
    ),
];

#[test]
fn runtime_support_registry_includes_applicable_new_migrations_in_order() {
    let registry = include_str!("runtime_support/mod.rs");
    let expected = [
        "0025_finance_refunds_disputes_boundary.sql",
        "0028_save_points.sql",
        "0029_google_oauth.sql",
        "0030_personal_tenants.sql",
        "0031_host_funding.sql",
    ];
    let positions = expected
        .iter()
        .map(|file| registry.find(file))
        .collect::<Option<Vec<_>>>();
    assert!(positions.is_some(), "a required migration is missing");
    let Some(positions) = positions else {
        return;
    };
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "runtime support migrations must be listed in version order"
    );
}

#[test]
fn new_migrations_are_registered_in_their_owner_ledgers() {
    let expected = [
        (
            "df-hosted database",
            "0025_finance_refunds_disputes_boundary.sql",
        ),
        (
            "runtime support fixture",
            "0025_finance_refunds_disputes_boundary.sql",
        ),
        (
            "finance fixture",
            "0025_finance_refunds_disputes_boundary.sql",
        ),
        ("support store fixture", "0027_support_inbox.sql"),
        ("runtime support fixture", "0028_save_points.sql"),
        ("save store", "0028_save_points.sql"),
        ("runtime support fixture", "0029_google_oauth.sql"),
        ("account store", "0029_google_oauth.sql"),
        ("runtime support fixture", "0030_personal_tenants.sql"),
        ("account store", "0030_personal_tenants.sql"),
        ("df-hosted database", "0031_host_funding.sql"),
        ("runtime support fixture", "0031_host_funding.sql"),
        ("finance host funding fixture", "0031_host_funding.sql"),
    ];

    for (ledger, file) in expected {
        let source = REGISTRY_SOURCES
            .iter()
            .find(|(name, _)| *name == ledger)
            .map(|(_, source)| source);
        assert!(source.is_some(), "missing registry source {ledger}");
        let Some(source) = source else {
            return;
        };
        assert_eq!(
            source.matches(file).count(),
            1,
            "{file} must be registered once in the {ledger}"
        );
    }
}

#[test]
fn every_persistence_migration_has_one_registration_per_ledger() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let migrations = manifest.join("migrations");
    let mut files = fs::read_dir(&migrations)
        .expect("migration directory")
        .map(|entry| {
            entry
                .expect("migration directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|file| file.ends_with(".sql"))
        .collect::<Vec<_>>();
    files.sort();

    let mut registered_by = BTreeMap::<String, Vec<&str>>::new();
    for file in &files {
        for (ledger, source) in REGISTRY_SOURCES {
            let count = source.matches(file).count();
            assert!(
                count <= 1,
                "{file} is registered {count} times in the {ledger} source"
            );
            if count == 1 {
                registered_by.entry(file.clone()).or_default().push(ledger);
            }
        }
    }

    let unregistered = files
        .iter()
        .filter(|file| !registered_by.contains_key(*file))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        unregistered.is_empty(),
        "migration files without a ledger registration: {unregistered:?}"
    );
}
