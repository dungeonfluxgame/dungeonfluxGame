use super::{
    GenerateOptions,
    campaign::{CampaignRecord, generate_frame, next_chapter},
    campaign_store::{CampaignStore, StoreError, database},
    campaign_tables::CampaignTables,
};
use std::{fs, path::PathBuf};
fn fixture() -> (PathBuf, CampaignRecord) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/play-g4")
        .join(format!(
            "store-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    let tables = CampaignTables::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables"),
    )
    .unwrap();
    let mut record =
        CampaignRecord::new(generate_frame(&tables, 42, GenerateOptions::default()).unwrap())
            .unwrap();
    let plan = next_chapter(&tables, &record.frame, &record).unwrap();
    record
        .begin(
            plan,
            "df_gameplay_demo_test_ch01".into(),
            super::campaign::digest(b"synthetic"),
        )
        .unwrap();
    (root, record)
}
#[test]
fn atomic_storage_refuses_stale_writers_and_corrupt_digest_and_repairs_derived_index() {
    let (root, record) = fixture();
    let store = CampaignStore::new(&root, "test").unwrap();
    let digest = store.save(&record, None).unwrap();
    assert!(matches!(
        store.save(&record, None),
        Err(StoreError::Conflict)
    ));
    let before = fs::read(root.join("development/campaigns/test/campaign.json")).unwrap();
    assert_eq!(store.load().unwrap().unwrap().digest, digest);
    assert_eq!(
        fs::read(root.join("development/campaigns/test/campaign.json")).unwrap(),
        before
    );
    fs::remove_file(root.join("development/campaigns/test/chapters/01.json")).unwrap();
    store.load().unwrap();
    assert!(
        root.join("development/campaigns/test/chapters/01.json")
            .is_file()
    );
    let mut value: serde_json::Value = serde_json::from_slice(&before).unwrap();
    value["digest"] = "0".repeat(64).into();
    fs::write(
        root.join("development/campaigns/test/campaign.json"),
        super::campaign::canonical_bytes(&value).unwrap(),
    )
    .unwrap();
    assert!(matches!(store.load(), Err(StoreError::Corrupt("digest"))));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn os_lock_is_nonblocking_and_optimistic_conflict_is_checked_under_lock() {
    let (root, record) = fixture();
    let store = CampaignStore::new(&root, "test").unwrap();
    store.save(&record, None).unwrap();
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(root.join("development/campaigns/test/write.lock"))
        .unwrap();
    file.try_lock().unwrap();
    let (root2, record2) = (root.clone(), record.clone());
    let error = std::thread::spawn(move || {
        CampaignStore::new(&root2, "test")
            .unwrap()
            .save(&record2, Some("stale"))
    })
    .join()
    .unwrap()
    .unwrap_err();
    assert!(matches!(error, StoreError::Busy));
    drop(file);
    assert!(matches!(
        store.save(&record, Some("stale")),
        Err(StoreError::Conflict)
    ));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn database_identity_preserves_dash_and_underscore_and_postgres_bound() {
    assert_ne!(database("a-b", 1).unwrap(), database("a_b", 1).unwrap());
    assert_eq!(database(&"a".repeat(41), 10).unwrap().len(), 63);
    for id in ["", "a/b", "x.y", " space", "é"] {
        assert!(database(id, 1).is_err());
    }
    assert!(database(&"a".repeat(42), 1).is_err());
}

#[test]
fn self_consistent_malformed_frame_and_prior_binding_rewrites_are_refused() {
    let (root, record) = fixture();
    let store = CampaignStore::new(&root, "test").unwrap();
    let digest = store.save(&record, None).unwrap();
    let mut changed = record.clone();
    changed.frame.title = "Different campaign".into();
    changed.frame_digest =
        super::campaign::digest(&super::campaign::canonical_bytes(&changed.frame).unwrap());
    assert!(store.save(&changed, Some(&digest)).is_err());
    let mut changed = record.clone();
    changed.chapters[0].adventure_digest = "0".repeat(64);
    assert!(store.save(&changed, Some(&digest)).is_err());
    for act in [0, 99] {
        let mut changed = record.clone();
        changed.frame.outline[0].act = act;
        changed.frame_digest =
            super::campaign::digest(&super::campaign::canonical_bytes(&changed.frame).unwrap());
        assert!(changed.validate().is_err());
        let envelope = serde_json::json!({"digest":changed.digest().unwrap(),"record":changed});
        fs::write(
            root.join("development/campaigns/test/campaign.json"),
            super::campaign::canonical_bytes(&envelope).unwrap(),
        )
        .unwrap();
        assert!(store.load().is_err());
    }
    let mut changed = serde_json::to_value(&record).unwrap();
    changed["frame"]["twists"] = serde_json::json!([]);
    assert!(serde_json::from_value::<CampaignRecord>(changed).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn campaign_startup_resumes_exact_bytes_requires_fresh_next_database_and_recovers_pending_bootstrap()
 {
    use super::super::startup_configuration::{AdventureMode, AdventureSettings};
    use super::campaign_startup::select_campaign;
    use df_persistence::local_demo_scope::AdventureStartup;
    let (root, _) = fixture();
    let tables = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables");
    let settings = AdventureSettings {
        mode: AdventureMode::Campaign,
        seed: None,
        tables: tables.clone(),
        rules_mode: super::RulesMode::Standard,
        chapter_type: None,
    };
    let fresh = select_campaign(
        &settings,
        &root,
        "test".into(),
        Some(42),
        "df_gameplay_demo_test_ch01",
        AdventureStartup::Fresh,
    )
    .unwrap();
    let store = CampaignStore::new(&root, "test").unwrap();
    let before = fs::read(root.join("development/campaigns/test/campaign.json")).unwrap();
    let unavailable = AdventureSettings {
        tables: root.join("removed-tables"),
        seed: Some(999),
        ..settings
    };
    let resumed = select_campaign(
        &unavailable,
        &root,
        "test".into(),
        Some(999),
        "df_gameplay_demo_test_ch01",
        AdventureStartup::Stored(fresh.documents.clone()),
    )
    .unwrap();
    assert!(!resumed.fresh);
    assert_eq!(resumed.documents.pack, fresh.documents.pack);
    assert_eq!(
        fs::read(root.join("development/campaigns/test/campaign.json")).unwrap(),
        before
    );
    let pending = select_campaign(
        &unavailable,
        &root,
        "test".into(),
        None,
        "df_gameplay_demo_test_ch01",
        AdventureStartup::Fresh,
    )
    .unwrap();
    assert_eq!(pending.documents.record, fresh.documents.record);
    assert!(
        select_campaign(
            &unavailable,
            &root,
            "test".into(),
            None,
            "df_gameplay_demo_test_ch02",
            AdventureStartup::Fresh
        )
        .is_err()
    );
    let stored = store.load().unwrap().unwrap();
    let mut record = stored.record;
    let outcome = super::campaign::ChapterOutcome {
        result: super::campaign::ChapterResult::Victory,
        approach: super::campaign::Approach::Asked,
        member: "hero-one".into(),
        rested: true,
        destination_reached: true,
        antagonist_spared: true,
        campaign_time: 3600,
    };
    record.complete(1, outcome.clone()).unwrap();
    let completed = store.save(&record, Some(&stored.digest)).unwrap();
    let unchanged = fs::read(root.join("development/campaigns/test/campaign.json")).unwrap();
    assert!(!record.complete(1, outcome).unwrap());
    assert_eq!(record.digest().unwrap(), completed);
    assert_eq!(
        fs::read(root.join("development/campaigns/test/campaign.json")).unwrap(),
        unchanged
    );
    let settings = AdventureSettings {
        mode: AdventureMode::Campaign,
        seed: None,
        tables,
        rules_mode: super::RulesMode::Standard,
        chapter_type: None,
    };
    assert!(
        select_campaign(
            &settings,
            &root,
            "test".into(),
            None,
            "df_gameplay_demo_test_ch02",
            AdventureStartup::Stored(fresh.documents)
        )
        .is_err()
    );
    let next = select_campaign(
        &settings,
        &root,
        "test".into(),
        None,
        "df_gameplay_demo_test_ch02",
        AdventureStartup::Fresh,
    )
    .unwrap();
    assert!(next.fresh);
    assert_ne!(next.documents.pack, pending.documents.pack);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn partial_unbound_cache_recovers_but_divergent_cache_refuses() {
    use super::campaign_startup::create_campaign_cache;
    let (root, _) = fixture();
    let tables = CampaignTables::load(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables"),
    )
    .unwrap();
    let record =
        CampaignRecord::new(generate_frame(&tables, 42, GenerateOptions::default()).unwrap())
            .unwrap();
    let mut plan = next_chapter(&tables, &record.frame, &record).unwrap();
    let generated =
        super::campaign_render::generate_chapter(&tables, &record.frame, &mut plan).unwrap();
    let documents = generated.documents(true).unwrap();
    let directory = root.join("development/runs/orphan");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("pack.json"),
        &documents.pack[..documents.pack.len() / 2],
    )
    .unwrap();
    create_campaign_cache(&directory, &documents).unwrap();
    super::record::verify_cache(&directory, &documents).unwrap();
    create_campaign_cache(&directory, &documents).unwrap();
    fs::write(directory.join("pack.json"), b"divergent").unwrap();
    assert!(create_campaign_cache(&directory, &documents).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn startup_refuses_self_consistent_documents_with_swapped_campaign_identity() {
    use super::super::startup_configuration::{AdventureMode, AdventureSettings};
    use super::campaign_startup::{CampaignStartupError, select_campaign};
    use df_persistence::local_demo_scope::AdventureStartup;
    for (key, replacement) in [
        ("id", serde_json::json!("other")),
        ("chapter", serde_json::json!(2)),
        ("frame_sha256", serde_json::json!("0".repeat(64))),
    ] {
        let (root, _) = fixture();
        let settings = AdventureSettings {
            mode: AdventureMode::Campaign,
            seed: None,
            tables: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/adventure-tables"),
            rules_mode: super::RulesMode::Standard,
            chapter_type: None,
        };
        let fresh = select_campaign(
            &settings,
            &root,
            "test".into(),
            Some(42),
            "df_gameplay_demo_test_ch01",
            AdventureStartup::Fresh,
        )
        .unwrap();
        let mut documents = fresh.documents;
        let mut pack: serde_json::Value = serde_json::from_slice(&documents.pack).unwrap();
        pack["campaign"][key] = replacement;
        documents.pack = serde_json::to_vec_pretty(&pack).unwrap();
        let mut adventure: serde_json::Value = serde_json::from_slice(&documents.record).unwrap();
        adventure["campaign"] = pack["campaign"].clone();
        adventure["pack_sha256"] = super::campaign::digest(&documents.pack).into();
        adventure["expression"]["output_sha256"] = super::campaign::digest(
            &[documents.pack.as_slice(), documents.tuning.as_slice()].concat(),
        )
        .into();
        documents.record = serde_json::to_vec_pretty(&adventure).unwrap();
        super::record::reload(&documents).unwrap();
        let store = CampaignStore::new(&root, "test").unwrap();
        let mut record = store.load().unwrap().unwrap().record;
        record.chapters[0].adventure_digest = super::campaign::digest(&documents.record);
        let envelope = serde_json::json!({"digest":record.digest().unwrap(),"record":record});
        fs::write(
            root.join("development/campaigns/test/campaign.json"),
            super::campaign::canonical_bytes(&envelope).unwrap(),
        )
        .unwrap();
        let directory = root.join("development/runs/df_gameplay_demo_test_ch01");
        for (name, bytes) in [
            ("pack.json", &documents.pack),
            ("tuning.json", &documents.tuning),
            ("adventure.json", &documents.record),
        ] {
            fs::write(directory.join(name), bytes).unwrap();
        }
        assert!(matches!(
            select_campaign(
                &settings,
                &root,
                "test".into(),
                None,
                "df_gameplay_demo_test_ch01",
                AdventureStartup::Stored(documents)
            ),
            Err(super::startup::StartupError::Campaign(
                CampaignStartupError::Run("persisted campaign binding disagrees with authority")
            ))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
#[test]
fn storage_rejects_rewritten_established_history_and_malformed_index_bindings() {
    let (root, record) = fixture();
    let store = CampaignStore::new(&root, "test").unwrap();
    let digest = store.save(&record, None).unwrap();
    let mut completed = record.clone();
    completed
        .complete(
            1,
            super::campaign::ChapterOutcome {
                result: super::campaign::ChapterResult::Victory,
                approach: super::campaign::Approach::Asked,
                member: "hero".into(),
                rested: true,
                destination_reached: true,
                antagonist_spared: false,
                campaign_time: 3600,
            },
        )
        .unwrap();
    let completed_digest = store.save(&completed, Some(&digest)).unwrap();
    assert!(!completed.facts.is_empty());
    let mut changed = completed.clone();
    changed.facts.clear();
    assert!(store.save(&changed, Some(&completed_digest)).is_err());
    let mut changed = completed.clone();
    changed.clues[0].text.push_str(" rewritten");
    assert!(store.save(&changed, Some(&completed_digest)).is_err());
    for (database, adventure) in [
        ("wrong", "0".repeat(64)),
        ("df_gameplay_demo_test_ch01", "digest".into()),
    ] {
        let mut changed = record.clone();
        changed.chapters[0].run_database = database.into();
        changed.chapters[0].adventure_digest = adventure;
        let envelope = serde_json::json!({"digest":changed.digest().unwrap(),"record":changed});
        fs::write(
            root.join("development/campaigns/test/campaign.json"),
            super::campaign::canonical_bytes(&envelope).unwrap(),
        )
        .unwrap();
        assert!(store.load().is_err());
    }
    for mutate in [0, 1, 2] {
        let mut changed = record.clone();
        match mutate {
            0 => changed.chapters[0].plan.seed ^= 1,
            1 => changed.chapters[0].plan.outcome_digest_in = "1".repeat(64),
            _ => {
                changed.chapters[0]
                    .plan
                    .forced
                    .insert("location".into(), "missing".into());
            }
        }
        assert!(
            changed.validate().is_err(),
            "mutated semantic plan field {mutate}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}
