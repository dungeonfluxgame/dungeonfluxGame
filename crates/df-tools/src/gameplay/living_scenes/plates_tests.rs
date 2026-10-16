use std::path::PathBuf;

use tokio::time::Instant;

use super::{
    super::{
        super::{
            adventure::setting::{SceneSetting, SettingKind},
            imagery::{ImageKey, Imagery},
            scene_mood::{Expression, HealthTier},
            social::SocialLedger,
            startup_configuration::{ContentRating, ImageryOff},
        },
        State, Timing,
    },
    *,
};

fn key(byte: char) -> ImageKey {
    ImageKey::parse(&byte.to_string().repeat(64)).unwrap()
}

#[test]
fn failed_canonical_dependency_projects_terminal_plate_failure_without_redispatch() {
    use crate::gameplay::adventure::scene_model::SceneModel;
    let (shared, mut queue) = shared(16);
    shared
        .inner
        .observe_checkpoint(&crate::gameplay::journey::initial().unwrap());
    let mut authored = scene(SceneKey::DialogueSeal, "Failed room", Some("Kestrel"));
    authored.setting.as_mut().unwrap().model = Some(SceneModel {
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: Vec::new(),
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    });
    let viewed = view(&authored, &[], false);
    shared.observe_with(&viewed, Some(authored.clone()), Vec::new());
    assert!(drain(&mut queue).is_empty());
    let jobs = locked(&shared.inner.state).canonical.take_jobs();
    assert_eq!(jobs.len(), 1);
    for job in jobs {
        let super::super::Job::Canonical(work) = job else {
            panic!("canonical job");
        };
        shared.inner.finish(super::super::Done::Canonical {
            work,
            result: Err(ImageFailure::Provider),
        });
    }
    shared.observe_with(&viewed, Some(authored.clone()), Vec::new());
    let mut message = display_message(&authored);
    apply(&shared, &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = message.audience else {
        panic!("display view");
    };
    assert_eq!(
        display.scene.unwrap().plate_state,
        rpc::PlateState::Failed as i32
    );
    shared.observe_with(&viewed, Some(authored), Vec::new());
    assert!(drain(&mut queue).is_empty());
    assert!(locked(&shared.inner.state).canonical.take_jobs().is_empty());
}

#[tokio::test]
async fn authored_room_does_not_substitute_static_npc_or_hero_art_for_missing_bindings() {
    use crate::gameplay::adventure::scene_model::SceneModel;

    let (shared, _) = shared(8);
    let mut authored = scene(SceneKey::DialogueSeal, "Bound room", Some("Kestrel"));
    authored.setting.as_mut().unwrap().model = Some(SceneModel {
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: Vec::new(),
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    });
    let plate = PlateScene::of(&authored).unwrap();
    let keeper = &plate.identity_cast[0];

    assert!(matches!(
        shared
            .npc_art(&plate, "keeper", &keeper.sprite, &keeper.portrait)
            .await,
        Err(ImageFailure::InvalidRequest)
    ));
    assert!(matches!(
        shared.canonical_hero_identity(&plate, 1).await,
        Err(ImageFailure::InvalidRequest)
    ));
}

#[test]
fn exact_story_variant_with_missing_canonical_bindings_is_not_dispatched() {
    let (shared, mut queue) = shared(8);
    let mut authored = scene(SceneKey::Opening, "Bound opening", Some("Kestrel"));
    authored.source_variant_id = Some("bound-opening".to_owned());
    let plate = PlateScene::of(&authored).unwrap();
    assert!(plate.requires_canonical_identity());

    let mut book = Book::new();
    shared.ensure(&mut book, &plate, &[1], &[], 0, false);

    assert!(
        book.entries
            .values()
            .any(|entry| matches!(entry, Entry::Failed))
    );
    assert!(drain(&mut queue).is_empty());
}

#[test]
fn authored_plate_waits_until_all_canonical_references_are_ready() {
    use crate::gameplay::adventure::scene_model::{SceneModel, ScenePerson, StartingAttitude};

    let (shared, mut queue) = shared(8);
    shared
        .inner
        .observe_checkpoint(&crate::gameplay::journey::initial().unwrap());
    let mut authored = scene(SceneKey::Opening, "Bound opening", Some("Kestrel"));
    authored.setting.as_mut().unwrap().model = Some(SceneModel {
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: vec![ScenePerson {
            id: "keeper".to_owned(),
            name: "Kestrel".to_owned(),
            role: "shopkeeper".to_owned(),
            description: "A middle-aged shopkeeper with gray hair and a plain wool tunic"
                .to_owned(),
            relation_to_speaker: String::new(),
            position: "behind the counter".to_owned(),
            aliases: Vec::new(),
            is_background: false,
            present_in: Vec::new(),
            attitude: StartingAttitude::Indifferent,
            attitudes: Default::default(),
            knowledge: Vec::new(),
            wants: String::new(),
            coin_cp: 0,
        }],
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    });
    let plate = PlateScene::of(&authored).unwrap();
    let mut book = Book::new();

    shared.ensure(&mut book, &plate, &[], &[], 0, false);

    assert!(book.entries.is_empty());
    assert!(drain(&mut queue).is_empty());
    assert_eq!(locked(&shared.inner.state).canonical.take_jobs().len(), 2);
}

#[tokio::test]
async fn canonical_plate_and_locator_use_generated_speaker_bytes_after_readiness() {
    use crate::gameplay::{
        adventure::scene_model::{SceneModel, ScenePerson, StartingAttitude},
        imagery::tests::DirectionFixture,
    };

    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/canonical-reference-tests/speaker-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::photographic(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    inner.observe_checkpoint(&crate::gameplay::journey::initial().unwrap());
    let mut authored = scene(SceneKey::DialogueSeal, "Canonical room", Some("Kestrel"));
    authored.setting.as_mut().unwrap().model = Some(SceneModel {
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: vec![ScenePerson {
            id: "keeper".to_owned(),
            name: "Kestrel".to_owned(),
            role: "shopkeeper".to_owned(),
            description: "A middle-aged shopkeeper with gray hair and a plain wool tunic"
                .to_owned(),
            relation_to_speaker: String::new(),
            position: "behind the counter".to_owned(),
            aliases: Vec::new(),
            is_background: false,
            present_in: Vec::new(),
            attitude: StartingAttitude::Indifferent,
            attitudes: Default::default(),
            knowledge: Vec::new(),
            wants: String::new(),
            coin_cp: 0,
        }],
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    });
    locked(&inner.state).canonical.observe_room(&authored);
    let (jobs, _queue) = mpsc::channel(8);
    let shared = Shared {
        inner: inner.clone(),
        book: Mutex::new(Book::new()),
        jobs,
    };
    let plate = PlateScene::of(&authored).unwrap();
    let queued = locked(&inner.state).canonical.take_jobs();
    let mut tasks = JoinSet::new();
    for job in queued {
        inner.spawn(&mut tasks, job);
    }
    while let Some(done) = tasks.join_next().await {
        inner.finish(done.unwrap());
    }

    let canonical = inner.canonical_npc_reference("keeper").await.unwrap();
    let (speaker, face) = shared
        .npc_art(
            &plate,
            "keeper",
            "sprite-patron-bell-ringer",
            "portrait-patron-bell-ringer",
        )
        .await
        .unwrap();
    assert_eq!(speaker.unwrap().sha256, canonical.sha256);
    assert!(face.is_none());
    assert_eq!(
        shared.locator_references(&plate, &[]).await.unwrap()["keeper"].sha256,
        canonical.sha256
    );
    assert_ne!(
        canonical.sha256,
        inner
            .references
            .load_photographic("sprite-patron-bell-ringer")
            .unwrap()
            .sha256
    );
}

#[tokio::test]
async fn generated_canonical_room_and_performer_bytes_reach_the_real_photographic_plate_request() {
    use crate::gameplay::{
        adventure::scene_model::{SceneKnowledge, SceneModel, ScenePerson, StartingAttitude},
        imagery::tests::DirectionFixture,
    };
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/canonical-reference-tests/plate-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::photographic(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    inner.observe_checkpoint(&crate::gameplay::journey::initial().unwrap());
    let mut authored = scene(
        SceneKey::DialogueSeal,
        "Canonical workshop",
        Some("Kestrel"),
    );
    authored.setting.as_mut().unwrap().model = Some(SceneModel {
        inventory_version: 1,
        time_of_day: "morning".to_owned(),
        weather: "clear".to_owned(),
        people: vec![ScenePerson {
            id: "keeper".to_owned(),
            name: "Kestrel".to_owned(),
            role: "shopkeeper".to_owned(),
            description: "An adult with gray hair and a plain wool tunic".to_owned(),
            relation_to_speaker: String::new(),
            position: "behind the counter".to_owned(),
            aliases: Vec::new(),
            is_background: false,
            present_in: Vec::new(),
            attitude: StartingAttitude::Indifferent,
            attitudes: Default::default(),
            knowledge: vec![SceneKnowledge {
                fact_id: "private".to_owned(),
                statement: "PRIVATE_DO_NOT_SHARE".to_owned(),
            }],
            wants: "PRIVATE_DO_NOT_SHARE".to_owned(),
            coin_cp: 0,
        }],
        things: Vec::new(),
        ways_out: Vec::new(),
        offers: Vec::new(),
    });
    locked(&inner.state).canonical.observe_room(&authored);
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Shared {
        inner: inner.clone(),
        book: Mutex::new(Book::new()),
        jobs,
    };
    let plate = PlateScene::of(&authored).unwrap();
    shared.ensure(&mut locked(&shared.book), &plate, &[], &[], 1, false);
    assert!(
        queue.try_recv().is_err(),
        "a pending identity must never admit a static substitute plate"
    );
    let queued = locked(&inner.state).canonical.take_jobs();
    let mut tasks = JoinSet::new();
    for job in queued {
        inner.spawn(&mut tasks, job);
    }
    while let Some(done) = tasks.join_next().await {
        inner.finish(done.unwrap());
    }
    let room = inner
        .canonical_room_reference(&plate.place_id)
        .await
        .unwrap();
    let performer = inner.canonical_npc_reference("keeper").await.unwrap();
    let (sheet, _) = shared.identity_sheet(&plate, &[]).await.unwrap();
    let spec = plate.stage(&[]).unwrap();
    let (request, _) = shared
        .photographic_request(&plate, &spec, &[], None)
        .await
        .unwrap();
    assert_eq!(request.references[0].sha256, room.sha256);
    assert_eq!(request.references[1].sha256, sheet.sha256);
    assert_eq!(request.references[2].sha256, performer.sha256);
    assert!(!request.prompt.as_str().contains("PRIVATE_DO_NOT_SHARE"));
    fixture.imagery.render(request).await.unwrap();
    assert_eq!(
        fixture.calls(),
        3,
        "room, performer and composed scene use the real image boundary"
    );
}

fn member(id: &str, name: &str, sprite: &str, role: pack::CastRole) -> pack::SceneCastMember {
    pack::SceneCastMember {
        id: id.to_owned(),
        name: name.to_owned(),
        role,
        sprite: format!("assets/ui/generated/{sprite}"),
        portrait: format!(
            "assets/ui/generated/{}",
            sprite.replace("sprite-", "portrait-")
        ),
        voice: "warm".to_owned(),
        gender: Some(Gender::She),
        age: Some(Age::Adult),
    }
}

fn scene(key: SceneKey, title: &str, speaker: Option<&str>) -> pack::Scene {
    pack::Scene {
        source_variant_id: None,
        key,
        title: title.to_owned(),
        narration: "Lamplight and quiet.".to_owned(),
        rules_note: String::new(),
        cast: vec![member(
            "keeper",
            "Kestrel",
            "sprite-patron-bell-ringer",
            pack::CastRole::Patron,
        )],
        narration_rested: None,
        speaker: speaker.unwrap_or_default().to_owned(),
        dialogue: speaker
            .map(|_| "Will you carry the word?".to_owned())
            .unwrap_or_default(),
        asset: "assets/ui/generated/scene-place-bell-chamber.webp".to_owned(),
        setting: Some(SceneSetting {
            place_id: "place-bell-chamber".to_owned(),
            name: "The Bell Chamber".to_owned(),
            short: "a high chamber under the bells".to_owned(),
            kind: SettingKind::BellChamber,
            exterior: false,
            art_key: "bell-chamber".to_owned(),
            model: None,
        }),
        speaker_portrait: String::new(),
        findings: Vec::new(),
    }
}

#[test]
fn canonical_lookup_uses_exact_authored_scene_variant() {
    const TITLE: &str = "Marek’s Cloth Shop";
    const ASSET: &str = "assets/ui/generated/scene-mareks-cloth-shop";
    let load_shop = || {
        pack::load_from_bytes(
            include_bytes!("../../../../../content/merchant-shop/pack.json"),
            include_bytes!("../../../../../content/merchant-shop/tuning.json"),
        )
        .unwrap()
    };
    for (label, expected) in [("room", SceneKey::Room), ("opening", SceneKey::Opening)] {
        let loaded = load_shop();
        pack::with_test_pack(loaded, || {
            let scene = locate(TITLE, ASSET, label, "").expect("base scene key resolves");
            assert_eq!(scene.key, expected);
            assert_eq!(scene.title, TITLE);
            assert_eq!(scene.asset, ASSET);
        });
    }

    let loaded = load_shop();
    pack::with_test_pack(loaded, || {
        assert_eq!(
            locate(TITLE, ASSET, "", ""),
            Err(LookupFailure::InvalidSceneKey)
        );
        assert!(locate(TITLE, ASSET, "Opening", "").is_err());
        assert!(locate(TITLE, ASSET, "dialogue-seal", "").is_err());
    });

    let mut ambiguous = load_shop();
    let mut opening = ambiguous
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::Opening)
        .unwrap()
        .clone();
    opening.source_variant_id = Some("duplicate-opening".to_owned());
    ambiguous
        .pack
        .beat_script
        .as_mut()
        .unwrap()
        .scenes
        .insert("duplicate-opening".to_owned(), opening);
    pack::with_test_pack(ambiguous, || {
        let base = locate(TITLE, ASSET, "opening", "").expect("base opening is exact");
        assert_eq!(base.source_variant_id, None);
        let variant = locate(TITLE, ASSET, "opening", "duplicate-opening")
            .expect("exact variant resolves despite matching title and art");
        assert_eq!(
            variant.source_variant_id.as_deref(),
            Some("duplicate-opening")
        );
        assert_eq!(
            locate(TITLE, ASSET, "opening", "missing-opening"),
            Err(LookupFailure::MissingSceneVariant)
        );
        assert_eq!(
            locate(TITLE, ASSET, "room", "duplicate-opening"),
            Err(LookupFailure::SceneVariantMismatch)
        );
        assert_eq!(
            locate(TITLE, ASSET, "opening", "Duplicate-opening"),
            Err(LookupFailure::InvalidSceneVariant)
        );
    });
}

#[tokio::test]
async fn displayed_shop_opening_queues_its_plate_when_room_shares_title_and_art() {
    let loaded =
        pack::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/merchant-shop"))
            .unwrap();
    let opening = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::Opening)
        .unwrap()
        .clone();
    let (shared, mut queue) = shared(16);
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let message = display_message(&opening);

    pack::with_test_pack(loaded, || plates.observe(&message, 1));

    let queued = drain(&mut queue);
    assert!(
        queued.iter().any(|job| matches!(
            job,
            Job::Fresh {
                scene,
                ahead: false,
                ..
            } if scene.key == SceneKey::Opening
        )),
        "the exact Opening source key must resolve and admit the current scene plate"
    );
    assert_eq!(
        locked(&shared.book)
            .current
            .as_ref()
            .map(|current| current.scene_key.as_str()),
        Some("opening|Marek’s Cloth Shop|assets/ui/generated/scene-mareks-cloth-shop")
    );
}

fn view(scene: &pack::Scene, heroes: &[u8], in_combat: bool) -> View {
    let source_scene_variant = scene.source_variant_id.clone().unwrap_or_default();
    View {
        scene_key: projection_scene_identity(
            scene.key.source_label(),
            &source_scene_variant,
            &scene.title,
            &scene.asset,
        ),
        source_scene_key: scene.key.source_label().to_owned(),
        source_scene_variant,
        title: scene.title.clone(),
        asset: scene.asset.clone(),
        in_combat,
        in_roll: false,
        graph_revision: 1,
        heroes: heroes.to_vec(),
        story: None,
        hero_names: heroes
            .iter()
            .map(|art| (*art, format!("Hero {art}")))
            .collect(),
    }
}

#[test]
fn later_plate_edits_do_not_inherit_the_original_scene_request_latency() {
    let mut first = None;
    assert_eq!(
        projection_request_latency(&mut first, Some(6_324), Some("original"), Some("original")),
        Some(6_324)
    );
    assert_eq!(
        projection_request_latency(&mut first, Some(56_801), Some("original"), Some("original")),
        Some(6_324)
    );
    assert_eq!(
        projection_request_latency(&mut first, Some(578_302), Some("edited"), Some("original")),
        None
    );
    assert_eq!(
        projection_request_latency(&mut first, Some(578_302), None, Some("original")),
        None
    );
    assert_eq!(first, Some(6_324));
}

#[test]
fn authored_story_reads_only_committed_npc_reply_after_the_scene_action_boundary() {
    let mut journey = rpc::JourneyView {
        table_log: vec![
            rpc::TableLine {
                sequence: 1,
                role: rpc::TableRole::Npc as i32,
                kind: rpc::TableKind::Speech as i32,
                speaker: "Courier".into(),
                text: "Earlier scene".into(),
                ..Default::default()
            },
            rpc::TableLine {
                sequence: 2,
                role: rpc::TableRole::Hero as i32,
                kind: rpc::TableKind::Action as i32,
                text: "Enter the quay".into(),
                ..Default::default()
            },
            rpc::TableLine {
                sequence: 3,
                role: rpc::TableRole::Hero as i32,
                kind: rpc::TableKind::Speech as i32,
                text: "Player instructions must not become image context".into(),
                ..Default::default()
            },
        ],
        dialogue: "partial reply streaming".into(),
        ..Default::default()
    };
    assert!(committed_story(&journey).is_none());
    journey.table_log.push(rpc::TableLine {
        sequence: 4,
        role: rpc::TableRole::Npc as i32,
        kind: rpc::TableKind::Speech as i32,
        speaker: "Courier".into(),
        text: "Keep your voice low.".into(),
        ..Default::default()
    });
    let first = committed_story(&journey).unwrap();
    assert_eq!(first.sequence, 4);
    assert_eq!(first.text, "Keep your voice low.");
    journey.table_log.last_mut().unwrap().sequence = 5;
    assert_eq!(committed_story(&journey).unwrap().sha256, first.sha256);
}

#[tokio::test]
async fn committed_stock_dialogue_reframes_actual_goods_with_room_and_cast_references() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/narrative-stock-focus/{}",
        std::process::id()
    ));
    // The photographic path builds a contact sheet from canonical cutouts, so this fake must
    // return decodable image bytes rather than the opaque image payload used by `new`.
    let fixture = DirectionFixture::photographic(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Arc::new(Shared {
        inner: Arc::new(Inner {
            imagery: fixture.imagery.clone(),
            references: super::super::ReferenceStore::photographic(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
            ),
            social: SocialLedger::none(),
            rating: ContentRating::Mature,
            timing,
            state: Mutex::new(State::new(timing)),
            wake: tokio::sync::watch::channel(0).0,
            origin: Instant::now(),
        }),
        book: Mutex::new(Book::new()),
        jobs,
    });
    let mut current = scene(SceneKey::Opening, "The cloth shop", Some("Marek"));
    let mut model = crate::gameplay::narrative_focus::tests::shop();
    model.people[0].id = "merchant".into();
    for offer in &mut model.offers {
        offer.seller_id = "merchant".into();
    }
    current.cast = model
        .people
        .iter()
        .enumerate()
        .map(|(index, person)| {
            member(
                &person.id,
                &person.name,
                match index {
                    0 => "sprite-patron-merchant",
                    1 => "sprite-shop-adventurer",
                    _ => "sprite-shop-soldier",
                },
                if index == 0 {
                    pack::CastRole::Patron
                } else {
                    pack::CastRole::Bystander
                },
            )
        })
        .collect();
    current.cast[0].id = "merchant".into();
    current.cast[0].name = "Marek".into();
    current.setting.as_mut().unwrap().place_id = "cloth-shop".into();
    current.setting.as_mut().unwrap().model = Some(model);
    let before = current.setting.as_ref().unwrap().model.clone().unwrap();
    shared
        .inner
        .observe_checkpoint(&crate::gameplay::beat_script_tests::prepared_story());
    locked(&shared.inner.state).canonical.observe_room(&current);
    loop {
        let canonical_jobs = locked(&shared.inner.state).canonical.take_jobs();
        if canonical_jobs.is_empty() {
            break;
        }
        for job in canonical_jobs {
            let super::super::Job::Canonical(work) = job else {
                panic!("canonical reference job")
            };
            let result = shared.inner.render_canonical(&work).await;
            shared
                .inner
                .finish(super::super::Done::Canonical { work, result });
        }
    }
    let current_room_revision =
        super::super::canonical::room_revision(&before).expect("authored room revision");
    let mut alternate = current.clone();
    let mut alternate_model = before.clone();
    alternate_model.weather = "overcast".into();
    let alternate_room_revision =
        super::super::canonical::room_revision(&alternate_model).expect("alternate room revision");
    assert_ne!(current_room_revision, alternate_room_revision);
    alternate.setting.as_mut().unwrap().model = Some(alternate_model);
    locked(&shared.inner.state)
        .canonical
        .observe_room(&alternate);
    // `finish` reacquires the canonical state mutex, so take the jobs into a local first.
    let failed_alternate_jobs = locked(&shared.inner.state).canonical.take_jobs();
    assert_eq!(
        failed_alternate_jobs.len(),
        1,
        "the changed authored room revision admits one canonical room job"
    );
    for job in failed_alternate_jobs {
        let super::super::Job::Canonical(work) = job else {
            panic!("canonical reference job")
        };
        shared.inner.finish(super::super::Done::Canonical {
            work,
            result: Err(ImageFailure::Provider),
        });
    }
    assert_eq!(
        shared
            .inner
            .canonical_room_status_at("cloth-shop", &current_room_revision),
        super::super::canonical::ReferenceStatus::Ready
    );
    assert_eq!(
        shared
            .inner
            .canonical_room_status_at("cloth-shop", &alternate_room_revision),
        super::super::canonical::ReferenceStatus::Failed(ImageFailure::Provider)
    );
    let initial = view(&current, &[1], false);
    shared.observe_with(&initial, Some(current.clone()), Vec::new());
    let Job::Fresh {
        id,
        scene,
        spec,
        heroes,
        faces,
        ..
    } = drain(&mut queue).remove(0)
    else {
        panic!("initial plate")
    };
    let (base, placement, _) = shared
        .render_fresh(&scene, &spec, &heroes, &faces, &id, 1, false)
        .await
        .unwrap();
    locked(&shared.book).insert(
        id,
        Entry::Ready {
            key: base.clone(),
            placement,
        },
    );

    let mut journey = rpc::JourneyView::default();
    journey.table_log.push(public_npc_line(
        42,
        &current.cast[0],
        "The fine clothes are fifteen hundred copper.",
    ));
    let mut focused = view(&current, &[1], false);
    focused.graph_revision = 2;
    focused.story = committed_story(&journey);
    shared.observe_with(&focused, Some(current.clone()), Vec::new());
    let eligible_at = {
        let book = locked(&shared.book);
        book.state_images
            .next_eligible_at(shared.inner.timing.interval, book.face_refresh.last_started)
            .expect("focus edit is pending")
    };
    shared.poll_state_images_at(shared.inner.now().max(eligible_at));
    let Job::State(job) = drain(&mut queue).remove(0) else {
        panic!("narrative reveal through production observer")
    };
    let state_images::Job::Bake {
        pending,
        base: shown,
        identities,
        ..
    } = &job
    else {
        panic!("focus bake")
    };
    assert_eq!(shown, &base);
    assert_eq!(pending.focus.as_ref().unwrap().subject.id(), "clothes-fine");
    let Lookup::Image { bytes, format } = fixture.imagery.image(base.as_str()).await else {
        panic!("base image")
    };
    let base_reference = ReferenceImage::new(format, bytes);
    let request = shared
        .state_image_request(pending, base_reference.clone(), identities.as_deref())
        .await
        .unwrap();
    assert_eq!(request.references.len(), 4);
    assert_eq!(request.references[0].sha256, base_reference.sha256);
    assert_eq!(
        request.references[1].sha256,
        shared
            .inner
            .canonical_room_reference_at("cloth-shop", &current_room_revision)
            .await
            .unwrap()
            .sha256
    );
    assert!(
        request
            .prompt
            .as_str()
            .contains("Image 3 is the canonical photographic identity contact sheet")
    );
    assert!(request.prompt.as_str().contains("npc:merchant"));
    assert!(request.prompt.as_str().contains("hero:"));
    for actor in &current.cast {
        assert!(
            request
                .prompt
                .as_str()
                .contains(&format!("npc:{}", actor.id))
        );
    }
    let canonical_speaker = shared
        .inner
        .canonical_npc_reference("merchant")
        .await
        .unwrap();
    assert_eq!(request.references[3].sha256, canonical_speaker.sha256);
    assert_ne!(
        request.references[3].sha256,
        shared
            .inner
            .references
            .load_photographic("sprite-patron-merchant")
            .unwrap()
            .sha256
    );
    let mut ordinary_story = (*pending).clone();
    ordinary_story.focus = None;
    let ordinary_story_request = shared
        .state_image_request(
            &ordinary_story,
            base_reference.clone(),
            identities.as_deref(),
        )
        .await
        .unwrap();
    assert_eq!(ordinary_story_request.references.len(), 4);
    assert_eq!(
        ordinary_story_request.references[1].sha256,
        request.references[1].sha256
    );
    assert_eq!(
        ordinary_story_request.references[2].sha256,
        request.references[2].sha256
    );
    assert_eq!(
        ordinary_story_request.references[3].sha256,
        canonical_speaker.sha256
    );
    assert!(
        ordinary_story_request
            .prompt
            .as_str()
            .contains("Image 2 is the canonical photograph")
    );
    assert!(
        ordinary_story_request
            .prompt
            .as_str()
            .contains("Image 4 is the canonical photographic identity reference")
    );

    ordinary_story.story = None;
    let ordinary_state_request = shared
        .state_image_request(
            &ordinary_story,
            base_reference.clone(),
            identities.as_deref(),
        )
        .await
        .unwrap();
    assert_eq!(ordinary_state_request.references.len(), 4);
    assert_eq!(
        ordinary_state_request.references[1].sha256,
        request.references[1].sha256
    );
    assert_eq!(
        ordinary_state_request.references[2].sha256,
        request.references[2].sha256
    );
    assert_eq!(
        ordinary_state_request.references[3].sha256,
        canonical_speaker.sha256
    );
    assert!(
        ordinary_state_request
            .prompt
            .as_str()
            .contains("Image 2 is the canonical photograph")
    );
    assert!(
        ordinary_state_request
            .prompt
            .as_str()
            .contains("Image 4 is the canonical photographic identity reference")
    );
    assert!(request.prompt.as_str().contains("clothes-fine"));
    assert!(request.prompt.as_str().contains("1500"));
    assert!(request.prompt.as_str().contains("no ownership transfer"));
    let rendered = shared.render_state_image(job).await;
    assert!(matches!(
        &rendered,
        state_images::Done::Bake { result: Ok(_), .. }
    ));
    shared.finish_state_image_at(rendered, Duration::from_secs(5));
    let mut projection = rpc::GameplayScene {
        title: current.title.clone(),
        scene_asset: current.asset.clone(),
        ..Default::default()
    };
    shared.project_scene_contents(&mut projection, None);
    let contents: serde_json::Value =
        serde_json::from_str(&projection.scene_contents_json).unwrap();
    assert_eq!(
        contents["narrative_focus"]["focus"]["subject"]["id"],
        "clothes-fine"
    );
    assert_eq!(
        contents["narrative_focus"]["focus"]["dialogue_sequence"],
        42
    );
    assert_eq!(contents["narrative_focus"]["stage"], "published");
    assert_eq!(contents["narrative_focus"]["published_revision"], 2);
    assert_eq!(
        current.setting.as_ref().unwrap().model.as_ref().unwrap(),
        &before
    );
    shared.observe_with(&focused, Some(current.clone()), Vec::new());
    shared.poll_state_images_at(Duration::from_secs(40));
    assert!(
        drain(&mut queue).is_empty(),
        "one committed reveal, no repeated render"
    );
}

