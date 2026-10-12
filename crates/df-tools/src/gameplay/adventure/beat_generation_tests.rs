use super::super::{beat_script, beat_script_variety_tests as native, journey, model, pack};
use super::campaign_tables::CampaignTables;
use super::*;
use df_protocol::common as rpc;
use std::{collections::BTreeSet, path::Path};
fn tables() -> CampaignTables {
    CampaignTables::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/adventure-tables")
            .as_path(),
    )
    .unwrap()
}
fn generated(tables: &Tables, seed: u64, kind: &str) -> GeneratedAdventure {
    generate(
        tables,
        seed,
        GenerateOptions::default(),
        &ForcedEntries::from([(beat_tables::CHAPTER_TYPE.into(), kind.into())]),
    )
    .unwrap()
}
fn loaded(generated: &GeneratedAdventure) -> pack::LoadedPack {
    pack::load_from_bytes(
        &serde_json::to_vec(&generated.pack).unwrap(),
        &serde_json::to_vec(&generated.tuning).unwrap(),
    )
    .unwrap()
}
#[test]
fn two_hundred_seeds_cover_all_sixteen_data_defined_scripts_settings_slots_and_deterministic_documents()
 {
    let tables = tables();
    let kinds = tables
        .types
        .tables
        .chapter_types
        .entries
        .iter()
        .map(|kind| kind.id.clone())
        .collect::<Vec<_>>();
    let mut seen = BTreeSet::new();
    for seed in 0..200 {
        let kind = &kinds[seed as usize % kinds.len()];
        seen.insert(kind.clone());
        let first = generated(&tables.chapter, seed, kind);
        let second = generated(&tables.chapter, seed, kind);
        let a = first.documents(true).unwrap();
        let b = second.documents(true).unwrap();
        assert_eq!((a.pack, a.tuning, a.record), (b.pack, b.tuning, b.record));
        let canonical = generate(
            &tables.chapter,
            seed,
            GenerateOptions::default(),
            &ForcedEntries::new(),
        )
        .unwrap();
        assert_eq!(
            first.rolls, canonical.rolls,
            "script choice adds no hidden random stream"
        );
        let script = first.pack.beat_script.as_ref().unwrap();
        script.validate(&first.pack).unwrap();
        assert_eq!(
            script.uses_encounter(),
            !["mystery", "parley"].contains(&kind.as_str())
        );
        if !["mystery", "parley"].contains(&kind.as_str()) {
            assert!(script.is_canonical());
        }
        if kind == "mystery" {
            assert!(
                script
                    .beats
                    .iter()
                    .any(|beat| beat.kind == beat_script::BeatKind::Investigate)
            );
        }
        for scene in script.scenes.values() {
            assert_eq!(
                scene.setting.as_ref(),
                Some(first.settings.scene(scene.key))
            );
            let base = first
                .pack
                .scenes
                .iter()
                .find(|base| base.key == scene.key)
                .unwrap();
            assert_eq!(
                (&scene.asset, &scene.speaker_portrait, &scene.cast),
                (&base.asset, &base.speaker_portrait, &base.cast)
            );
            assert!(scene.rules_note.is_empty() && scene.narration_rested.is_none());
            assert!(scene.narration.chars().count() <= 220);
            assert!(!scene.narration.contains(&first.facts["secret.fact"]));
        }
        let sheets = first
            .fact_sheet
            .iter()
            .filter(|sheet| sheet.beat == "scene-flow")
            .collect::<Vec<_>>();
        assert_eq!(sheets.len(), script.labels.len() + 3 * script.scenes.len());
        for sheet in sheets {
            assert!(matches!(
                sheet.slot.as_str(),
                "title" | "narration" | "line" | "beat_label"
            ));
            assert!(sheet.facts.keys().all(|key| !key.starts_with("secret.")));
            assert!(sheet.forbidden.contains(&first.facts["secret.fact"]));
        }
        loaded(&first);
    }
    assert_eq!(seen.len(), 16);
}
#[test]
fn typed_table_refuses_missing_unknown_types_scenes_labels_handlers_patterns_and_fields() {
    let tables = tables();
    let baseline: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../content/adventure-tables/beat-scripts.json"
    ))
    .unwrap();
    for damage in 0..8 {
        let mut value = baseline.clone();
        match damage {
            0 => {
                value["chapter_types"]
                    .as_object_mut()
                    .unwrap()
                    .remove("courier");
            }
            1 => {
                value["chapter_types"]["unknown"] = value["chapter_types"]["courier"].clone();
            }
            2 => {
                value["chapter_types"]["parley"]["script"]["labels"]
                    .as_object_mut()
                    .unwrap()
                    .remove("continue-label");
            }
            3 => {
                value["chapter_types"]["parley"]["scenes"]
                    .as_object_mut()
                    .unwrap()
                    .remove("opening");
            }
            4 => {
                value["chapter_types"]["parley"]["script"]["beats"][1]["kind"] =
                    "unknown-handler".into();
            }
            5 => {
                value["chapter_types"]["parley"]["script"]["beats"][1]["transitions"][0]["target"] =
                    "absent-beat".into();
            }
            6 => {
                value["chapter_types"]["parley"]["scenes"]["opening"]["narration"] =
                    "{secret.fact}".into();
            }
            7 => {
                value["chapter_types"]["parley"]["dc"] = 12.into();
            }
            _ => unreachable!(),
        }
        let error =
            beat_tables::BeatScripts::load(&serde_json::to_vec(&value).unwrap(), &tables.types)
                .unwrap_err();
        assert!(
            matches!(error, TablesError::Invalid { .. }),
            "case {damage}"
        );
        if damage == 4 {
            assert!(error.to_string().contains("opening"));
        }
    }
    assert!(
        generate(
            &tables.chapter,
            0,
            GenerateOptions::default(),
            &ForcedEntries::from([(beat_tables::CHAPTER_TYPE.into(), "unknown".into())])
        )
        .is_err()
    );
}
#[test]
fn old_generated_adventure_and_actual_opening_resume_without_current_table_reinterpretation() {
    let docs = df_persistence::local_demo_scope::DemoAdventure {
        pack: include_bytes!("../fixtures/legacy-adventure/pack.json").to_vec(),
        tuning: include_bytes!("../fixtures/legacy-adventure/tuning.json").to_vec(),
        record: include_bytes!("../fixtures/legacy-adventure/adventure.json").to_vec(),
    };
    let frozen = docs.clone();
    let (loaded, record) = super::record::reload(&docs).unwrap();
    assert!(loaded.pack.beat_script.is_none());
    assert_ne!(record.tables_revision(), tables().chapter.revision);
    assert_eq!(serde_json::to_vec_pretty(&loaded.pack).unwrap(), docs.pack);
    assert_eq!(
        serde_json::to_vec_pretty(&loaded.tuning).unwrap(),
        docs.tuning
    );
    assert_eq!(serde_json::to_vec_pretty(&record).unwrap(), docs.record);
    pack::with_test_pack(loaded, || {
        let codec = df_persistence::NativeCodecLimits {
            maximum_document_bytes: 1024 * 1024,
            maximum_allocated_bytes: 2 * 1024 * 1024,
            maximum_collection_items: 4096,
            maximum_text_bytes: 4096,
        };
        let bytes = include_bytes!("../fixtures/legacy-adventure/opening.bin");
        let mut basis = model::basis().unwrap();
        for _ in 0..5 {
            basis.revision = basis.revision.next_sequence().unwrap();
        }
        let current = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
            bytes,
            basis,
            &model::pins().unwrap(),
            &model::recovery().unwrap(),
            codec,
        )
        .unwrap();
        assert_eq!(
            df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec)
                .unwrap(),
            bytes
        );
        assert_eq!(
            journey::phase(&current).unwrap(),
            rpc::JourneyPhase::Opening
        );
        let next = native::act(&current, native::first(), 6, "escort-courier");
        assert_eq!(
            journey::phase(&native::persisted(&next)).unwrap(),
            rpc::JourneyPhase::Dialogue
        );
    });
    assert_eq!(
        (docs.pack, docs.tuning, docs.record),
        (frozen.pack, frozen.tuning, frozen.record)
    );
}
/// The shared display's choice labels are the phones' authored labels, at every step.
fn assert_labels_agree(current: &df_model::checkpoint::Checkpoint, kind: &str, step: &str) {
    let view = super::super::wire::journey_view(
        current,
        df_persistence::local_demo_scope::LocalDemoRole::Display,
        journey::bootstrap_member().unwrap(),
    )
    .unwrap();
    let Some(rpc::view_message::Audience::Display(view)) = view.audience else {
        panic!("display")
    };
    let display = view.journey.unwrap().table_choices;
    let phone: Vec<String> =
        super::super::beat_script_runtime::view_offers(current, native::first())
            .unwrap()
            .into_iter()
            .map(|(_, label, _)| label)
            .collect();
    assert_eq!(display, phone, "{kind} {step}");
}
/// A peaceful chapter ends through submitted actions only: one resolution, one epilogue, then an
/// end card that accepts nothing. The TV's choice labels are the phones' authored labels.
#[test]
fn peaceful_ending_resolves_once_closes_on_the_end_card_and_shows_the_same_labels_on_tv() {
    let tables = tables();
    for kind in ["mystery", "parley"] {
        let generated = generated(&tables.chapter, 7, kind);
        pack::with_test_pack(loaded(&generated), || {
            let mut current = native::act(&native::prepared(), native::first(), 5, "begin-story");
            assert_labels_agree(&current, kind, "opening");
            if kind == "mystery" {
                current = native::act(&current, native::first(), 6, "look-crates");
                current = native::act(
                    &native::persisted(&current),
                    native::first(),
                    7,
                    "look-desk",
                );
                current = native::act(
                    &native::persisted(&current),
                    native::first(),
                    8,
                    "leave-investigation",
                );
            } else {
                current = native::act(&current, native::first(), 6, "escort-courier");
                current = native::act(
                    &native::persisted(&current),
                    native::first(),
                    7,
                    "continue-conversation",
                );
            }
            assert_labels_agree(&current, kind, "after the story's first step");
            current = native::act(
                &native::persisted(&current),
                native::first(),
                9,
                "choose-harbor-scene",
            );
            assert_labels_agree(&current, kind, "travel");
            let offered = |current: &df_model::checkpoint::Checkpoint| {
                super::super::beat_script_runtime::view_offers(current, native::first())
                    .unwrap()
                    .into_iter()
                    .map(|(_, _, id)| id)
                    .collect::<Vec<_>>()
            };
            let resolve = offered(&current);
            assert_eq!(resolve.len(), 1, "{kind} resolution is the only choice");
            // The server's own admission of a submitted offer accepts what the phone lists.
            assert!(
                super::super::wire::action_offer_rejection(
                    &current,
                    native::first(),
                    rpc::GameplayActionKind::ResolveErrand,
                    &resolve[0],
                    &[]
                )
                .unwrap()
                .is_none(),
                "{kind} resolution must not be refused as stale"
            );
            current = native::act(&current, native::first(), 10, "resolve-errand");
            // The resolution is offered once: the epilogue offers only its close.
            let closing = offered(&current);
            assert_eq!(closing.len(), 1);
            assert!(!closing.contains(&resolve[0]));
            assert_eq!(
                super::super::beat_script::with_script(|script| script
                    .current(current.state())
                    .unwrap()
                    .id
                    .clone()),
                "epilogue"
            );
            // The consumed resolution id no longer matches any offer: a repeat is refused.
            assert!(
                !super::super::beat_script_runtime::matches_id(
                    &current,
                    rpc::GameplayActionKind::ResolveErrand,
                    &resolve[0]
                )
                .unwrap()
            );
            current = native::act(
                &native::persisted(&current),
                native::first(),
                11,
                "close-epilogue",
            );
            let persisted = native::persisted(&current);
            assert_eq!(
                journey::phase(&persisted).unwrap(),
                rpc::JourneyPhase::Complete
            );
            assert!(
                offered(&persisted).is_empty(),
                "{kind} end card offers nothing yet"
            );
            assert_eq!(
                super::super::beat_script::with_script(|script| script
                    .current(persisted.state())
                    .unwrap()
                    .id
                    .clone()),
                "the-end"
            );
            let outcome = super::campaign_completion::terminal_outcome(&persisted)
                .unwrap()
                .unwrap();
            assert_eq!(outcome.result, super::campaign::ChapterResult::Completed);
        });
    }
}
#[test]
fn generated_canonical_investigation_and_parley_use_registered_native_actions_and_honest_outcomes()
{
    let tables = tables();
    for kind in ["courier", "mystery", "parley"] {
        let generated = generated(&tables.chapter, 7, kind);
        pack::with_test_pack(loaded(&generated), || {
            let mut current = native::act(&native::prepared(), native::first(), 5, "begin-story");
            if kind == "courier" {
                current = native::act(&current, native::first(), 6, "escort-courier");
                current = native::act(&current, native::first(), 7, "defend-courier");
                assert_eq!(journey::phase(&current).unwrap(), rpc::JourneyPhase::Combat);
                assert_eq!(current.state().encounters.len(), 1);
            } else {
                if kind == "mystery" {
                    current = native::act(&current, native::first(), 6, "look-crates");
                    current = native::act(
                        &native::persisted(&current),
                        native::first(),
                        7,
                        "look-desk",
                    );
                }
                current = native::act(&current, native::second(), 8, "ask-courier");
                let intent = current.state().intents[0].clone();
                let result = super::super::courier_ai::execute(&current, &intent).unwrap();
                current = super::super::courier_ai::stage_completion(
                    &current,
                    &result,
                    super::super::courier_ai::completion_operation(&intent).unwrap(),
                )
                .unwrap();
                current = native::act(
                    &native::persisted(&current),
                    native::first(),
                    9,
                    "continue-conversation",
                );
                current = native::act(&current, native::first(), 10, "choose-harbor-scene");
                current = native::act(&current, native::first(), 11, "resolve-errand");
                current = native::act(&current, native::first(), 12, "close-epilogue");
                let current = native::persisted(&current);
                let outcome = super::campaign_completion::terminal_outcome(&current)
                    .unwrap()
                    .unwrap();
                assert_eq!(outcome.result, super::campaign::ChapterResult::Completed);
                assert_eq!(outcome.member, journey::hex(native::second().as_bytes()));
                assert!(current.state().encounters.is_empty() && current.state().draws.is_empty());
                assert_eq!(current.state().logical_time.ticks, 0);
                assert!(!outcome.rested && !outcome.antagonist_spared);
            }
        });
    }
}

