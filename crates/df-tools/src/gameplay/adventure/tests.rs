use super::*;
fn tables() -> Tables {
    Tables::load(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap()
}
#[test]
fn fixed_seeds_reproduce_current_settings_rolls_and_documents() {
    let tables = tables();
    for seed in [0x3c6ef378dd81de97, 0xdaa66d325ccc5aac, 0x11a25cdccc7d808f] {
        let first = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        let second = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        assert_eq!(first.pack, second.pack);
        assert_eq!(first.tuning, second.tuning);
        assert_eq!(first.rolls, second.rolls);
        assert_eq!(first.settings, second.settings);
        let a = first.documents(true).unwrap();
        let b = second.documents(true).unwrap();
        assert_eq!(a.pack, b.pack);
        assert_eq!(a.tuning, b.tuning);
        assert_eq!(a.record, b.record);
    }
}
#[test]
fn five_thousand_standard_seeds_validate() {
    let tables = tables();
    for seed in 0..5000 {
        let adventure = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let creature = tables
            .antagonists
            .entries
            .iter()
            .find(|e| e.id == adventure.facts["ant.id"])
            .unwrap();
        assert_eq!(creature.fidelity, "exact");
        let row = df_rules::SRD_521_CREATURES
            .iter()
            .find(|c| c.id == creature.id)
            .unwrap();
        assert!(row.exact);
        let a = adventure.tuning.bandit.abilities;
        assert_eq!(
            [
                a.strength,
                a.dexterity,
                a.constitution,
                a.intelligence,
                a.wisdom,
                a.charisma
            ],
            row.rules.abilities
        );
        assert_eq!(adventure.tuning.bandit.hit_points, row.rules.hit_points);
        assert_eq!(adventure.tuning.bandit.armor_class, row.rules.armor_class);
        assert_eq!(
            i32::from(adventure.tuning.bandit.attack_bonus),
            row.rules.attack_bonus
        );
        assert_eq!(
            i32::from(adventure.tuning.bandit.damage_modifier),
            row.rules.damage_modifier
        );
        assert_eq!(
            adventure.tuning.bandit.weapon.damage_dice,
            row.rules.weapon.damage_dice
        );
        assert_eq!(
            adventure.tuning.bandit.weapon.damage_sides,
            row.rules.weapon.damage_sides
        );
        assert_eq!(adventure.tuning.bandit.weapon.item, row.weapon_label);
        for sheet in &adventure.fact_sheet {
            assert!(sheet.baseline.chars().count() <= sheet.bounds);
            assert!(!sheet.baseline.contains(['{', '}']));
        }
        let text = serde_json::to_string(&adventure.pack).unwrap();
        for old in [
            "Dockside Bandit",
            "Harbor Inn",
            "Lantern Wharf",
            "sealed packet",
        ] {
            assert!(!text.contains(old), "seed {seed}: {old}");
        }
    }
}
#[test]
fn forced_entries_are_checked_and_recorded() {
    let tables = tables();
    let forced = ForcedEntries::from([("location".into(), "pinewood-trail".into())]);
    let adventure = generate(&tables, 1, GenerateOptions::default(), &forced).unwrap();
    assert_eq!(adventure.rolls[0].status, RollStatus::Forced);
    assert_eq!(adventure.rolls[0].natural, 0);
    let forced = ForcedEntries::from([("location".into(), "unknown".into())]);
    assert!(generate(&tables, 1, GenerateOptions::default(), &forced).is_err());
}

#[test]
fn longest_coherent_opening_fits_the_narration_bound() {
    let tables = tables();
    let forced = ForcedEntries::from([
        ("location".into(), "pinewood-trail".into()),
        ("patron".into(), "acolyte".into()),
        ("hook".into(), "pilgrim-vow".into()),
        ("stake".into(), "steady-lantern".into()),
        ("antagonist".into(), "skeleton".into()),
        (
            "prose.opening_narration".into(),
            tables.prose["opening_narration"].variants[1].text.clone(),
        ),
        (
            "prose.foreshadow_undead".into(),
            tables.prose["foreshadow_undead"].variants[1].text.clone(),
        ),
        (
            "atmosphere.c".into(),
            "Mist threads between the dark trunks like slow smoke.".into(),
        ),
    ]);
    // Force distinct other atmosphere values to keep this exact long sentence compatible.
    let mut forced = forced;
    forced.insert(
        "atmosphere.a".into(),
        tables.locations.entries[1].atmosphere.entries[0]
            .text
            .clone(),
    );
    forced.insert(
        "atmosphere.b".into(),
        tables.locations.entries[1].atmosphere.entries[1]
            .text
            .clone(),
    );
    let generated = generate(&tables, 1, GenerateOptions::default(), &forced).unwrap();
    let opening = generated
        .pack
        .scenes
        .iter()
        .find(|s| s.key == super::super::pack::SceneKey::Opening)
        .unwrap();
    assert!(opening.narration.chars().count() <= 220);
    assert!(
        opening
            .narration
            .contains(&generated.settings.meeting.short)
    );
    assert!(opening.narration.contains(&generated.settings.meeting.name));
}

#[test]
fn table_errors_name_the_source_and_json_path() {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/play-g1/table-errors");
    std::fs::create_dir_all(&directory).unwrap();
    let originals =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables");
    for entry in std::fs::read_dir(&originals).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), directory.join(entry.file_name())).unwrap();
    }
    let original = std::fs::read(directory.join("hooks.json")).unwrap();
    for (pointer, value) in [
        (
            "/tables/hooks/entries/0/unexpected",
            serde_json::json!(true),
        ),
        (
            "/tables/hooks/entries/0/pitch",
            serde_json::json!("{unknown.path}"),
        ),
        (
            "/tables/hooks/entries/0/fits",
            serde_json::json!(["unknown"]),
        ),
    ] {
        let mut hooks: serde_json::Value = serde_json::from_slice(&original).unwrap();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        hooks
            .pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(key.into(), value);
        std::fs::write(
            directory.join("hooks.json"),
            serde_json::to_vec(&hooks).unwrap(),
        )
        .unwrap();
        let error = match Tables::load(&directory) {
            Err(error) => error,
            Ok(_) => panic!("invalid table admitted"),
        };
        assert!(error.to_string().contains("hooks.json"));
        assert!(error.to_string().contains("tables"));
    }
    std::fs::write(directory.join("hooks.json"), &original).unwrap();
    for filename in ["prose-patterns.json", "locations.json", "names.json"] {
        let original = std::fs::read(originals.join(filename)).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match filename {
            "prose-patterns.json" => {
                value["tables"]
                    .as_object_mut()
                    .unwrap()
                    .remove("opening_narration");
            }
            "locations.json" => {
                for entry in value["tables"]["locations"]["entries"][0]["atmosphere"]["entries"]
                    .as_array_mut()
                    .unwrap()
                {
                    entry["text"] = "The same atmosphere.".into();
                }
            }
            _ => {
                for entry in value["tables"]["he"]["entries"].as_array_mut().unwrap() {
                    entry["text"] = "Tomas".into();
                }
            }
        }
        std::fs::write(
            directory.join(filename),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        let error = match Tables::load(&directory) {
            Err(error) => error,
            Ok(_) => panic!("uncovered downstream table admitted"),
        };
        assert!(error.to_string().contains(filename));
        std::fs::write(directory.join(filename), original).unwrap();
    }
    let mut mechanical: serde_json::Value =
        serde_json::from_slice(&std::fs::read(originals.join("prose-patterns.json")).unwrap())
            .unwrap();
    mechanical["tables"]["combat_mech"]["variants"][0]["text"] = "All foes are slain.".into();
    std::fs::write(
        directory.join("prose-patterns.json"),
        serde_json::to_vec(&mechanical).unwrap(),
    )
    .unwrap();
    let error = match Tables::load(&directory) {
        Err(error) => error,
        Ok(_) => panic!("altered mechanics admitted"),
    };
    assert!(error.to_string().contains("mechanical template differs"));
    std::fs::remove_dir_all(directory).unwrap();
    assert!(matches!(
        generate(
            &tables(),
            1,
            GenerateOptions::default(),
            &ForcedEntries::from([("unsupported".into(), "entry".into())])
        ),
        Err(GenerateError::UnknownForced(_))
    ));
}