#[test]
fn narrative_focus_waits_for_pending_references_and_reports_terminal_reference_failure() {
    let (shared, mut queue) = shared(16);
    let mut current = scene(
        SceneKey::Opening,
        "Focus reference lifecycle",
        Some("Marek"),
    );
    current.cast[0].id = "patron".into();
    current.cast[0].name = "Marek".into();
    current.setting.as_mut().unwrap().model = Some(crate::gameplay::narrative_focus::tests::shop());
    shared
        .inner
        .observe_checkpoint(&crate::gameplay::journey::initial().unwrap());
    locked(&shared.inner.state).canonical.observe_room(&current);
    shared.observe_with(
        &view(&current, &[], false),
        Some(current.clone()),
        Vec::new(),
    );
    let mut focused = view(&current, &[], false);
    focused.graph_revision = 2;
    focused.story = committed_story(&rpc::JourneyView {
        table_log: vec![public_npc_line(
            42,
            &current.cast[0],
            "The fine clothes are fifteen hundred copper.",
        )],
        ..Default::default()
    });
    shared.observe_with(&focused, Some(current.clone()), Vec::new());
    shared.poll_state_images_at(Duration::from_secs(4));
    assert!(locked(&shared.book).state_images.pending.is_some());
    assert!(drain(&mut queue).is_empty());
    loop {
        let jobs = locked(&shared.inner.state).canonical.take_jobs();
        if jobs.is_empty() {
            break;
        }
        for job in jobs {
            let super::super::Job::Canonical(work) = job else {
                panic!("canonical reference job")
            };
            shared.inner.finish(super::super::Done::Canonical {
                work,
                result: Err(ImageFailure::Provider),
            });
        }
    }
    shared.poll_state_images_at(Duration::from_secs(8));
    assert!(locked(&shared.book).state_images.pending.is_none());
    assert!(drain(&mut queue).is_empty());
    let mut projection = rpc::GameplayScene {
        title: current.title,
        scene_asset: current.asset,
        ..Default::default()
    };
    shared.project_scene_contents(&mut projection, None);
    let contents: serde_json::Value =
        serde_json::from_str(&projection.scene_contents_json).unwrap();
    assert_eq!(contents["narrative_focus"]["stage"], "failed");
    assert_eq!(
        contents["narrative_focus"]["focus"]["subject"]["id"],
        "clothes-fine"
    );
}