fn campaign_chapter(
    tables: &CampaignTables,
    kind: &str,
) -> (
    super::campaign::CampaignFrame,
    super::campaign::CampaignRecord,
    super::campaign::ChapterPlan,
    GeneratedAdventure,
) {
    use super::campaign::*;
    for seed in 0..200 {
        let frame = generate_frame(tables, seed, GenerateOptions::default()).unwrap();
        let mut record = CampaignRecord::new(frame.clone()).unwrap();
        loop {
            let mut plan = match next_chapter(tables, &frame, &record) {
                Ok(plan) => plan,
                Err(CampaignError::Complete) => break,
                Err(error) => panic!("campaign fixture: {error}"),
            };
            let generated =
                super::campaign_render::generate_chapter(tables, &frame, &mut plan).unwrap();
            if plan.chapter_type == kind {
                return (frame, record, plan, generated);
            }
            let number = plan.number;
            let peaceful = !generated
                .pack
                .beat_script
                .as_ref()
                .unwrap()
                .uses_encounter();
            record
                .begin(
                    plan,
                    format!("earlier-chapter-{number}"),
                    digest(&generated.documents(true).unwrap().record),
                )
                .unwrap();
            record
                .complete(
                    number,
                    ChapterOutcome {
                        result: if peaceful {
                            ChapterResult::Completed
                        } else {
                            ChapterResult::Victory
                        },
                        approach: Approach::Asked,
                        member: "earlier-member".into(),
                        rested: !peaceful,
                        destination_reached: true,
                        antagonist_spared: !peaceful,
                        campaign_time: 0,
                    },
                )
                .unwrap();
        }
    }
    panic!("no {kind} campaign chapter in bounded deterministic fixtures")
}
fn public_view(current: &df_model::checkpoint::Checkpoint) -> String {
    let view = super::super::wire::journey_view(
        current,
        df_persistence::local_demo_scope::LocalDemoRole::Display,
        journey::bootstrap_member().unwrap(),
    )
    .unwrap();
    let Some(rpc::view_message::Audience::Display(view)) = view.audience else {
        panic!("display")
    };
    format!("{} {}", view.narration, view.journey.unwrap().dialogue)
}
#[test]
fn actual_generated_peaceful_campaign_paths_project_delivered_leads_and_commit_honest_continuity() {
    use super::campaign::*;
    let tables = tables();
    for kind in ["mystery", "parley"] {
        let (frame, before, plan, generated) = campaign_chapter(&tables, kind);
        for asked in [false, true] {
            let mut record = before.clone();
            record
                .begin(
                    plan.clone(),
                    format!("actual-{kind}"),
                    digest(&generated.documents(true).unwrap().record),
                )
                .unwrap();
            pack::with_test_pack(loaded(&generated), || {
                let prepared = native::prepared();
                let mut public = public_view(&prepared);
                let mut current = native::act(&prepared, native::first(), 5, "begin-story");
                public.push_str(&public_view(&current));
                if kind == "mystery" {
                    current = native::act(&current, native::first(), 6, "look-crates");
                    public.push_str(&public_view(&current));
                    current = native::act(
                        &native::persisted(&current),
                        native::first(),
                        7,
                        "look-desk",
                    );
                    public.push_str(&public_view(&current));
                }
                if asked {
                    current = native::act(&current, native::second(), 8, "ask-courier");
                    let intent = current.state().intents[0].clone();
                    let result = super::super::courier_ai::execute(&current, &intent).unwrap();
                    current = super::super::courier_ai::stage_completion(
                        &current,
                        &result,
                        super::super::courier_ai::completion_operation(&intent).unwrap(),
                    )
                    .unwrap();
                    for member in [native::first(), native::second()] {
                        let view = super::super::wire::journey_view(
                            &current,
                            df_persistence::local_demo_scope::LocalDemoRole::Player,
                            member,
                        )
                        .unwrap();
                        let Some(rpc::view_message::Audience::Player(view)) = view.audience else {
                            panic!("player")
                        };
                        assert_eq!(!view.private_clue.is_empty(), member == native::second());
                    }
                    public.push_str(&public_view(&current));
                    current = native::act(
                        &native::persisted(&current),
                        native::first(),
                        9,
                        "continue-conversation",
                    );
                } else if kind == "mystery" {
                    current = native::act(&current, native::first(), 9, "leave-investigation");
                } else {
                    current = native::act(&current, native::first(), 8, "escort-courier");
                    public.push_str(&public_view(&current));
                    current = native::act(&current, native::first(), 9, "continue-conversation");
                }
                public.push_str(&public_view(&current));
                current = native::act(
                    &native::persisted(&current),
                    native::first(),
                    10,
                    "choose-harbor-scene",
                );
                public.push_str(&public_view(&current));
                current = native::act(&current, native::first(), 11, "resolve-errand");
                public.push_str(&public_view(&current));
                current = native::act(&current, native::first(), 12, "close-epilogue");
                public.push_str(&public_view(&current));
                let outcome = super::campaign_completion::terminal_outcome(&current)
                    .unwrap()
                    .unwrap();
                assert_eq!(outcome.result, ChapterResult::Completed);
                assert_eq!(outcome.approach == Approach::Asked, asked);
                if asked {
                    assert_eq!(outcome.member, journey::hex(native::second().as_bytes()));
                }
                assert!(current.state().encounters.is_empty() && current.state().draws.is_empty());
                assert!(!public.contains(&generated.pack.courier_clue));
                assert!(
                    !public.contains(
                        &plan
                            .leads
                            .iter()
                            .find(|lead| lead.vehicle == "private")
                            .unwrap()
                            .text
                    )
                );
                for rescue in &plan.rescued {
                    assert!(
                        public.contains(&rescue.text),
                        "{kind} public rescue {}",
                        rescue.clue_id
                    );
                }
                let contact = plan
                    .leads
                    .iter()
                    .find(|lead| lead.vehicle == "contact")
                    .unwrap();
                assert!(
                    public.contains(&contact.text),
                    "contact receipt must have a visible source"
                );
                assert!(record.complete(plan.number, outcome.clone()).unwrap());
                assert!(!record.complete(plan.number, outcome).unwrap());
                assert!(record.clock_history.iter().all(|advance| advance.chapter != plan.number || advance.reason != "defeat"));
                assert!(record.chapters.last().unwrap().lieutenant_fate.is_none());
                let delivered = record
                    .clues
                    .iter()
                    .find(|clue| clue.clue_id == contact.clue_id)
                    .unwrap();
                assert!(delivered.delivered);
                for lead in &plan.leads {
                    let clue = record
                        .clues
                        .iter()
                        .find(|clue| clue.clue_id == lead.clue_id)
                        .unwrap();
                    assert_eq!(
                        clue.delivered,
                        match lead.vehicle.as_str() {
                            "private" => asked,
                            "contact" => true,
                            "victory" => false,
                            _ => panic!("vehicle"),
                        }
                    );
                }
                let next = next_chapter(&tables, &frame, &record).unwrap();
                assert!(
                    serde_json::to_string(&next.recap)
                        .unwrap()
                        .contains("resolved the chapter")
                );
                record.validate().unwrap();
            });
        }
    }
}

