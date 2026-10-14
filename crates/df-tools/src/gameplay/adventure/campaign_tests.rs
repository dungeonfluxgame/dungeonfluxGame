use super::GenerateOptions;
use super::campaign::*;
use super::campaign_render::generate_chapter;
use super::campaign_tables::CampaignTables;
use crate::gameplay::pack::SceneKey;
use std::path::Path;
fn tables() -> CampaignTables {
    CampaignTables::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap()
}
fn synthetic(n: u8, seed: u64) -> ChapterOutcome {
    ChapterOutcome {
        result: if (seed.wrapping_add(u64::from(n))).is_multiple_of(3) {
            ChapterResult::Defeat
        } else {
            ChapterResult::Victory
        },
        approach: if (seed.wrapping_add(u64::from(n))).is_multiple_of(2) {
            Approach::Asked
        } else {
            Approach::Escorted
        },
        member: if n.is_multiple_of(2) {
            "hero-two"
        } else {
            "hero-one"
        }
        .into(),
        rested: n.is_multiple_of(2),
        destination_reached: !(seed.wrapping_add(u64::from(n))).is_multiple_of(3),
        antagonist_spared: !seed.wrapping_add(u64::from(n)).is_multiple_of(3),
        campaign_time: u64::from(n) * 3600,
    }
}
#[test]
fn two_thousand_frames_have_playable_first_chapters_and_bounded_resolved_prose() {
    let t = tables();
    for seed in 0..2000 {
        let frame = generate_frame(&t, seed, GenerateOptions::default())
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        assert!((6..=10).contains(&frame.outline.len()));
        assert!(balanced(&frame.outline));
        let record = CampaignRecord::new(frame.clone()).unwrap();
        let mut plan = next_chapter(&t, &frame, &record).unwrap();
        assert_eq!(plan.number, 1);
        assert!(!frame.outline[0].skipped);
        generate_chapter(&t, &frame, &mut plan).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let text = String::from_utf8(canonical_bytes(&frame).unwrap()).unwrap();
        assert_resolved(&serde_json::from_str::<serde_json::Value>(&text).unwrap());
    }
}
#[test]
fn two_hundred_campaigns_complete_with_clock_deadlines_and_act_invariants() {
    let t = tables();
    let mut peaceful_types = std::collections::BTreeSet::new();
    for seed in 0..200 {
        let frame = generate_frame(&t, seed, GenerateOptions::default()).unwrap();
        let mut record = CampaignRecord::new(frame.clone()).unwrap();
        let mut previous_clock = 1;
        loop {
            let mut plan = match next_chapter(&t, &frame, &record) {
                Ok(p) => p,
                Err(CampaignError::Complete) => break,
                Err(e) => panic!("seed {seed}: {e}"),
            };
            let generated = generate_chapter(&t, &frame, &mut plan)
                .unwrap_or_else(|e| panic!("seed {seed} chapter {}: {e}", plan.number));
            assert!(generated.pack.scenes.iter().all(|scene| {
                let mut expected_setting = generated.settings.scene(scene.key).clone();
                expected_setting.model = None;
                scene.setting.as_ref() == Some(&expected_setting)
            }));
            let victory_setting = generated
                .pack
                .scenes
                .iter()
                .find(|s| s.key == SceneKey::Victory)
                .unwrap();
            assert!(
                victory_setting
                    .narration
                    .contains(&generated.settings.destination.short)
            );
            let opening_setting = generated
                .pack
                .scenes
                .iter()
                .find(|s| s.key == SceneKey::Opening)
                .unwrap();
            assert!(
                opening_setting
                    .narration
                    .to_lowercase()
                    .contains(&generated.settings.meeting.short.to_lowercase())
            );
            assert_eq!(plan.leads.len(), 3);
            for rescued in &plan.rescued {
                let visible = generated
                    .pack
                    .scenes
                    .iter()
                    .filter(|scene| {
                        matches!(
                            scene.key,
                            super::super::pack::SceneKey::Opening
                                | super::super::pack::SceneKey::DialogueSeal
                                | super::super::pack::SceneKey::DialogueEscort
                        )
                    })
                    .any(|scene| {
                        scene.narration.contains(&rescued.text)
                            || scene.dialogue.contains(&rescued.text)
                    });
                assert!(visible, "rescue must be visible before combat");
            }
            if plan.number > 1 {
                let room = generated
                    .pack
                    .scenes
                    .iter()
                    .find(|scene| scene.key == super::super::pack::SceneKey::Room)
                    .unwrap();
                let first = room.narration.split('.').next().unwrap();
                assert!(first.starts_with("Previously,"));
                assert!(
                    first.contains("won")
                        || first.contains("lost")
                        || first.contains("resolved the chapter")
                );
            }

            for node in &t.acts.tables.nodes.entries {
                let deadline = frame
                    .outline
                    .iter()
                    .find(|s| s.role == node.needed_by)
                    .unwrap()
                    .number;
                if deadline <= plan.number {
                    let clues: Vec<_> = record
                        .clues
                        .iter()
                        .filter(|c| c.node == node.id && c.delivered)
                        .chain(plan.rescued.iter().filter(|c| c.node == node.id))
                        .collect();
                    assert!(
                        clues
                            .iter()
                            .map(|c| &c.clue_id)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            >= 3,
                        "seed {seed} node {} distinct",
                        node.id
                    );
                    assert!(
                        clues
                            .iter()
                            .map(|c| &c.vehicle)
                            .collect::<std::collections::BTreeSet<_>>()
                            .len()
                            >= 2,
                        "seed {seed} node {} vehicles",
                        node.id
                    );
                }
            }
            if plan.number < frame.outline.last().unwrap().number {
                assert!(
                    plan.leads
                        .iter()
                        .any(
                            |c| t.acts.tables.nodes.entries.iter().any(|n| n.id == c.node
                                && frame
                                    .outline
                                    .iter()
                                    .any(|s| s.role == n.needed_by && s.number > plan.number))
                        )
                        || record
                            .clues
                            .iter()
                            .any(|c| c.node == "stronghold" && c.delivered)
                );
            }
            if plan.twist.is_some() {
                let twist = &frame.twists[usize::from(plan.act - 2)];
                for hint in &twist.hints {
                    assert!(record.facts.iter().any(|f| f.text
                        == super::campaign_render::public_text(&frame, hint)
                        && f.audience == Audience::Public));
                }
            }
            for rescued in &plan.rescued {
                assert!(
                    generated
                        .pack
                        .scenes
                        .iter()
                        .any(|s| s.narration.contains(&rescued.text)
                            || s.dialogue.contains(&rescued.text))
                );
            }
            let number = plan.number;
            let peaceful = !generated
                .pack
                .beat_script
                .as_ref()
                .unwrap()
                .uses_encounter();
            if peaceful {
                peaceful_types.insert(plan.chapter_type.clone());
            }
            record
                .begin(
                    plan,
                    format!("db_ch{number}"),
                    digest(&generated.documents(true).unwrap().record),
                )
                .unwrap();
            let mut outcome = synthetic(number, seed);
            if peaceful {
                outcome.result = ChapterResult::Completed;
                outcome.rested = false;
                outcome.antagonist_spared = false;
                outcome.destination_reached = true;
                outcome.campaign_time = 0;
            }
            record.complete(number, outcome.clone()).unwrap();
            if peaceful {
                assert!(
                    record
                        .clock_history
                        .iter()
                        .all(|advance| advance.chapter != number || advance.reason != "defeat")
                );
                assert!(record.chapters.last().unwrap().lieutenant_fate.is_none());
                for lead in &record.chapters.last().unwrap().plan.leads {
                    if lead.vehicle == "victory" {
                        assert!(
                            !record
                                .clues
                                .iter()
                                .find(|clue| clue.clue_id == lead.clue_id)
                                .unwrap()
                                .delivered
                        );
                    }
                }
            }
            assert!(!record.complete(number, outcome).unwrap());
            assert!(record.clock_position >= previous_clock && record.clock_position <= 7);
            previous_clock = record.clock_position;
            assert_eq!(
                record.clock_position,
                (1 + record
                    .clock_history
                    .iter()
                    .map(|a| u32::from(a.delta))
                    .sum::<u32>())
                .min(7) as u8
            );
            record.validate().unwrap();
        }
        assert_eq!(
            record.chapters.len(),
            frame.outline.iter().filter(|s| !s.skipped).count()
        );
        assert_eq!(record.chapters.last().unwrap().plan.chapter_type, "finale");
        let actual: Vec<_> = record
            .chapters
            .iter()
            .map(|c| {
                let mut slot = frame
                    .outline
                    .iter()
                    .find(|s| s.number == c.plan.number)
                    .unwrap()
                    .clone();
                let kind = t
                    .types
                    .tables
                    .chapter_types
                    .entries
                    .iter()
                    .find(|t| t.id == c.plan.chapter_type)
                    .unwrap();
                slot.chapter_type = kind.id.clone();
                slot.pillars = kind.intended;
                slot
            })
            .collect();
        assert!(balanced(&actual));
    }
    assert_eq!(
        peaceful_types,
        std::collections::BTreeSet::from(["mystery".into(), "parley".into()])
    );
}
#[test]
fn histories_diverge_and_private_facts_do_not_enter_public_recap() {
    let t = tables();
    let frame = generate_frame(&t, 42, GenerateOptions::default()).unwrap();
    let mut a = CampaignRecord::new(frame.clone()).unwrap();
    let mut p = next_chapter(&t, &frame, &a).unwrap();
    generate_chapter(&t, &frame, &mut p).unwrap();
    a.begin(p, "db1".into(), "digest".into()).unwrap();
    let mut b = a.clone();
    let mut oa = synthetic(1, 0);
    oa.result = ChapterResult::Victory;
    oa.approach = Approach::Asked;
    oa.destination_reached = true;
    let mut ob = oa.clone();
    ob.result = ChapterResult::Defeat;
    ob.destination_reached = false;
    ob.antagonist_spared = false;
    a.complete(1, oa).unwrap();
    b.complete(1, ob).unwrap();
    let pa = next_chapter(&t, &frame, &a).unwrap();
    let pb = next_chapter(&t, &frame, &b).unwrap();
    assert_ne!(pa.seed, pb.seed);
    assert_ne!(pa.hook_context, pb.hook_context);
    assert!(!pa.recap.private_facts.is_empty());
    for private in pa.recap.private_facts.values().flatten() {
        assert!(!pa.recap.public_facts.contains(private));
    }
    assert!(!pa.recap.latest_sign.contains(&frame.villain.name));
}
#[test]
fn canonical_encoding_and_appendix_seed_are_reproducible() {
    let t = tables();
    let frame = generate_frame(&t, 0x79a5_3da6_463a_aeed, GenerateOptions::default()).unwrap();
    let again = generate_frame(&t, frame.seed, GenerateOptions::default()).unwrap();
    assert_eq!(
        canonical_bytes(&frame).unwrap(),
        canonical_bytes(&again).unwrap()
    );
    let mut golden: serde_json::Value =
        serde_json::from_slice(include_bytes!("fixtures/campaign-appendix-b.json")).unwrap();
    // The tables are editable content and the frame pins their revision; every rolled value
    // must still match the recorded frame.
    golden["tables_revision"] = serde_json::to_value(&frame).unwrap()["tables_revision"].clone();
    assert_eq!(
        canonical_bytes(&frame).unwrap(),
        canonical_bytes(&golden).unwrap()
    );
    assert_eq!(frame.premise.id, "fey-bargain");
    assert_eq!(frame.villain.name, "Aldous");
    assert_eq!(frame.villain.title, "Winter Regent");
    assert_eq!(frame.artifact.id, "tidehorn");
    assert_eq!(frame.region, "the Tarn Marches");
    assert_eq!(frame.patron.first, "Dessa");
    assert_eq!(frame.contact.first, "Rudd");
    let record = CampaignRecord::new(frame).unwrap();
    assert_eq!(
        record.digest().unwrap(),
        digest(&record.canonical_bytes().unwrap())
    );
}