#[tokio::test]
async fn authored_story_coalesces_latest_reply_edits_shown_base_and_respects_configured_interval() {
    let current = scene(SceneKey::DialogueSeal, "Harbor story", Some("Kestrel"));
    let (shared, mut queue) = shared(16);
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    let story = |sequence, text: &str| StoryBeat {
        actor_id: String::new(),
        sequence,
        speaker: "Kestrel".into(),
        text: text.into(),
        sha256: format!("hash-{text}"),
        tempo: crate::gameplay::shot::PublicBeat::Build,
        dialogue: crate::gameplay::shot::PublicDialogue::FirstReply,
    };
    let mut next = view(&current, &[1], false);
    next.story = Some(story(2, "Keep your voice low."));
    next.graph_revision = 2;
    shared.observe_with(&next, Some(current.clone()), Vec::new());
    shared.poll_state_images_at(Duration::from_secs(2));
    assert!(drain(&mut queue).is_empty());
    shared.poll_state_images_at(Duration::from_secs(4));
    let Job::State(state_images::Job::Bake { pending, base, .. }) = drain(&mut queue).remove(0)
    else {
        panic!("story edit")
    };
    assert_eq!(base, key('a'));
    assert_eq!(pending.story.as_ref().unwrap().sequence, 2);
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: next.scene_key.clone(),
            state: pending.state,
            revision: pending.revision,
            story_actor: pending.story.as_deref().map(StoryBeat::actor_key),
            result: Ok(key('b')),
        },
        Duration::from_secs(5),
    );
    // The same text with a new log sequence does not cost another image.
    next.story.as_mut().unwrap().sequence = 3;
    next.graph_revision = 3;
    shared.observe_with(&next, Some(current.clone()), Vec::new());
    shared.poll_state_images_at(Duration::from_secs(40));
    assert!(drain(&mut queue).is_empty());
    for (sequence, text) in [(4, "I glance at the enforcer."), (5, "I lower my voice.")] {
        next.story = Some(story(sequence, text));
        next.graph_revision = sequence;
        shared.observe_with(&next, Some(current.clone()), Vec::new());
    }
    shared.poll_state_images_at(Duration::from_secs(33));
    assert!(drain(&mut queue).is_empty());
    shared.poll_state_images_at(Duration::from_secs(34));
    let Job::State(state_images::Job::Bake { pending, base, .. }) = drain(&mut queue).remove(0)
    else {
        panic!("latest story edit")
    };
    assert_eq!(
        base,
        key('b'),
        "edit the currently shown known-good picture"
    );
    assert_eq!(pending.story.as_ref().unwrap().sequence, 5);
    assert_eq!(
        pending.story.as_ref().unwrap().dialogue,
        crate::gameplay::shot::PublicDialogue::SameSpeakerFollowup
    );
    assert!(
        drain(&mut queue).is_empty(),
        "one bounded pending story, not one job per reply"
    );
}

fn public_story_message(scene: &pack::Scene, table_log: Vec<rpc::TableLine>) -> rpc::ViewMessage {
    let mut message = display_message(scene);
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        panic!("display fixture")
    };
    display.journey = Some(rpc::JourneyView {
        table_log,
        tempo: Some(rpc::TempoCue {
            phase: rpc::TempoPhase::Build as i32,
            ..Default::default()
        }),
        ..Default::default()
    });
    message
}

fn public_npc_line(sequence: u64, actor: &pack::SceneCastMember, text: &str) -> rpc::TableLine {
    rpc::TableLine {
        sequence,
        actor_id: actor.id.clone(),
        speaker: actor.name.clone(),
        role: rpc::TableRole::Npc as i32,
        kind: rpc::TableKind::Speech as i32,
        text: text.to_owned(),
        ..Default::default()
    }
}

#[tokio::test]
async fn public_story_caller_changes_camera_at_constant_tempo_and_measures_each_edited_key() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/filmic-camera-repair/caller-{}",
        std::process::id(),
    ));
    let fixture = DirectionFixture::new(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Arc::new(Shared {
        inner: Arc::new(Inner {
            imagery: fixture.imagery.clone(),
            references: super::super::ReferenceStore::new(
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
            ),
            social: SocialLedger::none(),
            rating: ContentRating::Restrained,
            timing,
            state: Mutex::new(State::new(timing)),
            wake: tokio::sync::watch::channel(0).0,
            origin: Instant::now(),
        }),
        book: Mutex::new(Book::new()),
        jobs,
    });
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let loaded =
        pack::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/harbor-journey"))
            .unwrap();
    let current = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::DialogueSeal)
        .unwrap();
    let first_actor = &current.cast[0];
    let other_actor = &current.cast[1];
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(current, Vec::new()), 1);
    });
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    let scene = PlateScene::of(current).unwrap();
    let (request, _) = shared
        .fresh_request(&scene, &scene.stage(&[]).unwrap(), &[], &[], None)
        .await
        .unwrap();
    let base = fixture.imagery.render(request).await.unwrap();
    locked(&shared.book).insert(
        id,
        Entry::Ready {
            key: base.clone(),
            placement: None,
        },
    );
    let mut lines = Vec::new();
    for (sequence, actor, text, camera, phrase) in [
        (
            2,
            first_actor,
            "Keep your voice low.",
            shot::PublicDialogue::FirstReply,
            "medium dialogue",
        ),
        (
            3,
            first_actor,
            "I am still listening.",
            shot::PublicDialogue::SameSpeakerFollowup,
            "close conversational",
        ),
        (
            4,
            other_actor,
            "I heard that.",
            shot::PublicDialogue::SpeakerChange,
            "over-the-shoulder dialogue",
        ),
    ] {
        lines.push(public_npc_line(sequence, actor, text));
        pack::with_test_pack(loaded.clone(), || {
            plates.observe(&public_story_message(current, lines.clone()), sequence);
        });
        let now = shared.inner.now() + Duration::from_secs(4 + (sequence - 2) * 31);
        shared.poll_state_images_at(now);
        let Job::State(job) = drain(&mut queue).remove(0) else {
            panic!("automatic story edit")
        };
        let state_images::Job::Bake {
            pending,
            base,
            identities,
            ..
        } = &job
        else {
            panic!("story bake")
        };
        assert_eq!(pending.story.as_ref().unwrap().dialogue, camera);
        let Lookup::Image { bytes, format } = fixture.imagery.image(base.as_str()).await else {
            panic!("shown base bytes")
        };
        let request = shared
            .state_image_request(
                pending,
                ReferenceImage::new(format, bytes),
                identities.as_deref(),
            )
            .await
            .unwrap();
        assert!(request.prompt.as_str().contains(phrase));
        assert!(
            !request
                .prompt
                .as_str()
                .contains("Keep the exact aspect ratio and framing")
        );
        assert!(!request.prompt.as_str().contains("Keep its camera position"));
        let expected = fixture.imagery.key_for(&request);
        assert_ne!(&expected, base);
        let done = shared.render_state_image(job).await;
        assert!(
            matches!(&done, state_images::Done::Bake { result: Ok(key), .. } if key == &expected)
        );
        assert!(
            locked(&shared.book)
                .image_layouts
                .contains_key(expected.as_str()),
            "the actual edit renderer measured this exact new image, using the fixture fallback"
        );
        shared.finish_state_image_at(done, now + Duration::from_secs(1));
        assert_eq!(locked(&shared.book).shown().unwrap().0, expected);
        assert!(fixture.prompt().contains(phrase));
    }
}

#[tokio::test]
async fn public_story_caller_counts_only_published_edits_and_resets_when_the_scene_changes() {
    let loaded =
        pack::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/harbor-journey"))
            .unwrap();
    let current = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::DialogueSeal)
        .unwrap();
    let next = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::DialogueEscort)
        .unwrap();
    let actor = &current.cast[0];
    let other = &current.cast[1];
    let (shared, mut queue) = shared(16);
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let mut lines = Vec::new();
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(current, Vec::new()), 1)
    });
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    // Both replies settle into one first shot; observing a reply does not count as showing it.
    for (sequence, speaker, text) in [(2, other, "Wait."), (3, actor, "Let me explain.")] {
        lines.push(public_npc_line(sequence, speaker, text));
        pack::with_test_pack(loaded.clone(), || {
            plates.observe(&public_story_message(current, lines.clone()), sequence)
        });
    }
    shared.poll_state_images_at(Duration::from_secs(4));
    let Job::State(state_images::Job::Bake { pending, .. }) = drain(&mut queue).remove(0) else {
        panic!("first edit")
    };
    assert_eq!(
        pending.story.as_ref().unwrap().dialogue,
        shot::PublicDialogue::FirstReply
    );
    let completed_actor = pending.story.as_deref().map(StoryBeat::actor_key);
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: view(current, &[], false).scene_key,
            state: pending.state,
            revision: pending.revision,
            story_actor: completed_actor,
            result: Ok(key('b')),
        },
        Duration::from_secs(5),
    );
    // Identical text with another public sequence is suppressed.
    lines.push(public_npc_line(4, actor, "Let me explain."));
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(current, lines.clone()), 4)
    });
    shared.poll_state_images_at(Duration::from_secs(34));
    assert!(drain(&mut queue).is_empty());
    // A failed change of speaker does not become the previous shown actor.
    lines.push(public_npc_line(5, other, "No."));
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(current, lines.clone()), 5)
    });
    shared.poll_state_images_at(Duration::from_secs(35));
    let Job::State(state_images::Job::Bake { pending, .. }) = drain(&mut queue).remove(0) else {
        panic!("changed speaker")
    };
    assert_eq!(
        pending.story.as_ref().unwrap().dialogue,
        shot::PublicDialogue::SpeakerChange
    );
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: view(current, &[], false).scene_key,
            state: pending.state,
            revision: pending.revision,
            story_actor: pending.story.as_deref().map(StoryBeat::actor_key),
            result: Err(ImageFailure::InvalidRequest),
        },
        Duration::from_secs(36),
    );
    for (sequence, speaker, text) in [(6, other, "Listen."), (7, actor, "As I said.")] {
        lines.push(public_npc_line(sequence, speaker, text));
        pack::with_test_pack(loaded.clone(), || {
            plates.observe(&public_story_message(current, lines.clone()), sequence)
        });
    }
    shared.poll_state_images_at(Duration::from_secs(66));
    let Job::State(state_images::Job::Bake { pending, .. }) = drain(&mut queue).remove(0) else {
        panic!("followup")
    };
    assert_eq!(
        pending.story.as_ref().unwrap().dialogue,
        shot::PublicDialogue::SameSpeakerFollowup
    );
    // An uncompleted old-scene job and retained log cannot set the next scene's continuity.
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(next, lines.clone()), 8)
    });
    assert!(locked(&shared.book).state_images.pending.is_none());
    drain(&mut queue);
    let id = locked(&shared.book)
        .current
        .as_ref()
        .unwrap()
        .id
        .clone()
        .unwrap();
    ready(&shared, &id, 'c');
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: view(current, &[], false).scene_key,
            state: pending.state,
            revision: pending.revision,
            story_actor: pending.story.as_deref().map(StoryBeat::actor_key),
            result: Ok(key('d')),
        },
        Duration::from_secs(67),
    );
    lines.push(public_npc_line(9, actor, "Here we are."));
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(next, lines), 9)
    });
    shared.poll_state_images_at(Duration::from_secs(70));
    let Job::State(state_images::Job::Bake { pending, .. }) = drain(&mut queue).remove(0) else {
        panic!("new scene first reply")
    };
    assert_eq!(
        pending.story.as_ref().unwrap().dialogue,
        shot::PublicDialogue::FirstReply
    );
}

#[tokio::test]
async fn public_story_caller_ignores_private_withheld_and_action_only_updates() {
    let loaded =
        pack::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/harbor-journey"))
            .unwrap();
    let current = loaded
        .pack
        .scenes
        .iter()
        .find(|scene| scene.key == SceneKey::DialogueSeal)
        .unwrap();
    let (shared, mut queue) = shared(16);
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    pack::with_test_pack(loaded.clone(), || {
        plates.observe(&public_story_message(current, Vec::new()), 1)
    });
    drain(&mut queue);
    let mut private = public_npc_line(2, &current.cast[0], "murmurs to Hero 1");
    private.kind = rpc::TableKind::Action as i32;
    let action = rpc::TableLine {
        sequence: 3,
        role: rpc::TableRole::Hero as i32,
        kind: rpc::TableKind::Action as i32,
        text: "Look around.".into(),
        ..Default::default()
    };
    for (revision, lines) in [(2, Vec::new()), (3, vec![private]), (4, vec![action])] {
        let mut message = public_story_message(current, lines);
        let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
            panic!("display")
        };
        display.journey.as_mut().unwrap().dialogue =
            "Uncommitted or withheld prose is not a camera cue.".into();
        pack::with_test_pack(loaded.clone(), || plates.observe(&message, revision));
        shared.poll_state_images_at(Duration::from_secs(40));
        assert!(drain(&mut queue).is_empty());
        assert!(locked(&shared.book).state_images.pending.is_none());
    }
    plates.observe(&player_message(current), 5);
    assert!(locked(&shared.book).state_images.pending.is_none());
}

#[test]
fn modelled_face_refresh_settles_and_has_a_shared_cadence() {
    let angry = Some(Face {
        expression: Expression::Angry,
        tier: HealthTier::Healthy,
    });
    let mut gate = FaceRefresh::default();
    assert!(!gate.ready(angry, Duration::ZERO));
    assert!(gate.ready(angry, Duration::from_secs(3)));
    gate.last_started = Some(Duration::from_secs(3));
    assert!(!gate.ready(None, Duration::from_secs(4)));
    assert!(!gate.ready(None, Duration::from_secs(7)));
    assert!(gate.ready(None, Duration::from_secs(13)));
}