#[test]
fn renamed_initial_variant_preserves_campaign_continuity_and_observation_role_preserves_discovery()
{
    let mut tables = tables();
    let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../content/adventure-tables/beat-scripts.json"
    ))
    .unwrap();
    let mystery = &mut value["chapter_types"]["mystery"];
    let initial = mystery["scenes"]
        .as_object_mut()
        .unwrap()
        .remove("opening")
        .unwrap();
    mystery["scenes"]["renamed-hook"] = initial;
    for beat in mystery["script"]["beats"].as_array_mut().unwrap() {
        if beat["scene_variant"] == "opening" {
            beat["scene_variant"] = "renamed-hook".into();
        }
        if beat["id"] == "observed-crates" {
            beat["scene"] = "dialogue-escort".into();
        }
    }
    mystery["scenes"]["crates"]["scene"] = "dialogue-escort".into();
    let scripts =
        beat_tables::BeatScripts::load(&serde_json::to_vec(&value).unwrap(), &tables.types)
            .unwrap();
    tables.chapter.replace_beat_scripts(scripts);
    let (_, _, plan, generated) = campaign_chapter(&tables, "mystery");
    let script = generated.pack.beat_script.as_ref().unwrap();
    let opening = generated
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == pack::SceneKey::Opening)
        .unwrap();
    assert_eq!(
        (
            &script.scenes["renamed-hook"].narration,
            &script.scenes["renamed-hook"].dialogue
        ),
        (&opening.narration, &opening.dialogue)
    );
    assert!(
        script.scenes["crates"]
            .narration
            .contains("disturbed ground")
    );
    pack::with_test_pack(loaded(&generated), || {
        let mut current = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let mut public = public_view(&current);
        current = native::act(&current, native::first(), 6, "look-crates");
        assert!(public_view(&current).contains("disturbed ground"));
        current = native::act(
            &native::persisted(&current),
            native::first(),
            7,
            "leave-investigation",
        );
        public.push_str(&public_view(&current));
        for rescue in &plan.rescued {
            assert!(public.contains(&rescue.text));
        }
    });
    let mystery = &mut value["chapter_types"]["mystery"];
    for beat in mystery["script"]["beats"].as_array_mut().unwrap() {
        if beat["id"] == "observed-crates" {
            beat["scene_variant"] = "renamed-hook".into();
            beat["scene"] = "opening".into();
        }
    }
    mystery["scenes"].as_object_mut().unwrap().remove("crates");
    let error = beat_tables::BeatScripts::load(&serde_json::to_vec(&value).unwrap(), &tables.types)
        .unwrap_err();
    assert!(error.to_string().contains("observed-crates"));
}

