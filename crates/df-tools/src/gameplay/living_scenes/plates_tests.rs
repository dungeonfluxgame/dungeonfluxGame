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

fn view(scene: &pack::Scene, heroes: &[u8], in_combat: bool) -> View {
    View {
        scene_key: format!("{}|{}", scene.title, scene.asset),
        title: scene.title.clone(),
        asset: scene.asset.clone(),
        in_combat,
        in_roll: false,
        graph_revision: 1,
        heroes: heroes.to_vec(),
        hero_names: heroes
            .iter()
            .map(|art| (*art, format!("Hero {art}")))
            .collect(),
    }
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
    let (jobs, _queue) = mpsc::channel(16);
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
    shared.render_edit(&scene, &edit_base, angry).await.unwrap();
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
    let scene_key = format!("{}|{}", current.title, current.asset);

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
    let id = locked(&shared.book)
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
        .id
        .clone();
    plates.request_item("Brakka", &id, None, false).unwrap();
    plates.request_item("Brakka", &id, None, false).unwrap();
    assert_eq!(drain(&mut queue).len(), 1);
    let mut private = rpc::GameplayScene {
        title: scene.title.clone(),
        scene_asset: scene.asset.clone(),
        ..Default::default()
    };
    shared.project_scene_contents(&mut private, Some("Brakka"));
    assert_eq!(private.item.as_ref().unwrap().thing_id, id);
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
        .render_fresh(&scene, &spec, &heroes, &faces)
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
async fn a_modelled_room_keeps_its_plate_when_only_the_speakers_expression_changes() {
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
    shared.poll_face();
    assert!(drain(&mut queue).is_empty());
    assert_eq!(locked(&shared.book).shown().unwrap().0, key('a'));
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
    shot::direct(&ShotInput {
        scene_key: SceneKey::Opening,
        place_id: "p",
        place_kind: "bell-chamber",
        exterior: false,
        title: "t",
        story_text: "",
        speaker: Some(SpeakerInput {
            sprite: "sprite-patron-bell-ringer",
            gender: None,
            age: None,
        }),
        others: &[],
        heroes: &[1],
    })
    .unwrap()
    .speaker
    .unwrap()
    .layout
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
    shared.finish(Done::Edit {
        scene_key: "Word Before Dawn|assets/ui/generated/scene-place-bell-chamber.webp".to_owned(),
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
        "the edited plate replaces the first one, with the same layout"
    );
    assert_eq!(
        shown_scene(&message)
            .plate_layout
            .as_ref()
            .unwrap()
            .head_x_permille,
        640
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
        scene_key: "Word Before Dawn|assets/ui/generated/scene-place-bell-chamber.webp".to_owned(),
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
    let (jobs, mut queue) = mpsc::channel(16);
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