#[test]
fn retained_narrator_findings_do_not_hold_the_plate_roll_guard() {
    let current = scene(SceneKey::Opening, "Harbor", Some("Kestrel"));
    let mut message = display_message(&current);
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        panic!("display")
    };
    display.journey = Some(rpc::JourneyView {
        table_log: vec![rpc::TableLine {
            role: rpc::TableRole::Narrator as i32,
            kind: rpc::TableKind::Action as i32,
            speaker: "Roll".into(),
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(view_of(&message, 1).unwrap().in_roll);
    let Some(rpc::view_message::Audience::Display(display)) = message.audience.as_mut() else {
        panic!("display")
    };
    display
        .journey
        .as_mut()
        .unwrap()
        .table_log
        .last_mut()
        .unwrap()
        .speaker = "Dungeon Master".into();
    assert!(!view_of(&message, 1).unwrap().in_roll);
}

#[test]
fn shipped_harbor_story_scenes_are_plannable_except_lobby_and_combat() {
    let document: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/pack.json"
    ))
    .expect("harbor pack must be JSON");

    for raw in document["scenes"]
        .as_array()
        .expect("scenes must be an array")
    {
        let scene: pack::Scene = serde_json::from_value(raw.clone()).expect("scene must decode");
        let plannable = PlateScene::of(&scene).is_some();
        assert_eq!(
            plannable,
            !matches!(scene.key, SceneKey::Room | SceneKey::Combat),
            "unexpected plate eligibility for {:?}",
            scene.key
        );
    }
}

fn shared(capacity: usize) -> (Arc<Shared>, mpsc::Receiver<Job>) {
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: Imagery::disabled(ImageryOff::Qualification),
        references: super::super::ReferenceStore::new(PathBuf::new()),
        social: SocialLedger::none(),
        rating: ContentRating::Mature,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, queue) = mpsc::channel(capacity);
    (
        Arc::new(Shared {
            inner,
            book: Mutex::new(Book::new()),
            jobs,
        }),
        queue,
    )
}

#[tokio::test]
async fn directed_plate_uses_the_same_references_updates_views_and_keeps_the_base_cached() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/campaign/luna6/J17/unit-plate-cache-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::new(&cache);
    let mut timing = Timing::from_interval(Duration::from_secs(30));
    timing.roll_quiet = Duration::ZERO;
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Arc::new(Shared {
        inner,
        book: Mutex::new(Book::new()),
        jobs,
    });
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    let scene = PlateScene::of(&current).unwrap();
    let spec = scene.stage(&[1]).unwrap();
    let (request, _) = shared
        .fresh_request(&scene, &spec, &[1], &[], None)
        .await
        .unwrap();
    let baseline_prompt = request.prompt.as_str().to_owned();
    let reference_hashes: Vec<_> = request
        .references
        .iter()
        .map(|reference| reference.sha256.clone())
        .collect();
    let base = fixture.imagery.render(request).await.unwrap();
    locked(&shared.book).insert(
        id,
        Entry::Ready {
            key: base.clone(),
            placement: None,
        },
    );
    let receive = plates
        .direct("Brakka unsheathes his sword".to_owned(), true)
        .unwrap();
    let Job::Directed(work) = queue.recv().await.unwrap() else {
        panic!("directed work")
    };
    let (directed, _) = shared
        .fresh_request(
            &work.scene,
            &work.spec,
            &work.heroes,
            &work.faces,
            Some(&work.text),
        )
        .await
        .unwrap();
    assert_eq!(
        reference_hashes,
        directed
            .references
            .iter()
            .map(|reference| reference.sha256.clone())
            .collect::<Vec<_>>()
    );
    let done = shared.render_directed(*work).await;
    let response = receive.await.unwrap().unwrap();
    assert_eq!(response["state"], "pending");
    shared.finish(done);
    assert_eq!(
        plates
            .direct_status(response["key"].as_str().unwrap())
            .unwrap()["state"],
        "ready"
    );
    assert_ne!(response["key"], base.as_str());
    assert!(fixture.prompt().starts_with(&baseline_prompt));
    assert!(fixture.prompt().ends_with("Brakka unsheathes his sword"));
    for mut message in [display_message(&current), player_message(&current)] {
        apply(&shared, &mut message);
        assert_eq!(
            shown_scene(&message).live_plate,
            format!("/gameplay/imagery/{}", response["key"].as_str().unwrap())
        );
    }
    // A living-scene face refresh uses the directed base and keeps the same action direction.
    let angry = Face {
        expression: Expression::Angry,
        tier: HealthTier::Healthy,
    };
    locked(&shared.inner.state)
        .plate_wants
        .insert("keeper".to_owned(), angry);
    shared.poll_face();
    let Job::Edit {
        base: edit_base, ..
    } = queue.recv().await.unwrap()
    else {
        panic!("face refresh")
    };
    assert_eq!(edit_base.as_str(), response["key"].as_str().unwrap());
    shared
        .render_edit(&scene, &edit_base, angry, &[1])
        .await
        .unwrap();
    assert!(fixture.prompt().ends_with("Brakka unsheathes his sword"));
    plates.clear_direction();
    assert!(locked(&shared.book).direct.keep.is_none());
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(shown_scene(&message).live_plate, base.path());
    let calls = fixture.calls();
    let (request, _) = shared
        .fresh_request(&scene, &spec, &[1], &[], None)
        .await
        .unwrap();
    assert_eq!(fixture.imagery.render(request).await.unwrap(), base);
    assert_eq!(fixture.calls(), calls, "undirected cache is retained");
}

fn observe(shared: &Shared, current: &pack::Scene, next: &[pack::Scene], heroes: &[u8]) {
    shared.observe_with(
        &view(current, heroes, false),
        Some(current.clone()),
        next.to_vec(),
    );
}

#[test]
fn clip_context_exists_only_for_the_displayed_plate_and_is_blocked_during_rolls() {
    let (shared, mut queue) = shared(8);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    let plate_key = key('a');
    locked(&shared.book).entries.insert(
        id,
        Entry::Ready {
            key: plate_key.clone(),
            placement: None,
        },
    );
    let scene_key = format!(
        "{}|{}|{}",
        current.key.source_label(),
        current.title,
        current.asset
    );

    let (description, reason, blocked) = shared
        .current_clip_context(&scene_key, plate_key.as_str())
        .expect("current plate is eligible for a scene clip");
    assert_eq!(reason, "scene-plate");
    assert!(!blocked);
    assert!(!description.contains(&current.dialogue));
    let successor_key = key('b');
    assert!(
        shared
            .current_clip_context(&scene_key, successor_key.as_str())
            .is_none(),
        "an unshown successor is not eligible"
    );

    locked(&shared.book).current.as_mut().unwrap().in_roll = true;
    let (_, _, blocked) = shared
        .current_clip_context(&scene_key, plate_key.as_str())
        .expect("the displayed plate remains known during a roll");
    assert!(blocked);
}

fn drain(queue: &mut mpsc::Receiver<Job>) -> Vec<Job> {
    let mut jobs = Vec::new();
    while let Ok(job) = queue.try_recv() {
        jobs.push(job);
    }
    jobs
}

fn fresh_ids(jobs: &[Job]) -> Vec<String> {
    jobs.iter()
        .filter_map(|job| match job {
            Job::Fresh { id, .. } => Some(id.clone()),
            Job::State(_) | Job::Directed(_) | Job::Edit { .. } | Job::Locate { .. } => None,
        })
        .collect()
}

fn display_message(scene: &pack::Scene) -> rpc::ViewMessage {
    rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                narration: String::new(),
                scene_asset: scene.asset.clone(),
                scene: Some(rpc::GameplayScene {
                    title: scene.title.clone(),
                    scene_asset: scene.asset.clone(),
                    source_scene_key: scene.key.source_label().to_owned(),
                    source_scene_variant: scene.source_variant_id.clone().unwrap_or_default(),
                    ..Default::default()
                }),
                journey: None,
                ..Default::default()
            },
        )),
    }
}

fn player_message(scene: &pack::Scene) -> rpc::ViewMessage {
    rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Player(
            rpc::PlayerGameplayView {
                scene: Some(rpc::GameplayScene {
                    title: scene.title.clone(),
                    scene_asset: scene.asset.clone(),
                    source_scene_key: scene.key.source_label().to_owned(),
                    source_scene_variant: scene.source_variant_id.clone().unwrap_or_default(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
}

fn shown_scene(message: &rpc::ViewMessage) -> &rpc::GameplayScene {
    match message.audience.as_ref().unwrap() {
        rpc::view_message::Audience::Display(display) => display.scene.as_ref().unwrap(),
        rpc::view_message::Audience::Player(player) => player.scene.as_ref().unwrap(),
    }
}

#[test]
fn visible_changes_settle_into_one_pending_edit_and_searches_make_none() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(37);
    let mut scene = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    scene.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let mut plated = PlateScene::of(&scene).unwrap();
    let view = view(&scene, &[1], false);
    let mut book = Book::new();
    state_images::observe(&mut book, &view, &plated, Duration::ZERO);
    plated.snapshot.as_mut().unwrap().model.things[0]
        .searched_by
        .push("Brakka".into());
    state_images::observe(&mut book, &view, &plated, Duration::from_secs(1));
    assert!(book.state_images.pending.is_none());
    plated
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Brakka pockets a coin".into());
    state_images::observe(&mut book, &view, &plated, Duration::from_secs(1));
    assert!(
        book.state_images.pending.is_none(),
        "an invisible inventory change is not a bake"
    );
    plated.snapshot.as_mut().unwrap().model.things[0].open = true;
    plated
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Brakka opens the door".into());
    state_images::observe(&mut book, &view, &plated, Duration::from_secs(2));
    plated.snapshot.as_mut().unwrap().model.things[0].lit = true;
    plated
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Sigrun lights the candle".into());
    state_images::observe(&mut book, &view, &plated, Duration::from_secs(3));
    assert!(book.state_images.pending.is_some());
}

#[test]
fn speech_graph_revisions_do_not_edit_an_unchanged_modelled_room() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(37);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let plated = PlateScene::of(&current).unwrap();
    let mut talk = view(&current, &[1], false);
    let mut book = Book::new();
    state_images::observe(&mut book, &talk, &plated, Duration::ZERO);
    for revision in 2..=10 {
        talk.graph_revision = revision;
        state_images::observe(&mut book, &talk, &plated, Duration::from_secs(revision));
    }
    assert!(book.state_images.pending.is_none());
}

#[tokio::test]
async fn settled_edits_are_shown_at_least_ten_seconds_apart_even_when_decode_is_fast() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(41);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (shared, mut queue) = shared(16);
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    let mut plated = PlateScene::of(&current).unwrap();
    let mut changed = view(&current, &[1], false);
    changed.graph_revision = 2;
    plated.snapshot.as_mut().unwrap().model.things[0].open = true;
    state_images::observe(
        &mut locked(&shared.book),
        &changed,
        &plated,
        Duration::from_secs(15),
    );
    // A ready edit is held during a roll, then the roll ends.
    locked(&shared.book).current.as_mut().unwrap().in_roll = true;
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: changed.scene_key.clone(),
            state: plated
                .snapshot
                .as_ref()
                .unwrap()
                .model
                .picture_state()
                .unwrap(),
            revision: 2,
            story_actor: None,
            result: Ok(key('b')),
        },
        Duration::from_secs(19),
    );
    locked(&shared.book).current.as_mut().unwrap().in_roll = false;
    shared.poll_state_images_at(Duration::from_secs(20));
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('b'));
    changed.graph_revision = 3;
    plated.snapshot.as_mut().unwrap().model.things[0].lit = true;
    state_images::observe(
        &mut locked(&shared.book),
        &changed,
        &plated,
        Duration::from_secs(20),
    );
    shared.finish_state_image_at(
        state_images::Done::Bake {
            scene: changed.scene_key,
            state: plated
                .snapshot
                .as_ref()
                .unwrap()
                .model
                .picture_state()
                .unwrap(),
            revision: 3,
            story_actor: None,
            result: Ok(key('c')),
        },
        Duration::from_secs(21),
    );
    shared.poll_state_images_at(Duration::from_secs(21));
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('b'));
    shared.poll_state_images_at(Duration::from_secs(30));
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('c'));
}

#[tokio::test]
async fn settled_changes_edit_once_keep_the_old_plate_and_drop_an_outdated_completion() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(41);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (shared, mut queue) = shared(16);
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    let base = locked(&shared.book).shown().unwrap().0;
    let mut plated = PlateScene::of(&current).unwrap();
    let mut changed_view = view(&current, &[1], false);
    changed_view.graph_revision = 2;
    plated.snapshot.as_mut().unwrap().model.things[0].open = true;
    plated
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Brakka opens the door".into());
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &plated,
        Duration::from_secs(1),
    );
    plated.snapshot.as_mut().unwrap().model.things[0].lit = true;
    plated
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Sigrun lights the candle".into());
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &plated,
        Duration::from_secs(2),
    );
    shared.poll_state_images_at(Duration::from_secs(4));
    assert!(drain(&mut queue).is_empty());
    shared.poll_state_images_at(Duration::from_secs(5));
    let jobs = drain(&mut queue);
    assert_eq!(jobs.len(), 1);
    assert!(matches!(jobs[0], Job::State(_)));
    assert_eq!(locked(&shared.book).shown().unwrap().0, base);
    shared.poll_state_images_at(Duration::from_secs(6));
    assert!(drain(&mut queue).is_empty());
    // Another visible change supersedes the flight before its image arrives.
    plated.snapshot.as_mut().unwrap().model.things[0].broken = true;
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &plated,
        Duration::from_secs(6),
    );
    let obsolete = "earlier state".into();
    shared.finish_state_image(state_images::Done::Bake {
        scene: changed_view.scene_key,
        state: obsolete,
        revision: 2,
        story_actor: None,
        result: Ok(key('b')),
    });
    assert_eq!(locked(&shared.book).shown().unwrap().0, base);
}

#[tokio::test]
async fn item_cards_are_cached_per_object_state_and_private_until_shared() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(38);
    let mut scene = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    scene.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (shared, mut queue) = shared(16);
    observe(&shared, &scene, &[], &[1]);
    drain(&mut queue);
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let thing = locked(&shared.book)
        .current
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .model
        .things[0]
        .clone();
    let id = thing.id.clone();
    plates.request_item("Brakka", &id, None, false).unwrap();
    plates.request_item("Brakka", &id, None, false).unwrap();
    assert_eq!(drain(&mut queue).len(), 1);
    let mut private = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        item: Some(rpc::SceneItemCard {
            thing_id: id.clone(),
            name: thing.name.clone(),
            detail: thing.detail.clone(),
            state_revision: 8,
            ..Default::default()
        }),
        ..Default::default()
    };
    shared.project_scene_contents(&mut private, Some("Brakka"));
    assert_eq!(private.item.as_ref().unwrap().thing_id, id);
    assert_eq!(private.item.as_ref().unwrap().state_revision, 8);
    private.scene_contents_json = "{\"authoritative_revision\":9}".into();
    private.item = Some(rpc::SceneItemCard {
        thing_id: "item-clothes-travelers".into(),
        name: "traveler’s clothes".into(),
        detail: "The durable purchase belongs to Brakka.".into(),
        state_revision: 9,
        ..Default::default()
    });
    let authoritative = private.item.clone();
    shared.project_scene_contents(&mut private, Some("Brakka"));
    assert_eq!(
        private.item, authoritative,
        "stale image request must not replace a newer durable purchase"
    );
    assert_eq!(
        private.scene_contents_json,
        "{\"authoritative_revision\":9}"
    );
    private.item = None;
    shared.project_scene_contents(&mut private, Some("Brakka"));
    assert!(
        private.item.is_none(),
        "media requests cannot fabricate durable item selection"
    );
    let mut public = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        ..Default::default()
    };
    shared.project_scene_contents(&mut public, None);
    assert!(public.item.is_none());
    plates.request_item("Brakka", &id, None, true).unwrap();
    assert!(drain(&mut queue).is_empty());
    shared.project_scene_contents(&mut public, None);
    assert_eq!(public.item.as_ref().unwrap().thing_id, id);
}