#[test]
fn scripted_arrival_receipts_resume_across_painting_availability_flips_and_reject_forged_assets() {
    use df_model::checkpoint::Checkpoint;
    use prost::Message;
    let tables = tables();
    let codec = df_persistence::NativeCodecLimits {
        maximum_document_bytes: 1024 * 1024,
        maximum_allocated_bytes: 2 * 1024 * 1024,
        maximum_collection_items: 4096,
        maximum_text_bytes: 4096,
    };
    for variant in [false, true] {
        let mut generated = generated(&tables.chapter, 7, "parley");
        if !variant {
            let script = generated.pack.beat_script.as_mut().unwrap();
            let id = script
                .beats
                .iter_mut()
                .find(|beat| beat.id == "peaceful-destination")
                .unwrap()
                .scene_variant
                .take()
                .unwrap();
            script.scenes.remove(&id);
        }
        let documents = generated.documents(true).unwrap();
        let (original, record) = super::record::reload(&documents).unwrap();
        let source_asset = pack::with_test_pack(original.clone(), || {
            let script = original.pack.beat_script.as_ref().unwrap();
            super::super::beat_script_runtime::destination_scene(
                script.find("peaceful-destination").unwrap(),
            )
            .unwrap()
            .scene_asset
        });
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "../../artifacts/tmp/play-e1/painting-flip-{}-{variant}",
            std::process::id()
        ));
        let stem = source_asset.strip_prefix("assets/ui/generated/").unwrap();
        let painting = root.join("generated").join(format!("{stem}.png"));
        std::fs::create_dir_all(painting.parent().unwrap()).unwrap();
        for first_available in [false, true] {
            if first_available {
                std::fs::write(&painting, b"owned synthetic painting presence").unwrap();
            } else if painting.exists() {
                std::fs::remove_file(&painting).unwrap();
            }
            let mut presented = original.clone();
            let substitutions =
                super::startup::substitute_missing_art(&mut presented, &record, &root);
            assert_eq!(presented.pack.beat_script, original.pack.beat_script);
            assert_eq!(presented.pack_revision, original.pack_revision);
            assert_eq!(presented.tuning_revision, original.tuning_revision);
            assert!(!substitutions.is_empty());
            let current: Checkpoint = pack::with_test_pack(presented, || {
                let original_pins =
                    pack::with_test_pack(original.clone(), || model::pins().unwrap());
                assert_eq!(model::pins().unwrap(), original_pins);
                let mut current = native::prepared();
                for (operation, id) in [
                    (5, "begin-story"),
                    (6, "escort-courier"),
                    (7, "continue-conversation"),
                    (8, "choose-harbor-scene"),
                ] {
                    current = native::act(&current, native::first(), operation, id);
                }
                assert_eq!(
                    journey::accepted(current.state().decisions.last().unwrap())
                        .unwrap()
                        .scene
                        .unwrap()
                        .scene_asset,
                    source_asset
                );
                let projected = super::super::beat_script_runtime::scene(current.state()).unwrap();
                assert_eq!(projected.asset == source_asset, first_available);
                current
            });
            let bytes =
                df_persistence::local_demo_scope::encode_owned_demo_checkpoint(&current, codec)
                    .unwrap();
            if first_available {
                std::fs::remove_file(&painting).unwrap();
            } else {
                std::fs::write(&painting, b"owned synthetic painting presence").unwrap();
            }
            let mut resumed = original.clone();
            super::startup::substitute_missing_art(&mut resumed, &record, &root);
            pack::with_test_pack(resumed, || {
                assert_eq!(model::pins().unwrap(), *current.pins());
                let recovered = df_persistence::local_demo_scope::decode_owned_demo_checkpoint(
                    &bytes,
                    current.basis(),
                    &model::pins().unwrap(),
                    &model::recovery().unwrap(),
                    codec,
                )
                .unwrap();
                beat_script::validate_resume(&recovered).unwrap();
                assert_eq!(
                    df_persistence::local_demo_scope::encode_owned_demo_checkpoint(
                        &recovered, codec
                    )
                    .unwrap(),
                    bytes
                );
                let scene = super::super::beat_script_runtime::scene(recovered.state()).unwrap();
                assert_eq!(scene.asset == source_asset, !first_available);
                let view = super::super::wire::journey_view(
                    &recovered,
                    df_persistence::local_demo_scope::LocalDemoRole::Player,
                    native::first(),
                )
                .unwrap();
                let Some(rpc::view_message::Audience::Player(view)) = view.audience else {
                    panic!("player")
                };
                assert_eq!(view.scene.unwrap().scene_asset, scene.asset);
                let mut forged = recovered.state().clone();
                let decision = forged.decisions.last_mut().unwrap();
                let mut accepted = journey::accepted(decision).unwrap();
                accepted.scene.as_mut().unwrap().scene_asset =
                    "assets/ui/generated/unrelated-painting".into();
                decision.semantic_output = Some(journey::hex(&accepted.encode_to_vec()));
                let forged = model::checkpoint(recovered.basis(), forged).unwrap();
                assert!(beat_script::validate_resume(&forged).is_err());
            });
        }
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(generated.documents(true).unwrap().pack, documents.pack);
    }
}