fn assert_resolved(value: &serde_json::Value) {
    match value {
        serde_json::Value::String(s) => assert!(!s.contains(['{', '}']), "unresolved: {s}"),
        serde_json::Value::Array(a) => a.iter().for_each(assert_resolved),
        serde_json::Value::Object(o) => o.values().for_each(assert_resolved),
        _ => {}
    }
}
#[test]
fn malformed_campaign_tables_name_the_source_and_json_path() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../content/adventure-tables");
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/play-g4/table-validation-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    for item in std::fs::read_dir(&source).unwrap() {
        let item = item.unwrap();
        if item.path().is_file() {
            std::fs::copy(item.path(), directory.join(item.file_name())).unwrap();
        }
    }
    for (file, kind) in [
        ("campaign-premises.json", "unknown"),
        ("campaign-premises.json", "clock"),
        ("villains.json", "reference"),
        ("chapter-types.json", "die"),
        ("chapter-types.json", "site"),
        ("chapter-types.json", "stake"),
        ("clues.json", "text"),
        ("twists.json", "placeholder"),
        ("clocks.json", "steps"),
    ] {
        let original = std::fs::read(source.join(file)).unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match kind {
            "unknown" => value["tables"]["premises"]["entries"][0]["unrecognized"] = true.into(),
            "clock" => value["tables"]["premises"]["entries"][0]["clock"] = "missing-clock".into(),
            "reference" => {
                value["tables"]["lieutenants"]["entries"][0]["antagonist"] = "dragon-turtle".into()
            }
            "site" => {
                value["tables"]["chapter_types"]["entries"][0]["beat"]["location_role"] =
                    "nonexistent-site".into()
            }
            "stake" => {
                value["tables"]["chapter_types"]["entries"][0]["beat"]["stake_tags"] =
                    serde_json::json!(["nonexistent-stake"])
            }
            "die" => value["tables"]["chapter_types"]["die"] = 12.into(),
            "text" => value["tables"]["clues"]["omens"]["entries"][0]["p"] = "a".repeat(401).into(),
            "placeholder" => {
                value["tables"]["twists"]["entries"][0]["reveal"] = "Unknown {not.a.slot}".into()
            }
            "steps" => value["tables"]["clocks"]["creeping-dark"]["steps"][0]["step"] = 2.into(),
            _ => unreachable!(),
        }
        std::fs::write(directory.join(file), serde_json::to_vec(&value).unwrap()).unwrap();
        let error = CampaignTables::load(&directory).err().unwrap();
        let message = error.to_string();
        assert!(message.contains(file), "{message}");
        assert!(message.contains("$.tables"), "{message}");
        std::fs::write(directory.join(file), original).unwrap();
    }
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn semantic_history_is_independent_of_database_names_and_document_digests() {
    let t = tables();
    let frame = generate_frame(&t, 42, GenerateOptions::default()).unwrap();
    let mut a = CampaignRecord::new(frame.clone()).unwrap();
    let mut plan = next_chapter(&t, &frame, &a).unwrap();
    generate_chapter(&t, &frame, &mut plan).unwrap();
    let mut b = a.clone();
    a.begin(plan.clone(), "database-a".into(), "digest-a".into())
        .unwrap();
    b.begin(plan, "database-b".into(), "digest-b".into())
        .unwrap();
    let outcome = synthetic(1, 42);
    a.complete(1, outcome.clone()).unwrap();
    b.complete(1, outcome).unwrap();
    assert_eq!(
        next_chapter(&t, &frame, &a).unwrap().seed,
        next_chapter(&t, &frame, &b).unwrap().seed
    );
    let mut tampered = a.clone();
    tampered.clock_position = 7;
    assert!(tampered.validate().is_err());
}