#[tokio::test]
async fn committed_stock_items_and_reconnected_cards_rebuild_the_canonical_image_cache() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(381);
    let mut scene = scene(SceneKey::Opening, "Marek’s cloth shop", Some("Marek"));
    scene.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (plate_shared, mut queue) = shared(16);
    observe(&plate_shared, &scene, &[], &[1]);
    drain(&mut queue);
    let plates = Plates {
        shared: plate_shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let mut purchased = scene
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .things[0]
        .clone();
    purchased.id = "item-clothes-travelers".into();
    purchased.name = "traveler’s clothes".into();
    purchased.detail = "A practical set of traveler’s clothes. It was acquired from Marek.".into();
    purchased.picture_description = "neatly folded wool and linen traveler’s clothes".into();
    purchased.owner_id = Some("Brakka".into());
    purchased.holder_id = Some("Brakka".into());
    assert!(
        scene
            .setting
            .as_ref()
            .unwrap()
            .model
            .as_ref()
            .unwrap()
            .things
            .iter()
            .all(|thing| thing.id != purchased.id)
    );

    plates
        .request_committed_item("Brakka", &purchased, None, false)
        .expect("the committed Thing is sufficient even though the authored plate is stale");
    assert_eq!(drain(&mut queue).len(), 1);
    let committed_cache_id = locked(&plate_shared.book)
        .state_images
        .items
        .keys()
        .find(|id| id.starts_with("item-clothes-travelers:"))
        .cloned()
        .expect("the canonical purchased item has an image request");

    // A fresh cache after reconnect has only the phone card and current public scene snapshot.
    let (reconnected, mut reconnect_queue) = shared(16);
    observe(&reconnected, &scene, &[], &[1]);
    drain(&mut reconnect_queue);
    let reconnect_plates = Plates {
        shared: reconnected.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let card = rpc::SceneItemCard {
        thing_id: purchased.id.clone(),
        name: purchased.name.clone(),
        detail: purchased.detail.clone(),
        picture_description: purchased.picture_description.clone(),
        holder: "Brakka".into(),
        owner_id: "Brakka".into(),
        holder_id: "Brakka".into(),
        ..Default::default()
    };
    let scene_contents = serde_json::json!({
        "things": [{
            "id": purchased.id.clone(),
            "name": purchased.name.clone(),
            "look": purchased.picture_description.clone(),
            "position": "held by Brakka",
            "in_scene": true,
            "publicly_visible": true,
            "open": false,
            "lit": false,
            "broken": false
        }]
    })
    .to_string();
    reconnect_plates
        .request_hand_item(&card, false, &scene_contents)
        .expect("the current projection can reconstruct the image request after reconnect");
    assert_eq!(drain(&mut reconnect_queue).len(), 1);
    let first_reconnect_cache_id = locked(&reconnected.book)
        .state_images
        .items
        .keys()
        .next()
        .cloned()
        .unwrap();
    assert!(first_reconnect_cache_id.starts_with("item-clothes-travelers:"));

    let changed_card = rpc::SceneItemCard {
        detail: "The same clothes, now patched at one sleeve.".into(),
        picture_description: "folded wool and linen clothes with a patched sleeve".into(),
        ..card
    };
    let changed_contents = serde_json::json!({
        "things": [{
            "id": changed_card.thing_id.clone(),
            "name": changed_card.name.clone(),
            "look": changed_card.picture_description.clone(),
            "position": "held by Brakka",
            "in_scene": true,
            "publicly_visible": true,
            "open": false,
            "lit": false,
            "broken": false
        }]
    })
    .to_string();
    reconnect_plates
        .request_hand_item(&changed_card, false, &changed_contents)
        .expect("a canonical detail change starts a fresh image revision");
    assert_eq!(drain(&mut reconnect_queue).len(), 1);
    let reconnect_cache_ids = locked(&reconnected.book)
        .state_images
        .items
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(reconnect_cache_ids.len(), 2);
    assert_ne!(first_reconnect_cache_id, reconnect_cache_ids[1]);
    assert!(committed_cache_id.starts_with("item-clothes-travelers:"));
}

#[tokio::test]
async fn hand_item_image_tracks_custody_and_stays_out_of_private_display_projection() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(39);
    let mut scene = scene(SceneKey::Opening, "The Cloth Shop", Some("Marek"));
    let mut model = generated.settings.meeting.model.clone().unwrap();
    let thing = model.things.iter_mut().find(|thing| !thing.in_use).unwrap();
    let thing_id = thing.id.clone();
    let thing_name = thing.name.clone();
    let thing_detail = thing.detail.clone();
    let picture_description = thing.picture_description.clone();
    thing.owner_id = Some("merchant".to_owned());
    thing.holder_id = Some("Marek".to_owned());
    thing.publicly_visible = true;
    scene.setting.as_mut().unwrap().model = Some(model);

    let (shared, mut queue) = shared(16);
    observe(&shared, &scene, &[], &[1]);
    drain(&mut queue);
    let plates = Plates {
        shared: shared.clone(),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let hand_card = rpc::SceneItemCard {
        thing_id: thing_id.clone(),
        name: thing_name.clone(),
        detail: thing_detail.clone(),
        picture_description: picture_description.clone(),
        holder: "Marek".into(),
        owner_id: "merchant".into(),
        holder_id: "Marek".into(),
        observation_only: true,
        ..Default::default()
    };
    let contents = scene
        .setting
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .public_contents()
        .unwrap();
    plates
        .request_hand_item(&hand_card, true, &contents)
        .unwrap();
    assert_eq!(drain(&mut queue).len(), 1);
    let cache_id = {
        let book = locked(&shared.book);
        book.state_images
            .items
            .iter()
            .find(|(_, cached)| cached.thing.id == thing_id)
            .map(|(cache_id, _)| cache_id.clone())
            .expect("the requested hand item has a cache entry")
    };
    locked(&shared.book)
        .state_images
        .items
        .get_mut(&cache_id)
        .unwrap()
        .key = Some(key('a'));

    {
        let mut book = locked(&shared.book);
        let current = book.current.as_mut().unwrap();
        let thing = current
            .scene
            .as_mut()
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .model
            .things
            .iter_mut()
            .find(|thing| thing.id == thing_id)
            .unwrap();
        thing.holder_id = Some("Sigrun".to_owned());
    }
    let refreshed_card = rpc::SceneItemCard {
        thing_id: thing_id.clone(),
        name: thing_name.clone(),
        detail: thing_detail.clone(),
        picture_description: picture_description.clone(),
        holder: "Sigrun".into(),
        owner_id: "merchant".into(),
        holder_id: "Sigrun".into(),
        observation_only: true,
        ..Default::default()
    };
    let refreshed_contents = locked(&shared.book)
        .current
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .model
        .public_contents()
        .unwrap();
    plates
        .request_hand_item(&refreshed_card, true, &refreshed_contents)
        .unwrap();
    assert!(
        drain(&mut queue).is_empty(),
        "custody reuses the object image"
    );
    {
        let book = locked(&shared.book);
        let (current_cache_id, cached) = book
            .state_images
            .items
            .iter()
            .find(|(_, cached)| cached.thing.id == thing_id)
            .expect("the current hand item keeps a cache entry");
        assert_eq!(current_cache_id, &cache_id, "custody reuses cache identity");
        assert_eq!(cached.key.as_ref(), Some(&key('a')));
        assert!(!cached.explicitly_shared);
    }

    let hand_card = |holder: &str, detail: &str| rpc::SceneItemCard {
        thing_id: thing_id.clone(),
        name: thing_name.clone(),
        detail: detail.to_owned(),
        picture_description: picture_description.clone(),
        image_state: rpc::PlateState::Pending as i32,
        state_revision: 1,
        observation_only: true,
        holder: holder.to_owned(),
        owner_id: "merchant".into(),
        holder_id: holder.to_owned(),
        ..Default::default()
    };
    let mut phone = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        hand_items: vec![hand_card("Sigrun", &thing_detail)],
        ..Default::default()
    };
    shared.project_scene_contents(&mut phone, Some("Sigrun"));
    assert_eq!(phone.hand_items[0].image_path, key('a').path());
    assert!(phone.hand_items[0].observation_only);
    assert_eq!(phone.hand_items[0].holder, "Sigrun");

    let mut display = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        hand_items: vec![hand_card("Sigrun", &thing_detail)],
        ..Default::default()
    };
    shared.project_scene_contents(&mut display, None);
    assert_eq!(display.item.as_ref().unwrap().holder, "Sigrun");
    assert_eq!(display.item.as_ref().unwrap().image_path, key('a').path());

    {
        let mut book = locked(&shared.book);
        let current = book.current.as_mut().unwrap();
        let thing = current
            .scene
            .as_mut()
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .model
            .things
            .iter_mut()
            .find(|thing| thing.id == thing_id)
            .unwrap();
        thing.holder_id = Some("Vale".to_owned());
        thing.publicly_visible = false;
    }
    let private_card = hand_card("Vale", &thing_detail);
    let private_contents = locked(&shared.book)
        .current
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .model
        .public_contents()
        .unwrap();
    plates
        .request_hand_item(&private_card, false, &private_contents)
        .unwrap();
    plates
        .request_hand_item(&private_card, false, &private_contents)
        .unwrap();
    assert!(
        drain(&mut queue).is_empty(),
        "reconnect reuses the item image"
    );

    let mut private_phone = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        hand_items: vec![hand_card("Vale", &thing_detail)],
        ..Default::default()
    };
    shared.project_scene_contents(&mut private_phone, Some("Vale"));
    assert_eq!(private_phone.hand_items[0].image_path, key('a').path());

    let mut private_display = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        ..Default::default()
    };
    shared.project_scene_contents(&mut private_display, None);
    assert!(
        private_display.item.is_none(),
        "a cached public hand image cannot reveal a later private hand"
    );

    {
        let mut book = locked(&shared.book);
        let current = book.current.as_mut().unwrap();
        let thing = current
            .scene
            .as_mut()
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .model
            .things
            .iter_mut()
            .find(|thing| thing.id == thing_id)
            .unwrap();
        thing.detail.push_str(" The seam is newly described.");
    }
    let detailed_detail = format!("{thing_detail} The seam is newly described.");
    let changed_private_card = hand_card("Vale", &detailed_detail);
    let changed_private_contents = locked(&shared.book)
        .current
        .as_ref()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .snapshot
        .as_ref()
        .unwrap()
        .model
        .public_contents()
        .unwrap();
    plates
        .request_hand_item(&changed_private_card, false, &changed_private_contents)
        .unwrap();
    assert_eq!(
        drain(&mut queue).len(),
        1,
        "new details get their own image"
    );
    let detailed_card = hand_card("Vale", &detailed_detail);
    let mut detailed_phone = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        hand_items: vec![detailed_card],
        ..Default::default()
    };
    shared.project_scene_contents(&mut detailed_phone, Some("Vale"));
    assert!(
        detailed_phone.hand_items[0].image_path.is_empty(),
        "an image for an earlier detail cannot be attached to a changed card"
    );
}

#[tokio::test]
async fn visible_change_batch_uses_one_provider_edit_from_the_displayed_still() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/campaign/luna6/K03/state-image-fixture-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::new(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Mature,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Arc::new(Shared {
        inner,
        book: Mutex::new(Book::new()),
        jobs,
    });
    let generated = crate::gameplay::npc_dossier_tests::adventure(42);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    observe(&shared, &current, &[], &[1]);
    let Job::Fresh {
        id,
        scene,
        spec,
        heroes,
        faces,
        ..
    } = drain(&mut queue).remove(0)
    else {
        panic!("fresh plate");
    };
    let (base, placement, _) = shared
        .render_fresh(&scene, &spec, &heroes, &faces, &id, 1, false)
        .await
        .unwrap();
    locked(&shared.book).insert(
        id,
        Entry::Ready {
            key: base.clone(),
            placement,
        },
    );
    let mut changed = scene.clone();
    let mut changed_view = view(&current, &[1], false);
    changed_view.graph_revision = 2;
    changed.snapshot.as_mut().unwrap().model.things[0].open = true;
    changed
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Brakka opens the door".into());
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &changed,
        Duration::from_secs(1),
    );
    changed.snapshot.as_mut().unwrap().model.things[0].lit = true;
    changed
        .snapshot
        .as_mut()
        .unwrap()
        .changes
        .push("Sigrun lights the candle".into());
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &changed,
        Duration::from_secs(2),
    );
    shared.poll_state_images_at(Duration::from_secs(5));
    let Job::State(job) = drain(&mut queue).remove(0) else {
        panic!("state edit");
    };
    let done = shared.render_state_image(job).await;
    shared.finish_state_image(done);
    assert_eq!(
        fixture.calls(),
        2,
        "one initial still and one edit for the entire batch"
    );
    assert_ne!(locked(&shared.book).shown().unwrap().0, base);
    assert!(
        fixture
            .prompt()
            .contains("Brakka opens the door; Sigrun lights the candle")
    );
    changed.snapshot.as_mut().unwrap().model.things[0]
        .searched_by
        .push("Brakka".into());
    state_images::observe(
        &mut locked(&shared.book),
        &changed_view,
        &changed,
        Duration::from_secs(6),
    );
    shared.poll_state_images_at(Duration::from_secs(20));
    assert!(drain(&mut queue).is_empty());
    assert_eq!(fixture.calls(), 2);
}

fn apply(shared: &Shared, message: &mut rpc::ViewMessage) {
    let (scene, asset) = match message.audience.as_mut().unwrap() {
        rpc::view_message::Audience::Display(display) => {
            (display.scene.as_mut().unwrap(), display.scene_asset.clone())
        }
        rpc::view_message::Audience::Player(player) => {
            (player.scene.as_mut().unwrap(), String::new())
        }
    };
    let (scene_key, _) = scene_identity(&asset, scene);
    shared.apply(&scene_key, scene);
}

fn ready(shared: &Shared, id: &str, plate: char) {
    shared.finish(Done::Fresh {
        id: id.to_owned(),
        graph_revision: 1,
        ahead: false,
        elapsed: Duration::from_secs(14),
        result: Ok((
            key(plate),
            Some(PlatePlacement {
                actors: Vec::new(),
                speaker: layout(),
                heroes: vec![HeroHead {
                    art: 2,
                    layout: layout(),
                }],
            }),
        )),
    });
}