#[test]
fn every_compiled_creature_has_table_parity_and_loader_disclosure() {
    let tables = tables();
    assert_eq!(
        tables.antagonists.entries.len(),
        df_rules::SRD_521_CREATURES.len()
    );
    for creature in &df_rules::SRD_521_CREATURES {
        let archetype = tables
            .antagonists
            .entries
            .iter()
            .find(|e| e.id == creature.id)
            .unwrap();
        let location = tables
            .locations
            .entries
            .iter()
            .find(|e| super::tables::intersects(&e.tags, &archetype.fits))
            .unwrap();
        let forced = ForcedEntries::from([
            ("location".into(), location.id.clone()),
            ("antagonist".into(), creature.id.into()),
        ]);
        let generated = generate(
            &tables,
            42,
            GenerateOptions {
                rules_mode: RulesMode::CustomAllowed,
                ..GenerateOptions::default()
            },
            &forced,
        )
        .unwrap();
        let combat = generated
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == super::super::pack::SceneKey::Combat)
            .unwrap();
        assert_eq!(
            combat.speaker_portrait, tables.art[&archetype.art],
            "{} combat portrait",
            creature.id
        );
        if !archetype.speaks {
            assert!(combat.speaker.is_empty());
            assert!(combat.dialogue.is_empty());
        }
        let loaded = super::super::pack::load_from_bytes(
            &serde_json::to_vec(&generated.pack).unwrap(),
            &serde_json::to_vec(&generated.tuning).unwrap(),
        )
        .unwrap();
        if creature.exact {
            assert!(
                loaded.departures.is_empty(),
                "{}: {:?}",
                creature.id,
                loaded.departures
            );
        } else {
            assert_eq!(
                loaded.departures,
                vec![format!("antagonist.simplified:{}", creature.id)]
            );
        }
        let mut tampered = generated.tuning.clone();
        tampered.bandit.hit_points += 1;
        let loaded = super::super::pack::load_from_bytes(
            &serde_json::to_vec(&generated.pack).unwrap(),
            &serde_json::to_vec(&tampered).unwrap(),
        )
        .unwrap();
        assert!(loaded.departures.iter().any(|d| d == "bandit.hit_points"));
    }
}

