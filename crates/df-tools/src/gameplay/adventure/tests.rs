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
fn merchant_shop_opening_cast_includes_every_authored_background_person() {
    let tables = tables();
    let mut forced = ForcedEntries::new();
    forced.insert("location".into(), "lamplit-market".into());
    forced.insert("patron".into(), "merchant".into());
    let adventure = generate(&tables, 671003, GenerateOptions::default(), &forced).unwrap();
    let opening = adventure
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == super::super::pack::SceneKey::Opening)
        .unwrap();
    let setting = opening.setting.as_ref().unwrap();
    assert_eq!(setting.kind, super::setting::SettingKind::Shop);
    let cast = opening
        .cast
        .iter()
        .map(|actor| (&actor.id, &actor.name, &actor.sprite))
        .collect::<Vec<_>>();
    assert!(cast.iter().any(|(id, _, _)| id.as_str() == "merchant"));
    assert!(cast.iter().any(|(id, _, sprite)| {
        id.as_str().ends_with("bystander-1") && sprite.contains("sprite-shop-adventurer")
    }));
    assert!(cast.iter().any(|(id, _, sprite)| {
        id.as_str().ends_with("bystander-2") && sprite.contains("sprite-shop-soldier")
    }));
    assert_eq!(cast.len(), 3);

    for key in [
        super::super::pack::SceneKey::DialogueSeal,
        super::super::pack::SceneKey::DialogueEscort,
    ] {
        let scene = adventure
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == key)
            .unwrap();
        assert_eq!(scene.cast.len(), 3, "quiet shop cast for {key:?}");
        assert!(
            !scene.narration.contains(&adventure.facts["ant.short"]),
            "quiet shop narration should not stage the antagonist in {key:?}: {}",
            scene.narration
        );
        assert!(
            scene
                .cast
                .iter()
                .all(|actor| actor.role != super::super::pack::CastRole::Antagonist),
            "quiet shop should not stage the antagonist in {key:?}"
        );
    }
    assert!(
        adventure
            .fact_sheet
            .iter()
            .filter(|sheet| {
                matches!(
                    sheet.scene.as_deref(),
                    Some("dialogue-seal" | "dialogue-escort")
                ) && sheet.slot == "narration"
            })
            .all(|sheet| sheet.facts.get("scene.antagonist_present") == Some(&"false".to_owned()))
    );
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
fn combat_introduction_fact_sheet_includes_roster_and_source_weapons() {
    let forced = ForcedEntries::from([
        ("location".into(), "pinewood-trail".into()),
        ("antagonist".into(), "giant-weasel".into()),
    ]);
    let generated = generate(&tables(), 17, GenerateOptions::default(), &forced).unwrap();
    let sheet = generated
        .fact_sheet
        .iter()
        .find(|sheet| sheet.beat == "confrontation" && sheet.slot == "narration")
        .unwrap();

    assert_eq!(sheet.facts["encounter.count"], "3");
    assert_eq!(sheet.facts["encounter.weapons"], "bite");
    assert!(sheet.facts["encounter.creatures"].contains("giant weasels"));
    assert!(sheet.facts["encounter.fallback"].contains("three giant weasels"));
    assert!(sheet.facts["encounter.fallback"].contains("bite"));

    // Every antagonist's roster reads as English: the count as a word and a real plural.
    let creatures = |antagonist: &str| {
        let tables = tables();
        let fits = &tables
            .antagonists
            .entries
            .iter()
            .find(|entry| entry.id == antagonist)
            .unwrap()
            .fits;
        let location = tables
            .locations
            .entries
            .iter()
            .find(|entry| super::tables::intersects(&entry.tags, fits))
            .unwrap();
        let forced = ForcedEntries::from([
            ("location".into(), location.id.clone()),
            ("antagonist".into(), antagonist.into()),
        ]);
        let options = GenerateOptions {
            rules_mode: RulesMode::CustomAllowed,
            ..GenerateOptions::default()
        };
        let generated = generate(&tables, 17, options, &forced).unwrap();
        generated
            .fact_sheet
            .iter()
            .find(|sheet| sheet.beat == "confrontation" && sheet.slot == "narration")
            .unwrap()
            .facts["encounter.creatures"]
            .clone()
    };
    assert_eq!(creatures("wolf"), "two wolves");
    assert_eq!(creatures("mastiff"), "three mastiffs");
    assert_eq!(creatures("scout"), "one scout and one bandit");
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
        // Tactical source profiles implement the complete T01 stat blocks; legacy
        // one-weapon fidelity flags no longer describe their mechanical authority.
        assert!(loaded.tuning.tactical.is_some());
        assert_eq!(loaded.departures, ["tactical.cover-centre-line"]);
        let mut tampered = generated.tuning.clone();
        tampered.hero.armor_class += 1;
        assert!(
            super::super::pack::load_from_bytes(
                &serde_json::to_vec(&generated.pack).unwrap(),
                &serde_json::to_vec(&tampered).unwrap(),
            )
            .is_err()
        );
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
        chapter_type: None,
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
            let stored_setting = generated.settings.scene(scene.key);
            let mut pack_setting = stored_setting.clone();
            pack_setting.model = None;
            assert_eq!(setting, &pack_setting);
            assert!(stored_setting.model.is_some());
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
    // A legacy record has none on the chapter's own closing scenes either.
    if let Some(scenes) = pack["beat_script"]["scenes"].as_object_mut() {
        for scene in scenes.values_mut() {
            scene.as_object_mut().unwrap().remove("setting");
        }
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
    // Five base scenes and the eight closing-scene variants of a fight chapter.
    assert_eq!(substitutions.len(), 13);
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
    assert_eq!(substitutions.len(), 16);
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
    assert!(generated.dossiers.iter().all(|dossier| {
        dossier.subject_answers.len() == 8
            && dossier.subject_answers.values().all(|answer| {
                !answer.text.trim().is_empty() && !answer.restatement.trim().is_empty()
            })
    }));

    // SRD 5.2.1, p. 184 (Influence): this checks stored dialogue facts, not an Influence check.
    let mut old_dossier_record = documents.clone();
    let mut old_value: serde_json::Value =
        serde_json::from_slice(&old_dossier_record.record).unwrap();
    for dossier in old_value["dossiers"].as_array_mut().unwrap() {
        dossier.as_object_mut().unwrap().remove("subject_answers");
    }
    old_dossier_record.record = serde_json::to_vec_pretty(&old_value).unwrap();
    let (_, old_dossiers) = record::reload(&old_dossier_record).unwrap();
    assert!(
        old_dossiers
            .dossiers
            .iter()
            .all(|dossier| dossier.subject_answers.is_empty())
    );

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

/// Current tactical pack, encounter tuning, and disclosed-cover record pins.
/// PLAY-E03 re-pinned only the record hashes. Pack and tuning hashes were verified unchanged for
/// all three seeds; the record embeds the table revision, which covers the peaceful resolution,
/// epilogue and end-card entries added to `beat-scripts.json`.
/// PLAY-T02 intentionally changes fight/rest text and activates source encounters.
/// The record pins were taken again when PLAY-T02 met PLAY-N05 (the record embeds the NPC
/// dossiers, which now carry each speaker's errand knowledge); pack and tuning matched PLAY-T02's
/// pins unchanged. PLAY-N06 added actionable rumours and the errand lines to the NPC knowledge
/// table, so the record pins moved once more; pack and tuning stayed. Dossier text changes
/// re-pin only the record hash, after review; the pack and tuning hashes never move with it.
/// PLAY-E04 gave every fight chapter a resolution, an epilogue and an end card (labels, eight
/// closing-scene patterns and four beats in `beat-scripts.json`), so the pack and record hashes
/// moved. Verified for all three seeds by generating with the previous table: the old pack hash
/// matched the pin above this change, the pack minus its `beat_script` was byte-identical to the
/// new one, and the tuning hash is unchanged. The end-card narrations were reworded once more
/// (no sentence starts with a lower-case stake) and re-pinned the same way (tuning unchanged).
/// DIALOGUE-001 M1 fixed one colon in a `npc-social.json` bribe fallback line (spoken register),
/// so the dossier tables revision and the record hashes moved; pack and tuning are unchanged.
/// NTE-01 adds the compact scene facts used to populate the durable notebook, changing records
/// while leaving the pack and tuning bytes unchanged.
/// STO-01/02 add a coherent job, failure stake and exact gold reward to the opening, and name the
/// foreshadowed threat in the pre-fight offer; those story-facing changes move pack and record pins.
const RESTRAINED_GOLDEN: [(u64, [&str; 3]); 3] = [
    (
        0x3c6ef378dd81de97,
        [
            "35597663b0b53680085997ace1beb84031f028ac6d76a1cbcc2a63ff3fc1d482",
            "737a6b07cd72aea7cfa2ffa40d1668389bd5edb862b24a567bcfa8ea9e4548b4",
            "e6f3105f28d9b5e9ed66a6a1b1a378b3b7a0864d657bf98ad8cbfb31f3088860",
        ],
    ),
    (
        0xdaa66d325ccc5aac,
        [
            "05c6db51e18680a4580e0c5d04afcf4bd77aec344c85198fdd90e8a5cb45a16d",
            "4d05fcc39c31cb1013cc3db19184ca9f393ce34fdd179c336982b49daf3dca84",
            "3c1d3296987ee7584d12c6891f6630fae6eeca6b8492ac38c432cb4a96fac1a5",
        ],
    ),
    (
        0x11a25cdccc7d808f,
        [
            "84fec187fd66b235e3e8d6ec74b85ea67175ada87d02a1b359172c11584a099e",
            "e193cdaa460abcc354e13b7761e1c287b3ede5fa9b90d6be0517b236b1e7627e",
            "7de96e697b10f839f824b539f66462b51acf32c116a86586720896d2e1d1d372",
        ],
    ),
];

#[test]
fn restrained_generation_matches_current_tactical_document_pins() {
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
        assert_eq!(
            [
                sha256(&documents.pack),
                sha256(&documents.tuning),
                sha256(&documents.record),
            ],
            [pack.to_owned(), tuning.to_owned(), record.to_owned()],
            "{seed:#x} documents"
        );
        if let Err(error) = super::record::reload(&documents) {
            panic!("{seed:#x}: {error}");
        }
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

fn assert_no_story_rule_failures(rule: &str, failed_seeds: &[u64]) {
    assert!(
        failed_seeds.is_empty(),
        "{rule}: {} of 200 seeds failed; first failures: {:?}",
        failed_seeds.len(),
        &failed_seeds[..failed_seeds.len().min(8)]
    );
}

fn generated_story_seed(tables: &Tables, seed: u64) -> GeneratedAdventure {
    generate(
        tables,
        seed,
        GenerateOptions::default(),
        &ForcedEntries::new(),
    )
    .unwrap_or_else(|error| panic!("seed {seed}: {error}"))
}

#[test]
fn opening_gives_the_job_stake_failure_and_reward_for_two_hundred_seeds() {
    let tables = tables();
    let mut failed_seeds = Vec::new();
    for seed in 0..200 {
        let adventure = generated_story_seed(&tables, seed);
        let opening = adventure
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == crate::gameplay::pack::SceneKey::Opening)
            .unwrap();
        let opening_line = adventure
            .fact_sheet
            .iter()
            .find(|sheet| sheet.scene.as_deref() == Some("opening") && sheet.slot == "line");
        let stake = adventure.facts.get("stake.short");
        let consequence = adventure.facts.get("stake.failure");
        let reward = adventure
            .facts
            .get("reward.gold")
            .and_then(|amount| amount.parse::<u8>().ok());
        let reward_matches_table =
            reward.is_some_and(|amount| {
                adventure.facts.get("reward.line").is_some_and(|line| {
                    tables.rewards.entries.iter().any(|entry| {
                        entry.gold == Some(amount) && entry.text.as_str() == line.as_str()
                    })
                })
            });
        let reward_words = adventure.facts.get("reward.words");
        let includes_reward = reward_words.is_some_and(|words| {
            opening
                .dialogue
                .to_lowercase()
                .contains(&words.to_lowercase())
                && opening.dialogue.to_lowercase().contains("gold")
        });
        let job = adventure.facts.get("hook.job");
        let full_stake = adventure.facts.get("stake.full");
        let names_stake = stake.is_some_and(|value| opening.dialogue.contains(value))
            || full_stake.is_some_and(|value| opening.dialogue.contains(value));
        let meeting = &adventure.settings.meeting;
        let names_meeting = opening.narration.contains(&meeting.name)
            && opening.narration.contains(&meeting.short)
            && opening
                .narration
                .split('.')
                .next()
                .is_some_and(|sentence| sentence.contains(&meeting.name));
        let valid = opening_line.is_some_and(|sheet| {
            !sheet.baseline.trim().is_empty() && opening.dialogue.contains(&sheet.baseline)
        }) && job.is_some_and(|value| opening.narration.contains(value))
            && names_stake
            && names_meeting
            && consequence.is_some_and(|value| opening.narration.contains(value))
            && reward_matches_table
            && includes_reward;
        if !valid {
            failed_seeds.push(seed);
        }
    }
    assert_no_story_rule_failures("opening job, stake, failure and reward", &failed_seeds);
}

#[test]
fn the_foreshadowed_threat_matches_the_fight_choice_for_two_hundred_seeds() {
    let tables = tables();
    let mut failed_seeds = Vec::new();
    for seed in 0..200 {
        let adventure = generated_story_seed(&tables, seed);
        let script = adventure.pack.beat_script.as_ref().unwrap();
        let opening = script.find("opening").unwrap();
        let threat = adventure.facts.get("ant.full").unwrap().to_lowercase();
        let branches_name_threat = opening
            .offers
            .iter()
            .filter(|offer| {
                matches!(
                    offer.effect,
                    crate::gameplay::beat_script::BeatEffect::AskCourier
                        | crate::gameplay::beat_script::BeatEffect::EscortCourier
                )
            })
            .all(|offer| {
                opening
                    .transitions
                    .iter()
                    .find(|transition| {
                        transition.trigger
                            == (crate::gameplay::beat_script::BeatTrigger::Offer {
                                offer: offer.id.clone(),
                            })
                    })
                    .and_then(|transition| script.find(&transition.target))
                    .and_then(|beat| {
                        adventure
                            .pack
                            .scenes
                            .iter()
                            .find(|scene| scene.key == beat.scene)
                    })
                    .is_some_and(|scene| {
                        format!("{} {}", scene.narration, scene.dialogue)
                            .to_lowercase()
                            .contains(&threat)
                    })
            });
        let choice_names_threat = adventure
            .pack
            .offer_labels
            .defend_courier
            .to_lowercase()
            .contains(&threat);
        let threat_appears = adventure
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == crate::gameplay::pack::SceneKey::Combat)
            .is_some_and(|scene| {
                scene
                    .cast
                    .iter()
                    .any(|member| member.name == adventure.pack.bandit.name)
            });
        if !(branches_name_threat && choice_names_threat && threat_appears) {
            failed_seeds.push(seed);
        }
    }
    assert_no_story_rule_failures("foreshadowed and present fight threat", &failed_seeds);
}

#[test]
fn every_private_secret_has_a_clue_and_resolution_payoff_for_two_hundred_seeds() {
    let tables = tables();
    let mut failed_seeds = Vec::new();
    for seed in 0..200 {
        let adventure = generated_story_seed(&tables, seed);
        let secret = adventure.facts.get("secret.fact").unwrap();
        let confirmation = adventure.facts.get("secret.confirm").unwrap();
        let resolution = adventure
            .pack
            .scenes
            .iter()
            .find(|scene| scene.key == crate::gameplay::pack::SceneKey::Inn)
            .unwrap();
        if !adventure.pack.courier_clue.contains(secret)
            || !resolution.dialogue.contains(confirmation)
        {
            failed_seeds.push(seed);
        }
    }
    assert_no_story_rule_failures("secret clue and payoff", &failed_seeds);
}

#[test]
fn a_retrieval_holder_does_not_take_the_recovered_stake_for_two_hundred_seeds() {
    let tables = tables();
    let mut failed_seeds = Vec::new();
    let mut retrievals_checked = 0;
    for seed in 0..200 {
        let adventure = generated_story_seed(&tables, seed);
        if adventure.facts.get("hook.kind").map(String::as_str) != Some("retrieve") {
            continue;
        }
        retrievals_checked += 1;
        let contact = adventure.facts.get("contact.first").unwrap().to_lowercase();
        let stake = adventure.facts.get("stake.short").unwrap().to_lowercase();
        let patron = adventure.facts.get("patron.first").unwrap().to_lowercase();
        let returned_to_patron = adventure
            .pack
            .beat_script
            .as_ref()
            .unwrap()
            .scenes
            .values()
            .any(|scene| {
                format!("{} {}", scene.narration, scene.dialogue)
                    .to_lowercase()
                    .contains(&format!("{contact} returns {stake} to {patron}"))
            });
        let holder_takes_stake = adventure
            .pack
            .beat_script
            .as_ref()
            .unwrap()
            .scenes
            .values()
            .any(|scene| {
                format!("{} {}", scene.narration, scene.dialogue)
                    .to_lowercase()
                    .contains(&format!("{contact} takes {stake}"))
            });
        if holder_takes_stake || !returned_to_patron {
            failed_seeds.push(seed);
        }
    }
    assert!(
        retrievals_checked > 0,
        "seed range must cover retrieval hooks"
    );
    assert_no_story_rule_failures("retrieval holder takes the stake", &failed_seeds);
}

#[test]
fn the_single_pre_fight_scene_offers_observation_or_a_named_threat_for_two_hundred_seeds() {
    let tables = tables();
    let mut failed_seeds = Vec::new();
    for seed in 0..200 {
        let adventure = generated_story_seed(&tables, seed);
        let script = adventure.pack.beat_script.as_ref().unwrap();
        let opening = script.find("opening").unwrap();
        let has_single_multi_option_branch = opening
            .offers
            .iter()
            .filter(|offer| {
                matches!(
                    offer.effect,
                    crate::gameplay::beat_script::BeatEffect::AskCourier
                        | crate::gameplay::beat_script::BeatEffect::EscortCourier
                )
            })
            .all(|offer| {
                let branch = opening
                    .transitions
                    .iter()
                    .find(|transition| {
                        transition.trigger
                            == (crate::gameplay::beat_script::BeatTrigger::Offer {
                                offer: offer.id.clone(),
                            })
                    })
                    .and_then(|transition| script.find(&transition.target));
                branch.is_some_and(|branch| {
                    branch.kind == crate::gameplay::beat_script::BeatKind::Narrate
                        && branch.offers.iter().any(|offer| {
                            offer.effect == crate::gameplay::beat_script::BeatEffect::Observe
                                && script.label(offer).ok().as_deref() == Some("Look around")
                                && branch.transitions.iter().any(|transition| {
                                    transition.trigger
                                        == (crate::gameplay::beat_script::BeatTrigger::Offer {
                                            offer: offer.id.clone(),
                                        })
                                        && transition.target == branch.id
                                })
                        })
                        && branch.offers.iter().any(|offer| {
                            offer.effect == crate::gameplay::beat_script::BeatEffect::DefendCourier
                                && offer.id == "defend-courier"
                                && branch.transitions.iter().any(|transition| {
                                    transition.trigger
                                        == (crate::gameplay::beat_script::BeatTrigger::Offer {
                                            offer: offer.id.clone(),
                                        })
                                        && script.find(&transition.target).is_some_and(|next| {
                                            next.kind
                                                == crate::gameplay::beat_script::BeatKind::Fight
                                        })
                                })
                        })
                })
            });
        if !has_single_multi_option_branch {
            failed_seeds.push(seed);
        }
    }
    assert_no_story_rule_failures("single multi-option pre-fight scene", &failed_seeds);
}