#[tokio::test]
async fn late_plate_survives_speech_graph_revisions_in_the_same_scene() {
    let (shared, mut queue) = shared(8);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    let first_graph = view(&current, &[1], false);
    shared.observe_with(&first_graph, Some(current.clone()), Vec::new());
    let old_id = fresh_ids(&drain(&mut queue)).remove(0);

    let mut next_graph = view(&current, &[1], false);
    next_graph.graph_revision = 2;
    shared.observe_with(&next_graph, Some(current.clone()), Vec::new());
    assert!(drain(&mut queue).is_empty());

    ready(&shared, &old_id, 'c');
    assert!(matches!(
        locked(&shared.book).entries.get(&old_id),
        Some(Entry::Ready { .. })
    ));
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).plate_state,
        rpc::PlateState::Ready as i32
    );
    assert_eq!(shown_scene(&message).live_plate, key('c').path());
}

#[tokio::test]
async fn ten_lines_in_one_scene_request_only_one_plate_for_the_party() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    for revision in 1..=10 {
        let mut talk = view(&current, &[1, 2], false);
        talk.graph_revision = revision;
        shared.observe_with(&talk, Some(current.clone()), Vec::new());
    }
    assert_eq!(fresh_ids(&drain(&mut queue)).len(), 1);
}

#[tokio::test]
async fn a_modelled_room_coalesces_lasting_expression_changes_into_one_edit() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(41);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (shared, mut queue) = shared(16);
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    locked(&shared.inner.state).plate_wants.insert(
        "keeper".into(),
        Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy,
        },
    );
    shared.poll_face_at(Duration::ZERO);
    shared.poll_face_at(Duration::from_secs(2));
    assert!(drain(&mut queue).is_empty());
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('a'));
    shared.poll_face_at(Duration::from_secs(3));
    assert_eq!(drain(&mut queue).len(), 1);
    shared.poll_face_at(Duration::from_secs(20));
    assert!(
        drain(&mut queue).is_empty(),
        "same target cannot spawn repeated edits"
    );
}

#[tokio::test]
async fn a_prefetched_plate_is_reused_when_its_scene_arrives_on_a_later_graph_revision() {
    let (shared, mut queue) = shared(16);
    let opening = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    let next = scene(SceneKey::Inn, "At the Door", Some("Kestrel"));
    observe(&shared, &opening, &[next.clone()], &[1, 2]);
    let jobs = drain(&mut queue);
    let ahead_id = jobs
        .iter()
        .find_map(|job| match job {
            Job::Fresh {
                id, ahead: true, ..
            } => Some(id.clone()),
            _ => None,
        })
        .unwrap();
    // Ahead completions remain cached even before their scene is current.
    {
        let mut book = locked(&shared.book);
        assert!(matches!(book.entries.get(&ahead_id), Some(Entry::Pending)));
        book.entries.insert(
            ahead_id.clone(),
            Entry::Ready {
                key: key('b'),
                placement: None,
            },
        );
    }
    let mut arrived = view(&next, &[1, 2], false);
    arrived.graph_revision = 10;
    shared.observe_with(&arrived, Some(next), Vec::new());
    assert!(drain(&mut queue).is_empty());
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('b'));
}

#[tokio::test]
async fn a_visible_change_before_the_first_plate_stays_pending_until_that_plate_is_ready() {
    let generated = crate::gameplay::npc_dossier_tests::adventure(41);
    let mut current = scene(SceneKey::Opening, "Stateful room", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = generated.settings.meeting.model.clone();
    let (shared, mut queue) = shared(16);
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    current
        .setting
        .as_mut()
        .unwrap()
        .model
        .as_mut()
        .unwrap()
        .things[0]
        .open = true;
    let mut changed = view(&current, &[1], false);
    changed.graph_revision = 2;
    shared.observe_with(&changed, Some(current), Vec::new());
    shared.poll_state_images_at(Duration::from_secs(5));
    assert!(locked(&shared.book).state_images.pending.is_some());
    assert!(drain(&mut queue).is_empty());
    ready(&shared, &id, 'a');
    shared.poll_state_images_at(Duration::from_secs(6));
    assert!(matches!(drain(&mut queue).as_slice(), [Job::State(_)]));
}

fn layout() -> PlateLayout {
    // Ready-image geometry is a fixed provider result, independent of the director's camera.
    PlateLayout {
        bubble_side: shot::Side::Left,
        head_x_permille: 640,
        head_y_permille: 380,
        head_width_permille: 160,
        head_height_permille: 180,
    }
}

#[tokio::test]
async fn a_new_scene_requests_its_plate_and_its_successors_once_and_never_waits() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    let seal = scene(SceneKey::DialogueSeal, "Hard Questions", Some("Kestrel"));
    let escort = scene(SceneKey::DialogueEscort, "Walk Together", Some("Kestrel"));
    observe(&shared, &current, &[seal.clone(), escort.clone()], &[1, 2]);
    let jobs = drain(&mut queue);
    assert_eq!(fresh_ids(&jobs).len(), 3, "the scene and both successors");
    assert!(
        matches!(&jobs[0], Job::Fresh { scene, ahead: false, .. } if scene.title == "Word Before Dawn"),
        "the current scene's plate is first in the queue and is not held back"
    );
    assert!(
        jobs[1..]
            .iter()
            .all(|job| matches!(job, Job::Fresh { ahead: true, .. })),
        "the successors' plates wait for the current one to reach the image service"
    );
    // The same view again, and a view with a changed revision, ask for nothing more.
    observe(&shared, &current, &[seal, escort], &[1, 2]);
    assert!(drain(&mut queue).is_empty());
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).plate_state,
        rpc::PlateState::Pending as i32
    );
    assert!(shown_scene(&message).plate_update_pending);
    assert!(shown_scene(&message).live_plate.is_empty());
}

#[tokio::test]
async fn a_full_queue_drops_the_request_without_blocking_and_a_later_view_makes_it() {
    let (shared, mut queue) = shared(1);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    let next = scene(SceneKey::DialogueSeal, "Hard Questions", Some("Kestrel"));
    shared.observe_with(
        &view(&current, &[1], false),
        Some(current.clone()),
        vec![next.clone()],
    );
    assert_eq!(fresh_ids(&drain(&mut queue)).len(), 1);
    // The successor did not fit and is not remembered as asked for: the book would not ask again
    // for a scene it believes pending, so it must have forgotten it.
    let book = locked(&shared.book);
    assert_eq!(book.entries.len(), 1);
    drop(book);
    // A later view of the same scene (the party changed) asks again, and the queue has room.
    observe(&shared, &current, &[next], &[1, 2]);
    assert!(!fresh_ids(&drain(&mut queue)).is_empty());
}

#[tokio::test]
async fn a_ready_plate_reaches_the_display_and_the_phones_with_its_layout() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    for mut message in [display_message(&current), player_message(&current)] {
        apply(&shared, &mut message);
        let scene = shown_scene(&message);
        assert_eq!(scene.plate_state, rpc::PlateState::Ready as i32);
        assert!(!scene.plate_update_pending);
        assert_eq!(
            scene.live_plate,
            format!("/gameplay/imagery/{}", "a".repeat(64))
        );
        let layout = scene.plate_layout.as_ref().unwrap();
        assert_eq!(layout.bubble_side, rpc::PlateSide::Left as i32);
        assert_eq!(layout.head_x_permille, 640);
        assert_eq!(layout.head_y_permille, 380);
        assert_eq!(layout.heroes.len(), 1);
        assert_eq!(
            layout.heroes[0].hero_index, 1,
            "art 2 is the party's second hero"
        );
        assert_eq!(layout.heroes[0].bubble_side, rpc::PlateSide::Left as i32);
    }
    assert!(locked(&shared.inner.state).plate_covers);
}

#[tokio::test]
async fn a_plate_is_shown_with_its_predicted_layout_and_the_measured_heads_replace_it_later() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    let mut before = display_message(&current);
    apply(&shared, &mut before);
    assert_eq!(
        shown_scene(&before).plate_state,
        rpc::PlateState::Ready as i32,
        "the plate does not wait for the vision model to find the heads"
    );
    shared.finish(Done::Located {
        id: id.clone(),
        key: key('a'),
        elapsed: Duration::from_secs(20),
        placement: PlatePlacement {
            actors: Vec::new(),
            speaker: shot::measured_layout(0.30, 0.20, 0.40, 0.36),
            heroes: Vec::new(),
        },
    });
    let mut after = display_message(&current);
    apply(&shared, &mut after);
    let layout = shown_scene(&after).plate_layout.as_ref().unwrap();
    assert_ne!(
        layout.head_x_permille, 640,
        "the measured heads replaced the prediction"
    );
    // Heads found for a plate that has since been replaced change nothing.
    shared.finish(Done::Located {
        id,
        key: key('b'),
        elapsed: Duration::from_secs(20),
        placement: PlatePlacement {
            actors: Vec::new(),
            speaker: shot::measured_layout(0.80, 0.20, 0.90, 0.36),
            heroes: Vec::new(),
        },
    });
    let mut again = display_message(&current);
    apply(&shared, &mut again);
    assert_eq!(
        shown_scene(&again).plate_layout,
        shown_scene(&after).plate_layout
    );
}

#[tokio::test]
async fn a_failed_plate_changes_nothing_but_the_state_and_is_asked_for_again_when_the_scene_returns()
 {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    shared.finish(Done::Fresh {
        id,
        graph_revision: 1,
        ahead: false,
        elapsed: Duration::from_secs(45),
        result: Err(ImageFailure::DeadlineExceeded),
    });
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    let scene_after = shown_scene(&message).clone();
    assert_eq!(scene_after.plate_state, rpc::PlateState::Failed as i32);
    assert!(!scene_after.plate_update_pending);
    assert!(scene_after.live_plate.is_empty() && scene_after.plate_layout.is_none());
    assert!(!locked(&shared.inner.state).plate_covers);
    // Another scene, then back: the failed plate is requested once more.
    let other = scene(SceneKey::DialogueSeal, "Hard Questions", Some("Kestrel"));
    observe(&shared, &other, &[], &[1]);
    drain(&mut queue);
    observe(&shared, &current, &[], &[1]);
    assert_eq!(fresh_ids(&drain(&mut queue)).len(), 1);
}

#[tokio::test]
async fn a_fight_gets_no_plate_and_hides_a_ready_one() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    shared.observe_with(
        &view(&current, &[1], true),
        Some(current.clone()),
        Vec::new(),
    );
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(shown_scene(&message).plate_state, 0);
    assert!(shown_scene(&message).live_plate.is_empty());
    assert!(!locked(&shared.inner.state).plate_covers);
    let fight = scene(SceneKey::Combat, "No Quarter Asked", Some("Kestrel"));
    observe(&shared, &fight, &[], &[1]);
    assert!(
        drain(&mut queue).is_empty(),
        "a fight scene is never plated"
    );
}

#[tokio::test]
async fn a_lasting_face_edits_the_current_plate_and_the_neutral_face_returns_to_it() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    shared.poll_face();
    assert!(
        drain(&mut queue).is_empty(),
        "nothing to edit before the plate exists"
    );
    ready(&shared, &id, 'a');
    let angry = Face {
        expression: Expression::Angry,
        tier: HealthTier::Healthy,
    };
    locked(&shared.inner.state)
        .plate_wants
        .insert("keeper".to_owned(), angry);
    shared.poll_face();
    shared.poll_face();
    let jobs = drain(&mut queue);
    assert_eq!(jobs.len(), 1, "one edit however often the face is polled");
    let Job::Edit { base, face, .. } = &jobs[0] else {
        panic!("an edit of the plate, not a fresh render");
    };
    assert_eq!((base, *face), (&key('a'), angry));
    let mut edited_layout = layout();
    edited_layout.head_x_permille = 700;
    locked(&shared.book).image_layouts.insert(
        key('b').as_str().into(),
        PlatePlacement {
            speaker: edited_layout,
            heroes: vec![],
            actors: vec![],
        },
    );
    shared.finish(Done::Edit {
        scene_key: "opening|Word Before Dawn|assets/ui/generated/scene-place-bell-chamber.webp"
            .to_owned(),
        speaker_id: Some("keeper".to_owned()),
        face: angry,
        elapsed: Duration::from_secs(15),
        result: Ok(key('b')),
    });
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).live_plate,
        format!("/gameplay/imagery/{}", "b".repeat(64)),
        "the edited plate replaces the first one, with its own measured layout"
    );
    assert_eq!(
        shown_scene(&message)
            .plate_layout
            .as_ref()
            .unwrap()
            .head_x_permille,
        700
    );
    locked(&shared.inner.state)
        .plate_wants
        .insert("keeper".to_owned(), Face::AUTHORED);
    shared.poll_face();
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).live_plate,
        format!("/gameplay/imagery/{}", "a".repeat(64)),
        "the neutral face shows the first plate again"
    );
    assert!(drain(&mut queue).is_empty());
}

#[tokio::test]
async fn an_edit_that_lands_in_another_scene_is_ignored() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    let angry = Face {
        expression: Expression::Angry,
        tier: HealthTier::Healthy,
    };
    locked(&shared.inner.state)
        .plate_wants
        .insert("keeper".to_owned(), angry);
    shared.poll_face();
    drain(&mut queue);
    let other = scene(SceneKey::DialogueSeal, "Hard Questions", Some("Kestrel"));
    observe(&shared, &other, &[], &[1]);
    shared.finish(Done::Edit {
        scene_key: "opening|Word Before Dawn|assets/ui/generated/scene-place-bell-chamber.webp"
            .to_owned(),
        speaker_id: Some("keeper".to_owned()),
        face: angry,
        elapsed: Duration::from_secs(15),
        result: Ok(key('b')),
    });
    assert!(matches!(locked(&shared.book).edit, Edit::Idle));
}

#[tokio::test]
async fn a_creature_speaker_means_no_plate_and_no_failure_but_never_cut_outs() {
    let (shared, mut queue) = shared(16);
    let mut current = scene(SceneKey::Opening, "A Rat in the Cellar", Some("Gnaw"));
    current.cast = vec![member(
        "rat",
        "Gnaw",
        "sprite-ant-giant-rat",
        pack::CastRole::Antagonist,
    )];
    observe(&shared, &current, &[], &[1]);
    assert!(drain(&mut queue).is_empty());
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).plate_state,
        rpc::PlateState::Unavailable as i32,
        "no plate will come, and the display is told so instead of being left to guess"
    );
}

#[tokio::test]
async fn a_party_that_changes_in_a_scene_gets_a_plate_for_the_new_party() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let first = fresh_ids(&drain(&mut queue));
    observe(&shared, &current, &[], &[1, 2]);
    let second = fresh_ids(&drain(&mut queue));
    assert_eq!((first.len(), second.len()), (1, 1));
    assert_ne!(first, second);
}