#[test]
fn stored_adventure_resumes_identical_bytes_without_tables_or_new_seed() {
    use super::super::startup_configuration::{AdventureMode, AdventureSettings};
    use df_persistence::local_demo_scope::AdventureStartup;
    let tables = tables();
    let generated = generate(
        &tables,
        0x3c6ef378dd81de97,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let documents = generated.documents(true).unwrap();
    assert!(documents == generated.documents(true).unwrap());
    let current = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/play-g1/resume-{}",
        std::process::id()
    ));
    let directory = current.join("development/runs/df_gameplay_demo_record_test");
    super::record::create_cache(&directory, &documents).unwrap();
    let settings = AdventureSettings {
        mode: AdventureMode::Random,
        seed: Some(999),
        rules_mode: RulesMode::CustomAllowed,
        tables: current.join("does-not-exist"),
    };
    let resumed = super::startup::select(
        &settings,
        &current,
        "df_gameplay_demo_record_test",
        AdventureStartup::Stored(documents.clone()),
    )
    .unwrap();
    assert!(!resumed.fresh);
    assert_eq!(resumed.record.seed, "0x3c6ef378dd81de97");
    assert_eq!(resumed.loaded.pack, generated.pack);
    assert_eq!(resumed.loaded.tuning, generated.tuning);
    assert!(resumed.documents == documents);
    assert!(super::record::create_cache(&directory, &documents).is_err());
    std::fs::write(directory.join("pack.json"), b"edited cache").unwrap();
    assert!(
        super::startup::select(
            &settings,
            &current,
            "df_gameplay_demo_record_test",
            AdventureStartup::Stored(documents.clone())
        )
        .is_err()
    );
    std::fs::write(directory.join("pack.json"), &documents.pack).unwrap();
    std::fs::remove_file(directory.join("tuning.json")).unwrap();
    assert!(
        super::startup::select(
            &settings,
            &current,
            "df_gameplay_demo_record_test",
            AdventureStartup::Stored(documents)
        )
        .is_err()
    );
    std::fs::remove_dir_all(current).unwrap();
}