#[test]
fn previously_uses_only_delivered_public_history_and_pattern_sheet_matches_actual_secret() {
    let t = tables();
    for seed in 0..20 {
        let frame = generate_frame(&t, seed, GenerateOptions::default()).unwrap();
        let mut record = CampaignRecord::new(frame.clone()).unwrap();
        let mut first = next_chapter(&t, &frame, &record).unwrap();
        let generated = generate_chapter(&t, &frame, &mut first).unwrap();
        let victory = first.leads.iter().find(|c| c.vehicle == "victory").unwrap();
        assert!(victory.text.contains(&frame.villain.signature));
        let private = first.leads.iter().find(|c| c.vehicle == "private").unwrap();
        assert_eq!(&generated.facts["secret.fact"], &private.text);
        assert!(!generated.facts.contains_key("secret.id"));
        assert!(
            generated
                .fact_sheet
                .iter()
                .find(|s| s.slot == "clue")
                .unwrap()
                .baseline
                .contains(&private.text)
        );
        record.begin(first, "db".into(), "digest".into()).unwrap();
        let mut outcome = synthetic(1, seed);
        outcome.result = ChapterResult::Defeat;
        outcome.antagonist_spared = false;
        outcome.destination_reached = false;
        outcome.approach = Approach::Asked;
        record.complete(1, outcome).unwrap();
        let mut second = next_chapter(&t, &frame, &record).unwrap();
        for lead in &second.recap.open_leads {
            assert!(
                record
                    .clues
                    .iter()
                    .any(|c| c.delivered && c.audience == Audience::Public && c.text == *lead)
            );
        }
        let generated = generate_chapter(&t, &frame, &mut second).unwrap();
        let room = generated
            .pack
            .scenes
            .iter()
            .find(|s| s.key == super::super::pack::SceneKey::Room)
            .unwrap();
        for clue in second.leads.iter().filter(|c| c.vehicle == "victory") {
            if !record.clues.iter().any(|old| {
                old.delivered && old.audience == Audience::Public && old.text == clue.text
            }) && !second
                .rescued
                .iter()
                .any(|rescued| rescued.text == clue.text)
            {
                assert!(!room.dialogue.contains(&clue.text));
                assert!(!room.narration.contains(&clue.text));
            }
        }
    }
}