#[tokio::test]
async fn three_lobby_rosters_request_one_opening_plate_for_the_confirmed_party() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/campaign/luna2/R10/plate-cache-{}",
        std::process::id(),
    ));
    let fixture = DirectionFixture::new(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Shared {
        inner,
        book: Mutex::new(Book::new()),
        jobs,
    };
    let lobby = scene(SceneKey::Room, "The Gathering", None);
    let opening = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    for heroes in [&[][..], &[1][..], &[1, 2][..]] {
        shared.observe_with(&view(&lobby, heroes, false), None, vec![opening.clone()]);
        assert!(drain(&mut queue).is_empty(), "the lobby requests no plates");
    }
    shared.observe_with(
        &view(&opening, &[1, 2, 3], false),
        Some(opening),
        Vec::new(),
    );
    let jobs = drain(&mut queue);
    assert!(
        matches!(&jobs[..], [Job::Fresh { heroes, .. }] if heroes == &[1, 2, 3]),
        "the opening scene requests one plate for the confirmed party"
    );
    let [
        Job::Fresh {
            scene,
            heroes,
            faces,
            ..
        },
    ] = jobs.as_slice()
    else {
        panic!("one fresh opening plate");
    };
    let spec = scene.stage(heroes).unwrap();
    let (request, _) = shared
        .fresh_request(&scene, &spec, heroes, faces, None)
        .await
        .unwrap();
    fixture.imagery.render(request).await.unwrap();
    assert_eq!(
        fixture.calls(),
        1,
        "one provider call for the opening plate",
    );
}

#[tokio::test]
async fn a_scene_plate_with_two_heroes_and_two_cast_members_stays_within_reference_limit() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let cache = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/campaign/luna2/R10/reference-limit-{}",
        std::process::id(),
    ));
    let fixture = DirectionFixture::new(&cache);
    let timing = Timing::from_interval(Duration::from_secs(30));
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: super::super::ReferenceStore::new(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
        ),
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, _queue) = mpsc::channel(16);
    let shared = Shared {
        inner,
        book: Mutex::new(Book::new()),
        jobs,
    };
    let mut current = scene(SceneKey::DialogueSeal, "Two on the Stage", Some("Kestrel"));
    current.cast.push(member(
        "antagonist",
        "The Antagonist",
        "sprite-ant-giant-rat",
        pack::CastRole::Antagonist,
    ));
    let Some(scene) = PlateScene::of(&current) else {
        panic!("scene should be plannable");
    };
    let spec = scene.stage(&[1, 2]).expect("scene should stage");
    let (request, _) = shared
        .fresh_request(&scene, &spec, &[1, 2], &[], None)
        .await
        .expect("reference shaping should succeed");

    assert_eq!(request.references.len(), 4);
    fixture
        .imagery
        .render(request)
        .await
        .expect("provider-free fixture should accept the shaped request");
}

#[tokio::test]
async fn fixed_harbor_edits_the_reviewed_full_frame_before_identity_references() {
    let timing = Timing::from_interval(Duration::from_secs(30));
    let references = super::super::ReferenceStore::photographic(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/ui/generated"),
    );
    let base = references
        .load_photographic("scene-salt-quay-candid")
        .unwrap();
    let courier = references.load_photographic("sprite-courier").unwrap();
    let inner = Arc::new(Inner {
        imagery: Imagery::disabled(ImageryOff::Qualification),
        references,
        social: SocialLedger::none(),
        rating: ContentRating::Restrained,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: tokio::sync::watch::channel(0).0,
        origin: Instant::now(),
    });
    let (jobs, mut queue) = mpsc::channel(16);
    let shared = Shared {
        inner,
        book: Mutex::new(Book::new()),
        jobs,
    };
    let document: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../../../content/harbor-journey/pack.json"
    ))
    .unwrap();
    let current: pack::Scene = document["scenes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| serde_json::from_value::<pack::Scene>(value.clone()).unwrap())
        .find(|scene| scene.key == SceneKey::DialogueSeal)
        .unwrap();
    let mut scene = PlateScene::of(&current).unwrap();
    let spec = scene.stage(&[1, 2]).unwrap();
    let (request, _) = shared
        .fresh_request(&scene, &spec, &[1, 2], &[], None)
        .await
        .unwrap();
    assert_eq!(request.references.len(), 3);
    assert_eq!(request.references[0].sha256, base.sha256);
    assert_eq!(request.references[2].sha256, courier.sha256);
    assert!(request.prompt.as_str().contains("hero:1"));
    assert!(request.prompt.as_str().contains("hero:2"));
    for identity in &scene.identity_cast {
        assert!(request.prompt.as_str().contains(&identity.id));
    }
    // The authored automatic-story path keeps the same canonical reference contract.
    let mut story_view = view(&current, &[1, 2], false);
    shared.observe_with(&story_view, Some(current.clone()), Vec::new());
    let id = fresh_ids(&drain(&mut queue)).remove(0);
    ready(&shared, &id, 'a');
    story_view.story = Some(StoryBeat {
        actor_id: String::new(),
        sequence: 7,
        speaker: current.speaker.clone(),
        text: "The enforcer listens.".into(),
        sha256: "test-story-hash".into(),
        tempo: crate::gameplay::shot::PublicBeat::Build,
        dialogue: crate::gameplay::shot::PublicDialogue::FirstReply,
    });
    story_view.graph_revision = 2;
    shared.observe_with(&story_view, Some(current.clone()), Vec::new());
    shared.poll_state_images_at(shared.inner.now() + Duration::from_secs(4));
    let Job::State(state_images::Job::Bake {
        mut pending,
        identities,
        ..
    }) = drain(&mut queue).remove(0)
    else {
        panic!("automatic story edit")
    };
    let edit = shared
        .state_image_request(&pending, base.clone(), identities.as_deref())
        .await
        .unwrap();
    assert_eq!(edit.references.len(), 3);
    assert_eq!(edit.references[0].sha256, base.sha256);
    assert_eq!(edit.references[1].sha256, request.references[1].sha256);
    assert_eq!(edit.references[2].sha256, courier.sha256);
    assert!(edit.prompt.as_str().contains("The enforcer listens."));
    assert!(edit.prompt.as_str().contains("clearly readable change"));
    assert!(
        edit.prompt
            .as_str()
            .contains("An eyebrow-only expression tweak is insufficient")
    );
    assert!(
        edit.prompt
            .as_str()
            .contains("bounded public-beat camera direction")
    );
    assert!(edit.prompt.as_str().contains("actor_id"));
    assert!(
        edit.prompt
            .as_str()
            .contains("Do not infer or enact mechanical outcomes")
    );
    let edit_key = shared.inner.imagery.key_for(&edit);
    pending.story.as_mut().unwrap().text = "Keep your voice low.".into();
    pending.story.as_mut().unwrap().tempo = crate::gameplay::shot::PublicBeat::Peak;
    pending.story.as_mut().unwrap().sequence = 8;
    let changed = shared
        .state_image_request(&pending, base.clone(), identities.as_deref())
        .await
        .unwrap();
    assert_ne!(edit_key, shared.inner.imagery.key_for(&changed));
    assert!(changed.prompt.as_str().contains("Dutch-angle"));
    let (rebuilt, _) = shared
        .fresh_request(&scene, &spec, &[2, 1], &[], None)
        .await
        .unwrap();
    assert_eq!(request.references[1].sha256, rebuilt.references[1].sha256);
    assert!(
        request
            .prompt
            .as_str()
            .starts_with("Edit image 1 as the actual photograph")
    );
    assert!(!request.prompt.as_str().contains("best-lit person"));
    assert!(!request.prompt.as_str().contains("deep blue light outside"));

    scene.place_id = "another-quay".to_owned();
    let (other, _) = shared
        .fresh_request(&scene, &spec, &[1, 2], &[], None)
        .await
        .unwrap();
    assert_ne!(other.references[0].sha256, base.sha256);
    assert!(
        !other
            .prompt
            .as_str()
            .starts_with("Edit image 1 as the actual photograph")
    );
}

#[tokio::test]
async fn a_planned_scene_whose_request_was_dropped_is_pending_and_is_asked_for_by_the_next_view() {
    let (shared, mut queue) = shared(1);
    let other = scene(SceneKey::DialogueSeal, "Hard Questions", Some("Kestrel"));
    let current = scene(SceneKey::Opening, "Word Before Dawn", Some("Kestrel"));
    // The queue holds one request; the current scene's request after it does not fit.
    shared.observe_with(&view(&other, &[1], false), Some(other.clone()), Vec::new());
    shared.observe_with(
        &view(&current, &[1], false),
        Some(current.clone()),
        Vec::new(),
    );
    assert!(locked(&shared.book).current_entry().is_none());
    let mut message = display_message(&current);
    apply(&shared, &mut message);
    assert_eq!(
        shown_scene(&message).plate_state,
        rpc::PlateState::Pending as i32,
        "a plate that is on its way is never mistaken for a scene without one"
    );
    drain(&mut queue);
    observe(&shared, &current, &[], &[1]);
    assert_eq!(
        fresh_ids(&drain(&mut queue)).len(),
        1,
        "the next view of the same scene asks again"
    );
}

#[test]
fn the_successors_of_a_scene_follow_a_won_fight_and_the_lobby_leads_to_the_opening() {
    let keys = |scene: &pack::Scene| -> Vec<SceneKey> {
        successors(scene).into_iter().map(|next| next.key).collect()
    };
    assert_eq!(keys(&pack::scene(SceneKey::Room)), [SceneKey::Opening]);
    assert_eq!(
        keys(&pack::scene(SceneKey::Opening)),
        [SceneKey::DialogueSeal, SceneKey::DialogueEscort]
    );
    for answer in [SceneKey::DialogueSeal, SceneKey::DialogueEscort] {
        assert_eq!(
            keys(&pack::scene(answer)),
            [SceneKey::Victory],
            "the fight is never plated, so its won scene is the one made ahead"
        );
    }
}

/// A mystery chapter's shape: a look keeps the place, a journey leads to a new place, and the
/// ending follows it.
fn mystery_script() -> (BeatScript, Vec<pack::Scene>) {
    let mut scenes = std::collections::BTreeMap::new();
    for (id, key, title) in [
        ("opening", SceneKey::Opening, "Traces to Follow"),
        ("crates", SceneKey::Opening, "Marks on the Ground"),
        ("travel", SceneKey::DialogueEscort, "The Way Onward"),
        ("destination", SceneKey::Inn, "A Journey Settled"),
        ("epilogue", SceneKey::Inn, "The Ending"),
        ("end", SceneKey::Inn, "The Last Word"),
    ] {
        scenes.insert(id.to_owned(), scene(key, title, Some("Kestrel")));
    }
    let beat = |id: &str, kind: &str, targets: &[&str]| {
        serde_json::json!({
            "id": id, "kind": kind, "scene": "opening", "scene_variant": id,
            "state": { "durable": id }, "offers": [],
            "transitions": targets.iter().map(|target| serde_json::json!({
                "trigger": { "on": "offer", "offer": "go" }, "target": target
            })).collect::<Vec<_>>(),
        })
    };
    let script = serde_json::from_value(serde_json::json!({
        "entry": "opening", "terminal": ["end"], "scenes": scenes,
        "beats": [
            beat("opening", "investigate", &["crates", "travel"]),
            beat("crates", "investigate", &["crates", "travel"]),
            beat("travel", "travel", &["destination"]),
            beat("destination", "ending", &["epilogue"]),
            beat("epilogue", "ending", &["end"]),
            beat("end", "ending", &[]),
        ],
    }))
    .unwrap();
    (script, Vec::new())
}

#[test]
fn the_destination_of_a_journey_and_the_ending_after_it_are_made_while_the_party_is_still_on_the_way()
 {
    let (script, scenes) = mystery_script();
    let titles = |from: &str| -> Vec<String> {
        let current = script.scenes[from].clone();
        successors_in(&script, &scenes, &current)
            .into_iter()
            .map(|next| next.title)
            .collect()
    };
    assert_eq!(
        titles("opening"),
        [
            "Marks on the Ground",
            "The Way Onward",
            "A Journey Settled",
            "The Ending"
        ],
        "the look, the journey, and the new place with its ending, nearest first"
    );
    assert_eq!(
        titles("travel"),
        ["A Journey Settled", "The Ending", "The Last Word"]
    );
}