#[test]
fn record_binding_and_compiled_mechanical_text_fail_closed() {
    let tables = tables();
    let generated = generate(
        &tables,
        42,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let documents = generated.documents(false).unwrap();
    assert!(super::record::reload(&documents).is_ok());
    for key in ["seed", "pack_sha256", "tuning_sha256", "generator_version"] {
        let mut altered = documents.clone();
        let mut record: serde_json::Value = serde_json::from_slice(&altered.record).unwrap();
        record[key] = "changed".into();
        altered.record = serde_json::to_vec(&record).unwrap();
        assert!(super::record::reload(&altered).is_err());
    }
    struct UnvalidatedProse;
    impl ProseSource for UnvalidatedProse {
        fn express(&self, _: &FactSheet) -> Result<String, GenerateError> {
            Ok("A different unvalidated story.".into())
        }
    }
    assert!(
        generate_with_prose(
            &tables,
            42,
            GenerateOptions::default(),
            &ForcedEntries::new(),
            &UnvalidatedProse
        )
        .is_err()
    );
}

#[test]
fn two_hundred_seeds_bind_every_scene_to_its_story_setting() {
    use super::super::pack::SceneKey;
    let tables = tables();
    for seed in 0..200 {
        let generated = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        let keys: std::collections::BTreeSet<_> = generated
            .pack
            .scenes
            .iter()
            .map(|s| s.setting.as_ref().unwrap().art_key.clone())
            .collect();
        assert!(keys.len() >= 3, "seed {seed}");
        for scene in &generated.pack.scenes {
            let setting = scene.setting.as_ref().unwrap();
            assert_eq!(setting, generated.settings.scene(scene.key));
            assert!(tables.depicts[&setting.art_key].contains(&setting.kind));
            assert_eq!(scene.asset, tables.art[&setting.art_key]);
            if scene.key == SceneKey::Opening {
                assert_eq!(
                    setting.art_key,
                    tables.places.meeting_places[&generated.facts["patron.id"]].art
                );
                assert!(
                    scene
                        .narration
                        .to_lowercase()
                        .contains(&setting.short.to_lowercase())
                );
                assert!(
                    scene
                        .narration
                        .split('.')
                        .next()
                        .unwrap()
                        .contains(&setting.name)
                );
            }
            if scene.key == SceneKey::Combat {
                assert!(
                    scene
                        .narration
                        .contains(&generated.facts["setting.confrontation.detail"])
                );
            }
        }
        for sheet in &generated.fact_sheet {
            for prefix in ["meeting", "confrontation", "destination"] {
                assert!(sheet.facts.contains_key(&format!("setting.{prefix}.name")));
            }
        }
    }
}

#[test]
fn generic_narration_patterns_have_no_hardcoded_foreign_place_words() {
    let tables = tables();
    // These physical place words belong to settings, not generic reusable narration.
    let vocabulary = [
        "inn",
        "taproom",
        "temple",
        "sanctuary",
        "archive",
        "ferry",
        "shop",
        "shed",
        "sheepfold",
        "guardhouse",
        "chamber",
        "quay",
        "wharf",
        "forest",
        "trail",
        "pass",
        "market",
        "graveyard",
        "boardwalk",
        "farmyard",
        "library",
        "road",
        "loft",
        "boathouse",
        "hut",
        "lodge",
        "cabin",
        "hall",
        "bothy",
        "chapel",
        "vestry",
        "kitchen",
        "cellar",
        "bindery",
        "gallery",
    ];
    for (key, table) in &tables.prose {
        if !(key.contains("narration")
            || key.starts_with("combat_open")
            || key.starts_with("victory_lead")
            || key.starts_with("defeat_")
            || key == "inn_intro")
        {
            continue;
        }
        for variant in &table.variants {
            let words: Vec<_> = variant
                .text
                .split(|c: char| !c.is_ascii_alphabetic())
                .map(str::to_lowercase)
                .collect();
            for foreign in vocabulary {
                assert!(
                    !words.iter().any(|w| w == foreign),
                    "{key}: {foreign}: {}",
                    variant.text
                );
            }
        }
    }
    let fragments = tables
        .complications
        .entries
        .iter()
        .map(|e| e.text.as_str())
        .chain(
            tables
                .hooks
                .entries
                .iter()
                .flat_map(|e| [e.entrance.as_str(), e.title.as_str()]),
        )
        .chain(tables.secrets.entries.iter().map(|e| e.fact.as_str()));
    for text in fragments {
        let words: Vec<_> = text
            .split(|c: char| !c.is_ascii_alphabetic())
            .map(str::to_lowercase)
            .collect();
        for foreign in vocabulary {
            assert!(
                !words.iter().any(|w| w == foreign),
                "narration fragment: {foreign}: {text}"
            );
        }
    }
}

#[test]
fn loader_rejects_unknown_setting_kind_depiction_mismatch_and_partial_settings() {
    let generated = generate(
        &tables(),
        42,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let original = serde_json::to_value(&generated.pack).unwrap();
    for mutation in ["unknown", "mismatch", "partial"] {
        let mut pack = original.clone();
        match mutation {
            "unknown" => pack["scenes"][0]["setting"]["kind"] = "spaceship".into(),
            "mismatch" => pack["scenes"][0]["setting"]["kind"] = "inn".into(),
            _ => {
                pack["scenes"][0].as_object_mut().unwrap().remove("setting");
            }
        }
        assert!(
            super::super::pack::load_from_bytes(
                &serde_json::to_vec(&pack).unwrap(),
                &serde_json::to_vec(&generated.tuning).unwrap()
            )
            .is_err(),
            "{mutation}"
        );
    }
    let original_dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables");
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/play-g5/setting-errors");
    std::fs::create_dir_all(&directory).unwrap();
    for entry in std::fs::read_dir(original_dir).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), directory.join(entry.file_name())).unwrap();
    }
    let original = std::fs::read(directory.join("places.json")).unwrap();
    for kind in ["spaceship", "archive"] {
        let mut places: serde_json::Value = serde_json::from_slice(&original).unwrap();
        places["meeting_places"]["innkeeper"]["kind"] = kind.into();
        std::fs::write(
            directory.join("places.json"),
            serde_json::to_vec(&places).unwrap(),
        )
        .unwrap();
        assert!(Tables::load(&directory).is_err());
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn old_record_without_any_settings_loads_through_typed_legacy_path() {
    use sha2::{Digest, Sha256};
    let generated = generate(
        &tables(),
        42,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let mut documents = generated.documents(true).unwrap();
    let mut pack: serde_json::Value = serde_json::from_slice(&documents.pack).unwrap();
    for scene in pack["scenes"].as_array_mut().unwrap() {
        scene.as_object_mut().unwrap().remove("setting");
    }
    documents.pack = serde_json::to_vec_pretty(&pack).unwrap();
    let mut record: serde_json::Value = serde_json::from_slice(&documents.record).unwrap();
    record.as_object_mut().unwrap().remove("settings");
    record["pack_sha256"] = format!("{:x}", Sha256::digest(&documents.pack)).into();
    let mut digest = Sha256::new();
    digest.update(&documents.pack);
    digest.update(&documents.tuning);
    record["expression"]["output_sha256"] = format!("{:x}", digest.finalize()).into();
    documents.record = serde_json::to_vec_pretty(&record).unwrap();
    let (loaded, record) = record::reload(&documents).unwrap();
    assert!(loaded.pack.scenes.iter().all(|s| s.setting.is_none()));
    assert!(matches!(
        record.setting_state(),
        record::SettingState::LegacyNoSettings
    ));
}

#[test]
fn startup_substitutes_missing_paintings_without_changing_stored_documents() {
    let generated = generate(
        &tables(),
        42,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let documents = generated.documents(true).unwrap();
    let (mut loaded, record) = record::reload(&documents).unwrap();
    let original_pack = documents.pack.clone();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/play-g5/missing-art");
    std::fs::create_dir_all(directory.join("generated")).unwrap();
    let general = generated
        .pack
        .scenes
        .iter()
        .find(|s| s.key == super::super::pack::SceneKey::Room)
        .unwrap()
        .asset
        .strip_prefix("assets/ui/generated/")
        .unwrap();
    let general_file = directory.join("generated").join(format!("{general}.png"));
    std::fs::write(&general_file, b"fixture painting").unwrap();
    let substitutions = startup::substitute_missing_art(&mut loaded, &record, &directory);
    assert_eq!(substitutions.len(), 5);
    assert!(
        loaded
            .pack
            .scenes
            .iter()
            .all(|s| s.asset == format!("assets/ui/generated/{general}"))
    );
    assert_eq!(documents.pack, original_pack);
    assert!(loaded.pack.scenes.iter().all(|s| s.setting.is_some()));
    std::fs::remove_file(general_file).unwrap();
    let substitutions = startup::substitute_missing_art(&mut loaded, &record, &directory);
    assert_eq!(substitutions.len(), 8);
    assert!(
        loaded
            .pack
            .scenes
            .iter()
            .all(|s| s.asset == "assets/ui/scenes/mara-harbor-v4.png")
    );
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn every_fight_across_two_hundred_seeds_is_exterior() {
    let tables = tables();
    for seed in 0..200 {
        let generated = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        assert!(
            generated.settings.confrontation.exterior,
            "seed {seed}: {}",
            generated.settings.confrontation.name
        );
    }
}
#[test]
fn every_archetype_voice_agrees_with_its_gender_and_has_an_explicit_age() {
    let tables = tables();
    let female = ["woman", "women", "girl", "lady", "she", "her"];
    let male = ["man", "men", "boy", "he", "his", "him"];
    let archetypes = tables
        .patrons
        .entries
        .iter()
        .chain(&tables.contacts.entries)
        .map(|npc| {
            (
                npc.id.as_str(),
                npc.gender.as_str(),
                npc.voice.as_str(),
                npc.age.as_str(),
            )
        })
        .chain(tables.antagonists.entries.iter().map(|ant| {
            (
                ant.id.as_str(),
                ant.gender.as_str(),
                ant.voice.as_str(),
                ant.age.as_str(),
            )
        }));
    for (id, gender, voice, age) in archetypes {
        let voice_words: Vec<String> = voice
            .to_lowercase()
            .split(|c: char| !c.is_alphabetic())
            .map(str::to_owned)
            .collect();
        let has = |list: &[&str]| voice_words.iter().any(|word| list.contains(&word.as_str()));
        match gender {
            "he" => assert!(!has(&female), "{id}: {voice:?} contradicts he"),
            "she" => assert!(!has(&male), "{id}: {voice:?} contradicts she"),
            "it" => assert!(
                !has(&female) && !has(&male),
                "{id}: {voice:?} contradicts it"
            ),
            other => assert_eq!(other, "they", "{id}"),
        }
        assert!(["young", "adult", "elder"].contains(&age), "{id}: {age}");
        // The age agrees with the phrase's own age words.
        if voice.contains("young") {
            assert_eq!(age, "young", "{id}");
        }
        if ["old", "older", "elderly", "grizzled"]
            .iter()
            .any(|w| voice_words.contains(&(*w).to_owned()))
        {
            assert_eq!(age, "elder", "{id}");
        }
        if voice.contains("middle") {
            assert_eq!(age, "adult", "{id}");
        }
    }
}

#[test]
fn generated_prose_pronouns_match_each_characters_gender_and_the_pack_says_it_explicitly() {
    use super::super::pack::{Age, Gender};
    let tables = tables();
    let opposite = |gender: &str| -> [&'static str; 3] {
        match gender {
            "he" => ["she", "her", "herself"],
            "she" => ["he", "his", "himself"],
            _ => ["he", "she", "his"],
        }
    };
    for seed in 0..300_u64 {
        let adventure = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        for (prefix, entries) in [
            ("patron", &tables.patrons.entries),
            ("contact", &tables.contacts.entries),
        ] {
            let id = &adventure.facts[&format!("{prefix}.id")];
            let archetype = entries.iter().find(|npc| &npc.id == id).unwrap();
            let sub = &adventure.facts[&format!("{prefix}.sub")];
            assert_eq!(sub, &archetype.gender, "{id}");
            let mannerism = adventure.facts[&format!("{prefix}.mannerism")].to_lowercase();
            for word in opposite(&archetype.gender) {
                assert!(
                    !mannerism
                        .split(|c: char| !c.is_alphabetic())
                        .any(|w| w == word),
                    "{id} mannerism {mannerism:?}"
                );
            }
        }
        let by_name = |name: &str| {
            adventure
                .pack
                .speakers
                .iter()
                .find(|speaker| speaker.name == name)
                .unwrap()
        };
        let patron = by_name(&adventure.facts["patron.first"]);
        assert_eq!(
            patron.gender,
            Gender::from_table(&adventure.facts["patron.gender"])
        );
        assert!(patron.gender.is_some() && patron.age.is_some());
        let contact = by_name(&adventure.facts["contact.first"]);
        assert_eq!(
            contact.age,
            Age::from_table(&adventure.facts["contact.age"])
        );
        for scene in &adventure.pack.scenes {
            for actor in &scene.cast {
                assert!(
                    actor.gender.is_some() && actor.age.is_some(),
                    "{}",
                    actor.name
                );
            }
        }
    }
    // A creature without a person's gender is `it`; a speaking person carries theirs.
    let rat = generate(
        &tables,
        0xdaa66d325ccc5aac,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    let combat = rat
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == super::super::pack::SceneKey::Combat)
        .unwrap();
    assert_eq!(combat.cast[0].gender, Some(Gender::It));
}

#[test]
fn every_scene_has_its_cast_and_the_speaker_is_the_one_marked_speaking() {
    use super::super::{pack::SceneKey, wire::scene_cast};
    use df_protocol::common as rpc;
    let tables = tables();
    type SceneCast = Vec<(SceneKey, Vec<(String, i32, bool)>)>;
    let cast_of = |seed: u64| -> SceneCast {
        let adventure = generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        adventure
            .pack
            .scenes
            .iter()
            .map(|scene| {
                for actor in &scene.cast {
                    assert_eq!(
                        actor.sprite,
                        actor.portrait.replace("/portrait-", "/sprite-"),
                        "sprite stem follows the portrait stem"
                    );
                    assert!(actor.portrait.contains("/portrait-"));
                }
                (
                    scene.key,
                    scene_cast(scene)
                        .into_iter()
                        .map(|actor| (actor.name, actor.role, actor.speaking))
                        .collect(),
                )
            })
            .collect()
    };
    let patron = rpc::SceneRole::Patron as i32;
    let enemy = rpc::SceneRole::Antagonist as i32;
    let contact = rpc::SceneRole::Contact as i32;
    // Some antagonists speak and some creatures do not; take the first seed of each kind.
    let rolled = |seed: u64| {
        generate(
            &tables,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap()
    };
    let speaks = |seed: &u64| {
        rolled(*seed)
            .pack
            .scenes
            .iter()
            .any(|scene| scene.key == SceneKey::Combat && !scene.dialogue.is_empty())
    };
    let speaking_seed = (0..200_u64).find(speaks).unwrap();
    let mute_seed = (0..200_u64).find(|seed| !speaks(seed)).unwrap();
    let spoken = rolled(speaking_seed);
    let goblin = cast_of(speaking_seed);
    let scene = |cast: &SceneCast, key| {
        cast.iter()
            .find(|(candidate, _)| *candidate == key)
            .unwrap()
            .1
            .clone()
    };
    let ilsa = (spoken.facts["patron.first"].clone(), patron);
    let snag = (spoken.pack.bandit.name.clone(), enemy);
    assert_eq!(
        scene(&goblin, SceneKey::Opening),
        [(ilsa.0.clone(), ilsa.1, true)]
    );
    assert_eq!(
        scene(&goblin, SceneKey::DialogueSeal),
        [
            (ilsa.0.clone(), ilsa.1, true),
            (snag.0.clone(), snag.1, false)
        ]
    );
    assert_eq!(scene(&goblin, SceneKey::DialogueEscort).len(), 2);
    assert_eq!(scene(&goblin, SceneKey::Combat), [(snag.0, snag.1, true)]);
    assert_eq!(
        scene(&goblin, SceneKey::Inn),
        [(spoken.facts["contact.first"].clone(), contact, true)]
    );
    assert_eq!(scene(&goblin, SceneKey::Victory), [(ilsa.0, ilsa.1, true)]);
    assert!(scene(&goblin, SceneKey::Room).is_empty());
    assert!(scene(&goblin, SceneKey::Defeat).is_empty());
    let rat = cast_of(mute_seed);
    let combat = scene(&rat, SceneKey::Combat);
    assert_eq!(combat.len(), 1);
    assert_eq!(combat[0].1, enemy);
    assert!(
        !combat[0].2,
        "a creature that does not speak is never speaking"
    );
    assert_eq!(scene(&rat, SceneKey::DialogueSeal).len(), 2);
    let documents = generate(
        &tables,
        0x3c6ef378dd81de97,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap()
    .documents(false)
    .unwrap();
    let record = String::from_utf8(documents.record).unwrap();
    assert!(record.contains("\"scene_cast\""));
    assert!(record.contains("\"role\": \"antagonist\""));
}

#[test]
fn generated_packs_carry_secret_terms_and_a_profile_for_every_shown_speaker() {
    let tables = tables();
    let adventure = generate(
        &tables,
        0x3c6ef378dd81de97,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();

    // The secret's own phrases, then where it leads and who waits there.
    let terms = &adventure.pack.secret_terms;
    assert!(terms.len() >= 3, "{terms:?}");
    assert!(
        terms.contains(&adventure.facts["contact.first"]),
        "{terms:?}"
    );
    let speakers: Vec<(&str, &str)> = adventure
        .pack
        .speakers
        .iter()
        .map(|speaker| (speaker.name.as_str(), speaker.voice.as_str()))
        .collect();
    assert_eq!(
        speakers[0],
        (
            adventure.facts["patron.first"].as_str(),
            adventure.facts["patron.voice"].as_str()
        )
    );
    assert_eq!(
        speakers[1],
        (
            adventure.facts["contact.first"].as_str(),
            adventure.facts["contact.voice"].as_str()
        )
    );
    assert_eq!(speakers[2].0, adventure.pack.bandit.name);
    let enemy = &adventure.pack.speakers[2].role;
    assert!(enemy.starts_with("a hostile ") && enemy.contains("fighting the heroes"));
    for scene in &adventure.pack.scenes {
        assert!(
            scene.speaker.is_empty()
                || adventure
                    .pack
                    .speakers
                    .iter()
                    .any(|speaker| speaker.name == scene.speaker),
            "{} has no profile",
            scene.speaker
        );
    }
    let documents = adventure.documents(false).unwrap();
    let record = String::from_utf8(documents.record).unwrap();
    assert!(record.contains("\"secret_terms\""));
    assert!(record.contains("\"speakers\""));
}

#[test]
fn dossiers_are_stored_with_the_adventure_and_an_adventure_saved_without_them_still_loads() {
    use sha2::{Digest, Sha256};
    let generated = generate(
        &tables(),
        42,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap();
    assert!(
        generated
            .rolls
            .iter()
            .all(|roll| !roll.table.starts_with("dossier.")),
        "dossier rolls never enter the adventure's own record of dice"
    );
    assert_eq!(
        generated
            .dossiers
            .iter()
            .map(|d| d.npc_id.split(':').next().unwrap())
            .collect::<Vec<_>>()[..2],
        ["patron", "contact"]
    );
    let mut documents = generated.documents(true).unwrap();
    let (_, stored) = record::reload(&documents).unwrap();
    assert_eq!(stored.dossiers, generated.dossiers);

    let mut value: serde_json::Value = serde_json::from_slice(&documents.record).unwrap();
    value.as_object_mut().unwrap().remove("dossiers");
    documents.record = serde_json::to_vec_pretty(&value).unwrap();
    let (loaded, old) = record::reload(&documents).unwrap();
    assert!(old.dossiers.is_empty());
    assert_eq!(
        format!("{:x}", Sha256::digest(&documents.pack)),
        format!(
            "{:x}",
            Sha256::digest(&generated.documents(true).unwrap().pack)
        ),
        "the pack bytes do not depend on dossiers"
    );
    assert!(!loaded.pack.scenes.is_empty());

    // A tampered dossier that would say a secret is refused like any tampered document.
    let mut tampered: serde_json::Value = serde_json::from_slice(&documents.record).unwrap();
    let mut dossier = serde_json::to_value(&generated.dossiers[0]).unwrap();
    dossier["knowledge"]["says_freely"][0]["text"] = generated.dossiers[0].knowledge.never_says[0]
        .terms[0]
        .clone()
        .into();
    tampered["dossiers"] = serde_json::json!([dossier]);
    documents.record = serde_json::to_vec_pretty(&tampered).unwrap();
    assert!(record::reload(&documents).is_err());
}

fn options(rating: ContentRating) -> GenerateOptions {
    GenerateOptions {
        rating,
        ..GenerateOptions::default()
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// The SHA-256 of pack, tuning and record of three fixed seeds as the generator wrote them
/// before the content rating existed (with `GenerateOptions::default()`).
/// Pack and record were re-pinned when every pack gained its `beat_script`: with that one key
/// removed, each pack was byte-identical to the earlier pin.
// The third hash of each seed is the adventure record, which embeds the NPC dossiers. Dossier text
// changes re-pin only that hash, after review; the pack and tuning hashes never move with it.
const RESTRAINED_GOLDEN: [(u64, [&str; 3]); 3] = [
    (
        0x3c6ef378dd81de97,
        [
            "072519d1e82c29c030e6788b77af55483ff477e1290f8ffbe9e5952692cf70cb",
            "fd318025d2d7b016708169b59777687fe1f7b630a3b70eb2715dad37af962933",
            "2541b5d7ea0bfab9d19dea23262cf40a583faf9a32f30698ff4ea5a752ca6233",
        ],
    ),
    (
        0xdaa66d325ccc5aac,
        [
            "80a4f6aa32d5b1ecbe68ab33a5e2e43ecb6697e5c62a499df1b91001e6c01e44",
            "6e122920b8921cd08441ec5c13360ac3898742e2d143a16e0c8a8d678d367ace",
            "796fff4becbced8f36a7bcc1fb11d5eee0d4516ca4aae6d17654a02dbce5f7bb",
        ],
    ),
    (
        0x11a25cdccc7d808f,
        [
            "2c75bec4abecef0694322db9b71f6bf826b7161641ed6c520f9377008eebb705",
            "d29e661273c6de6896f5723c824099989ad0b023d8f9085a5f9bc73cbb5450d7",
            "18c0c3c7c72d991e58d0bc28e65d4bc689c51deb84383da70df70b5e8e38a4ea",
        ],
    ),
];

#[test]
fn restrained_generation_is_byte_identical_to_the_tables_before_the_content_rating() {
    let tables = tables();
    for (seed, [pack, tuning, record]) in RESTRAINED_GOLDEN {
        let documents = generate(
            &tables,
            seed,
            options(ContentRating::Restrained),
            &ForcedEntries::new(),
        )
        .unwrap()
        .documents(true)
        .unwrap();
        assert_eq!(sha256(&documents.pack), pack, "{seed:#x} pack");
        assert_eq!(sha256(&documents.tuning), tuning, "{seed:#x} tuning");
        assert_eq!(sha256(&documents.record), record, "{seed:#x} record");
    }
}

#[test]
fn mature_generation_is_deterministic_differs_from_restrained_and_reloads() {
    let tables = tables();
    for (seed, _) in RESTRAINED_GOLDEN {
        let mature = |tables: &Tables| {
            generate(
                tables,
                seed,
                options(ContentRating::Mature),
                &ForcedEntries::new(),
            )
            .unwrap()
        };
        let (first, second) = (mature(&tables), mature(&tables));
        let (a, b) = (
            first.documents(true).unwrap(),
            second.documents(true).unwrap(),
        );
        assert_eq!(
            (&a.pack, &a.tuning, &a.record),
            (&b.pack, &b.tuning, &b.record)
        );
        assert_eq!(first.dossiers, second.dossiers);

        let restrained = generate(
            &tables,
            seed,
            options(ContentRating::Restrained),
            &ForcedEntries::new(),
        )
        .unwrap();
        assert_ne!(
            first.pack, restrained.pack,
            "{seed:#x}: the fight reads differently"
        );
        assert_ne!(
            first.tables_revision, restrained.tables_revision,
            "a mature adventure records the overlay in its revision"
        );
        assert_ne!(
            first.dossiers[0].tables_revision,
            restrained.dossiers[0].tables_revision
        );
        assert_eq!(
            first.facts["loc.id"], restrained.facts["loc.id"],
            "the same story"
        );
        assert_eq!(first.facts["ant.id"], restrained.facts["ant.id"]);
        assert_eq!(first.tuning, restrained.tuning, "mechanics are never rated");
        let (loaded, record) = record::reload(&a).unwrap();
        assert_eq!(loaded.pack, first.pack);
        assert_eq!(record.tables_revision(), first.tables_revision);
    }
}

#[test]
fn a_thousand_mature_seeds_validate_within_every_bound() {
    let tables = tables();
    for seed in 0..1000 {
        let adventure = generate(
            &tables,
            seed,
            options(ContentRating::Mature),
            &ForcedEntries::new(),
        )
        .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
        record::reload(&adventure.documents(true).unwrap())
            .unwrap_or_else(|error| panic!("seed {seed}: {error}"));
    }
}

#[test]
fn the_fight_and_its_aftermath_are_darker_when_mature_and_still_nonlethal() {
    let tables = tables();
    let scene_text = |rating| {
        let mut text = String::new();
        for seed in 0..300 {
            let adventure =
                generate(&tables, seed, options(rating), &ForcedEntries::new()).unwrap();
            for scene in &adventure.pack.scenes {
                text.push_str(&format!(
                    "{} {} {}\n",
                    scene.title, scene.narration, scene.dialogue
                ));
            }
        }
        text.to_lowercase()
    };
    let (mature, restrained) = (
        scene_text(ContentRating::Mature),
        scene_text(ContentRating::Restrained),
    );
    for word in ["blood", "bleed", "damn", "gnaw", "split lip"] {
        assert!(mature.contains(word), "mature text has {word}");
        assert!(!restrained.contains(word), "restrained text has no {word}");
    }
    // The rules decide the fight: nobody dies in either rating's narration of it.
    let words: std::collections::BTreeSet<&str> = mature
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    for word in ["dies", "die", "slain", "killed", "kills", "dead", "corpse"] {
        assert!(!words.contains(word), "{word}");
    }
}