#[test]
fn the_party_comes_from_the_ready_members_by_their_art() {
    let message = rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                scene: Some(rpc::GameplayScene {
                    title: "T".to_owned(),
                    scene_asset: "a".to_owned(),
                    ..Default::default()
                }),
                journey: Some(rpc::JourneyView {
                    party: vec![
                        rpc::PartyMemberView {
                            character_ready: true,
                            ..Default::default()
                        },
                        rpc::PartyMemberView::default(),
                        rpc::PartyMemberView {
                            character_ready: true,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    };
    assert_eq!(view_of(&message, 0).unwrap().heroes, [1, 3]);
    assert!(view_of(&player_message(&scene(SceneKey::Opening, "T", None)), 0).is_none());
}

#[tokio::test]
async fn a_hurt_hero_stands_in_the_plate_as_their_health_variant_and_makes_another_plate() {
    let (shared, mut queue) = shared(16);
    let current = scene(SceneKey::Victory, "What the Fight Cost", Some("Kestrel"));
    observe(&shared, &current, &[], &[1, 2]);
    let healthy = drain(&mut queue);
    assert!(matches!(&healthy[0], Job::Fresh { faces, .. } if faces.is_empty()));
    locked(&shared.inner.state).hero_faces.insert(
        "Hero 1".to_owned(),
        crate::gameplay::living_scenes::HeroFace {
            stage: Some(key('d')),
            battle: None,
        },
    );
    // The same party with a hurt first hero in another scene: its plate is another picture.
    let other = scene(SceneKey::Victory, "What the Fight Cost 2", Some("Kestrel"));
    observe(&shared, &other, &[], &[1, 2]);
    let hurt = drain(&mut queue);
    let Job::Fresh { id, faces, .. } = &hurt[0] else {
        panic!("a fresh plate");
    };
    assert_eq!(faces, &[(1, key('d'))], "only the hurt hero has a variant");
    assert!(id.contains("dddddddd"), "the variant names the plate: {id}");
}

#[test]
fn a_measured_placement_survives_the_note_beside_the_cached_plate_and_a_damaged_note_is_refused() {
    let placement = PlatePlacement {
        actors: Vec::new(),
        speaker: layout(),
        heroes: vec![HeroHead {
            art: 1,
            layout: layout(),
        }],
    };
    let note = serde_json::to_vec(&StoredPlacement::of(&placement)).unwrap();
    let restored = serde_json::from_slice::<StoredPlacement>(&note)
        .unwrap()
        .placement()
        .unwrap();
    assert_eq!(restored, placement);
    let damaged = String::from_utf8(note)
        .unwrap()
        .replace("\"width\":", "\"width\":9000,\"unused\":");
    assert!(
        serde_json::from_str::<StoredPlacement>(&damaged)
            .ok()
            .and_then(|stored| stored.placement())
            .is_none(),
        "numbers outside the plate are not trusted"
    );
}

#[test]
fn locator_deduplicates_focus_and_maps_all_four_harbor_people_to_admitted_ids() {
    let mut scene = scene(SceneKey::Opening, "The Wharf", Some("Kestrel"));
    scene.cast.push(member(
        "bandit",
        "Bandit",
        "figure-bandit-documentary-photo",
        pack::CastRole::Antagonist,
    ));
    let scene = PlateScene::of(&scene).unwrap();
    let spec = scene.stage(&[1, 2]).unwrap();
    let (focus, targets) = locator_targets(
        &spec,
        spec.speaker.as_ref().unwrap(),
        &scene.locator_actors(),
        Some("keeper"),
    );
    assert_eq!(focus, "npc-0");
    assert_eq!(
        targets
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
        ["npc-0", "hero-1", "hero-2", "npc-1"]
    );
    assert!(targets.chunks(4).all(|batch| batch.len() <= 4));
    assert!(targets.iter().all(|(id, description)| {
        id.len() <= 16
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && description.chars().count() <= 300
    }));
}
#[test]
fn edited_image_never_borrows_actor_heads_from_a_different_image_key() {
    let mut book = Book::new();
    let old = PlatePlacement {
        speaker: layout(),
        heroes: vec![],
        actors: vec![ActorHead {
            actor_id: "bandit".into(),
            layout: layout(),
        }],
    };
    book.state_images.shown = Some((key('a'), Some(old.clone())));
    book.image_layouts
        .insert(key('a').as_str().into(), old.clone());
    book.edit = Edit::Ready {
        face: Face::AUTHORED,
        key: key('b'),
    };
    assert!(book.shown().unwrap().1.is_none());
    book.image_layouts
        .insert(key('b').as_str().into(), old.clone());
    assert_eq!(book.shown().unwrap().1, Some(old));
}

#[tokio::test]
async fn four_person_locator_batch_reaches_the_real_provider_adapter_fixture() {
    use crate::gameplay::storyteller::tests::{FakeEndpoint, Reply};
    use df_providers::{HeadLocateConfig, HeadLocateError, LocateTarget, NativeHeadLocator};
    let endpoint = FakeEndpoint::start(vec![Reply::Status(503, serde_json::json!({}))]);
    let locator = NativeHeadLocator::new(
        HeadLocateConfig::loopback_fixture(endpoint.address).unwrap(),
        "fixture",
        Duration::from_secs(2),
    )
    .unwrap();
    let mut scene = scene(SceneKey::Opening, "The Wharf", Some("Kestrel"));
    scene.cast.push(member(
        "bandit",
        "Bandit",
        "figure-bandit-documentary-photo",
        pack::CastRole::Antagonist,
    ));
    let scene = PlateScene::of(&scene).unwrap();
    let spec = scene.stage(&[1, 2]).unwrap();
    let (_, targets) = locator_targets(
        &spec,
        spec.speaker.as_ref().unwrap(),
        &scene.locator_actors(),
        Some("keeper"),
    );
    let borrowed = targets
        .iter()
        .map(|(id, description)| LocateTarget { id, description })
        .collect::<Vec<_>>();
    assert_eq!(
        locator
            .locate(b"jpeg-fixture", None, &borrowed, Duration::from_secs(2))
            .await,
        Err(HeadLocateError::Provider(503))
    );
    assert_eq!(
        endpoint.connections(),
        1,
        "local admission allowed the actual four-person request"
    );
    let body = &endpoint.bodies()[0];
    let instruction = body["messages"][0]["content"][0]["text"].as_str().unwrap();
    for (id, _) in targets {
        assert_eq!(instruction.matches(&format!("{id}: ")).count(), 1);
    }
}

#[tokio::test]
async fn a_failed_measurement_is_remembered_in_process_without_repeating_the_locator() {
    let (shared, _) = shared(16);
    let scene = PlateScene::of(&scene(SceneKey::Opening, "The Wharf", Some("Kestrel"))).unwrap();
    let spec = scene.stage(&[1, 2]).unwrap();
    let mut fallback = layout();
    fallback.head_x_permille = 123;
    let failed = PlatePlacement {
        speaker: fallback,
        heroes: vec![],
        actors: vec![],
    };
    locked(&shared.book)
        .image_layouts
        .insert(key('a').as_str().into(), failed.clone());
    let known = shared
        .measured_or_predicted(
            &key('a'),
            &spec,
            spec.speaker.as_ref().unwrap(),
            &BTreeMap::new(),
            &scene.locator_actors(),
            Some("keeper"),
        )
        .await;
    assert_eq!(
        known, failed,
        "repeated views reuse even a missing-head result until a new owner/run"
    );
}

#[test]
fn head_diagnostics_are_plate_filterable_correlated_and_bounded_without_prompt_text() {
    let image = key('a');
    let partial = HeadDiagnostic {
        key: &image,
        stage: "head_location",
        reference_sha256: None,
        source: "provider",
        batch: Some(0),
        targets: vec!["npc-0".into(), "npc-1".into()],
        located: vec!["npc-0".into()],
        actor_ids: vec![
            ("npc-0".into(), "courier".into()),
            ("npc-1".into(), "bandit".into()),
        ],
        elapsed_ms: 1234,
        failure: None,
    }
    .fields();
    assert_eq!(partial["resource_type"], "plate");
    assert_eq!(partial["stage"], "head_location");
    assert_eq!(partial["status"], "partial");
    assert_eq!(partial["partial"], true);
    assert_eq!(partial["image_key"], image.as_str());
    assert_eq!(partial["actor_id_mapping"]["npc-1"], "bandit");
    assert_eq!(partial["elapsed_ms"], 1234);
    assert_eq!(partial["cache_source"], "provider");
    let failed = HeadDiagnostic {
        key: &image,
        stage: "head_location",
        reference_sha256: None,
        source: "provider",
        batch: Some(1),
        targets: vec!["npc-1".into()],
        located: vec![],
        actor_ids: vec![],
        elapsed_ms: 10,
        failure: Some("provider_failed"),
    }
    .fields();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["failure_category"], "provider_failed");
    let failed_complete = HeadDiagnostic {
        key: &image,
        stage: "actor_heads",
        reference_sha256: None,
        source: "provider",
        batch: None,
        targets: vec!["bandit".into()],
        located: vec!["bandit".into()],
        actor_ids: vec![],
        elapsed_ms: 10,
        failure: Some("provider_failed"),
    }
    .fields();
    assert_eq!(
        failed_complete["status"], "partial",
        "a failed batch cannot be reported ready"
    );
    let cached = HeadDiagnostic {
        key: &image,
        stage: "actor_heads",
        reference_sha256: None,
        source: "durable_note",
        batch: None,
        targets: vec!["bandit".into()],
        located: vec!["bandit".into()],
        actor_ids: vec![],
        elapsed_ms: 2,
        failure: None,
    }
    .fields();
    assert_eq!(cached["status"], "ready");
    assert_eq!(cached["cache_source"], "durable_note");
    assert_eq!(cached["target_ids"][0], "bandit");
    let bounded = HeadDiagnostic {
        key: &image,
        stage: "actor_heads",
        reference_sha256: None,
        source: "in_process",
        batch: None,
        targets: vec!["x".repeat(1000); 100],
        located: vec![],
        actor_ids: vec![],
        elapsed_ms: 0,
        failure: None,
    }
    .fields();
    assert_eq!(bounded["ids_truncated"], true);
    assert_eq!(bounded["target_ids"].as_array().unwrap().len(), 16);
    assert!(serde_json::to_vec(&bounded).unwrap().len() < 4096);
    assert!(partial.get("prompt").is_none());
}

#[test]
fn every_locator_batch_correlates_its_sole_target_with_its_own_pinned_reference() {
    let courier = ReferenceImage::new(
        df_providers::ImageFormat::Webp,
        bytes::Bytes::from_static(b"courier"),
    );
    let bandit = ReferenceImage::new(
        df_providers::ImageFormat::Webp,
        bytes::Bytes::from_static(b"bandit"),
    );
    let references = BTreeMap::from([
        ("courier".into(), courier.clone()),
        ("bandit".into(), bandit.clone()),
        (
            "hero-1".into(),
            ReferenceImage::new(
                df_providers::ImageFormat::Webp,
                bytes::Bytes::from_static(b"brakka"),
            ),
        ),
        (
            "hero-2".into(),
            ReferenceImage::new(
                df_providers::ImageFormat::Webp,
                bytes::Bytes::from_static(b"sigrun"),
            ),
        ),
    ]);
    let actors = vec![
        ("courier".into(), "Courier".into()),
        ("bandit".into(), "Bandit".into()),
    ];
    let targets = vec![
        ("npc-0".into(), "Courier".into()),
        ("hero-1".into(), "hero".into()),
        ("hero-2".into(), "hero".into()),
        ("npc-1".into(), "Bandit".into()),
    ];
    let batches = referenced_locator_batches(&targets, &actors, &references);
    assert_eq!(batches.len(), 4);
    for (batch, expected) in batches.iter().zip(["npc-0", "hero-1", "hero-2", "npc-1"]) {
        assert_eq!(batch.targets.len(), 1);
        assert_eq!(batch.targets[0].0, expected);
    }
    assert_eq!(batches[0].reference.sha256, courier.sha256);
    assert_eq!(batches[1].reference.sha256, references["hero-1"].sha256);
    assert_eq!(batches[2].reference.sha256, references["hero-2"].sha256);
    assert_eq!(batches[3].reference.sha256, bandit.sha256);
    let missing = BTreeMap::from([("courier".into(), courier)]);
    let batches = referenced_locator_batches(&targets, &actors, &missing);
    assert!(
        !batches
            .iter()
            .flat_map(|batch| &batch.targets)
            .any(|target| target.0 == "npc-1"),
        "missing canonical art never becomes a filename-only identity guess"
    );
    let many = (0..20)
        .map(|index| (format!("npc-{index}"), "actor".into()))
        .collect::<Vec<_>>();
    let actors = (0..20)
        .map(|index| (format!("actor-{index}"), "actor".into()))
        .collect::<Vec<_>>();
    let references = actors
        .iter()
        .map(|(id, _)| (id.clone(), bandit.clone()))
        .collect();
    assert_eq!(
        referenced_locator_batches(&many, &actors, &references).len(),
        4
    );
}

#[test]
fn locator_cache_provenance_changes_with_each_canonical_source() {
    let reference = ReferenceImage::new(
        df_providers::ImageFormat::Webp,
        bytes::Bytes::from_static(b"old"),
    );
    let mut references = BTreeMap::from([("bandit".into(), reference)]);
    let old = reference_hashes(&references);
    references.insert(
        "bandit".into(),
        ReferenceImage::new(
            df_providers::ImageFormat::Webp,
            bytes::Bytes::from_static(b"new"),
        ),
    );
    assert_ne!(old, reference_hashes(&references));
    let legacy = serde_json::json!({"speaker":{"art":0,"x":500,"y":300,"width":100,"height":100,"bubble_left":true},"heroes":[]});
    let legacy: StoredPlacement = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        legacy.locator_revision, 0,
        "unreferenced historic notes cannot satisfy revision2"
    );
}

#[tokio::test]
async fn reference_measurements_are_polled_concurrently_inside_one_owned_deadline() {
    let barrier = tokio::sync::Barrier::new(4);
    let pending = FuturesUnordered::new();
    for index in 0..4 {
        let barrier = &barrier;
        pending.push(async move {
            barrier.wait().await;
            (
                index,
                HeadResult {
                    elapsed_ms: 1,
                    heads: Ok(Vec::new()),
                },
            )
        });
    }
    let results = collect_head_results(
        pending,
        4,
        tokio::time::Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert!(
        results.iter().all(Option::is_some),
        "all four must be polled before the barrier can release; serial dispatch cannot finish"
    );
}

#[tokio::test]
async fn image_measurement_deadline_preserves_completed_heads_and_drops_unfinished_requests() {
    let dropped = std::sync::atomic::AtomicBool::new(false);
    struct Dropped<'a>(&'a std::sync::atomic::AtomicBool);
    impl Drop for Dropped<'_> {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let pending = FuturesUnordered::new();
    for index in 0..2 {
        let dropped = &dropped;
        pending.push(async move {
            if index == 1 {
                let _owned = Dropped(dropped);
                std::future::pending::<()>().await;
            }
            (
                index,
                HeadResult {
                    elapsed_ms: 0,
                    heads: Ok(Vec::new()),
                },
            )
        });
    }
    let results = collect_head_results(
        pending,
        2,
        tokio::time::Instant::now() + Duration::from_millis(5),
    )
    .await;
    assert!(results[0].is_some());
    assert!(results[1].is_none());
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn missing_focus_identity_never_borrows_a_named_or_speaker_reference() {
    let scene = PlateScene::of(&scene(SceneKey::Opening, "The Wharf", Some("Kestrel"))).unwrap();
    let spec = scene.stage(&[]).unwrap();
    let actors = vec![
        ("keeper".into(), "Kestrel".into()),
        ("speaker".into(), "Another canonical NPC".into()),
    ];
    let keeper = ReferenceImage::new(
        df_providers::ImageFormat::Webp,
        bytes::Bytes::from_static(b"keeper"),
    );
    let other = ReferenceImage::new(
        df_providers::ImageFormat::Webp,
        bytes::Bytes::from_static(b"other-speaker"),
    );
    let references = BTreeMap::from([
        ("keeper".into(), keeper.clone()),
        ("speaker".into(), other.clone()),
    ]);
    for focus in [None, Some("not-present")] {
        let (focus, targets) =
            locator_targets(&spec, spec.speaker.as_ref().unwrap(), &actors, focus);
        assert_eq!(focus, "speaker");
        let batches = referenced_locator_batches(&targets, &actors, &references);
        assert_eq!(
            batches.len(),
            2,
            "known canonical people remain independently measurable"
        );
        assert!(batches.iter().all(|batch| batch.targets[0].0 != "speaker"));
        assert_eq!(batches[0].reference.sha256, keeper.sha256);
        assert_eq!(batches[1].reference.sha256, other.sha256);
    }
    let (focus, targets) = locator_targets(
        &spec,
        spec.speaker.as_ref().unwrap(),
        &actors,
        Some("speaker"),
    );
    assert_eq!(
        focus, "npc-1",
        "an explicitly validated canonical ID uses the indexed target, never the unbound slot"
    );
    let batches = referenced_locator_batches(&targets, &actors, &references);
    assert_eq!(batches[0].targets[0].0, "npc-1");
    assert_eq!(batches[0].reference.sha256, other.sha256);
    let diagnostic = HeadDiagnostic {
        key: &key('a'),
        stage: "head_location",
        reference_sha256: None,
        source: "identity_validation",
        batch: None,
        actor_ids: vec![],
        targets: vec!["speaker".into()],
        located: vec![],
        elapsed_ms: 0,
        failure: Some("missing_focus_identity"),
    }
    .fields();
    assert_eq!(diagnostic["status"], "failed");
    assert_eq!(diagnostic["failure_category"], "missing_focus_identity");
    assert_eq!(diagnostic["cache_source"], "identity_validation");
}
