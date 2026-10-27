use super::*;

use super::super::tests::{
    admit_canonical_references, drain, finish_canonical_references, key, scene, shared, view,
};

fn pipeline_fixture() -> (
    Arc<Shared>,
    mpsc::Receiver<super::super::Job>,
    pack::Scene,
    View,
) {
    let mut current = scene(SceneKey::Opening, "HS61 cadence", Some("Kestrel"));
    current.setting.as_mut().unwrap().model = Some(
        serde_json::from_slice(include_bytes!(
            "../../../../../../content/merchant-shop/scene-model.json"
        ))
        .unwrap(),
    );
    let (shared, mut queue) = shared(16);
    admit_canonical_references(&shared.inner, &current);
    finish_canonical_references(&shared.inner);
    locked(&shared.inner.state)
        .canonical
        .fixture_hero(1, "Hero 1", key('a'));
    let projected = view(&current, &[1], false);
    shared.observe_with(&projected, Some(current.clone()), Vec::new());
    drain(&mut queue);
    {
        let mut book = locked(&shared.book);
        let id = book.current.as_ref().unwrap().id.clone().unwrap();
        book.insert(
            id,
            super::super::Entry::Ready {
                key: key('a'),
                placement: None,
            },
        );
    }
    shared.apply_at(
        &projected.scene_key,
        &mut rpc::GameplayScene::default(),
        Duration::ZERO,
    );
    (shared, queue, current, projected)
}

fn commit_beat(
    shared: &Shared,
    current: &pack::Scene,
    projected: &mut View,
    sequence: u64,
    now: Duration,
    modelled: bool,
) {
    projected.graph_revision = sequence;
    projected.story = Some(StoryBeat {
        actor_id: current.cast[0].id.clone(),
        sequence,
        speaker: current.cast[0].name.clone(),
        text: format!("Recorded vendor moment {sequence}: examining the wares."),
        sha256: sequence.to_string(),
        tempo: shot::PublicBeat::Calm,
        dialogue: shot::PublicDialogue::SameSpeakerFollowup,
    });
    let mut plate = PlateScene::for_view(projected, current).unwrap();
    if !modelled {
        plate.snapshot = None;
    }
    let mut book = locked(&shared.book);
    observe(&mut book, projected, &plate, now);
    let current = book.current.as_mut().unwrap();
    current.scene = Some(plate);
    current.graph_revision = sequence;
}

struct FakeRender {
    scene: String,
    pending: Pending,
    finishes_at: Duration,
}

impl FakeRender {
    fn from_job(job: super::super::Job, finishes_at: Duration) -> Self {
        let super::super::Job::State(Job::Bake { scene, pending, .. }) = job else {
            panic!("expected a plate bake")
        };
        Self {
            scene,
            pending: *pending,
            finishes_at,
        }
    }

    fn complete(self, shared: &Shared, now: Duration) {
        let sequence = locked(&shared.book).state_images.in_flight[&self.pending.state].sequence;
        shared.finish_state_image_at(
            Done::Bake {
                scene: self.scene,
                state: self.pending.state,
                revision: self.pending.revision,
                story_actor: self.pending.story.as_deref().map(StoryBeat::actor_key),
                fresh: true,
                result: Ok(ImageKey::parse(&format!("{sequence:064x}")).unwrap()),
            },
            now,
        );
    }
}

#[test]
fn hs61_plates_pipeline_sixty_to_eighty_second_renders_for_five_minutes() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    let mut renders: Vec<FakeRender> = Vec::new();
    let mut starts = Vec::new();
    let mut presentations = Vec::new();
    let mut last_key = key('a').path();
    let mut maximum_in_flight = 0;
    for second in 0..=300 {
        let now = Duration::from_secs(second);
        if second % 7 == 0 {
            commit_beat(&shared, &current, &mut projected, second / 7 + 1, now, true);
        }
        shared.poll_state_images_at(now);
        for job in drain(&mut queue) {
            let mut render = FakeRender::from_job(job, now);
            let book = locked(&shared.book);
            let composition = &book.state_images.in_flight[&render.pending.state];
            assert_eq!(
                composition.beat,
                second / 7 + 1,
                "compose the beat current at start"
            );
            render.finishes_at = now + Duration::from_secs(60 + (composition.sequence - 1) % 5 * 5);
            starts.push(second);
            renders.push(render);
        }
        maximum_in_flight = maximum_in_flight.max(renders.len());
        assert!(renders.len() <= 3);
        let mut index = 0;
        while index < renders.len() {
            if renders[index].finishes_at <= now {
                renders.remove(index).complete(&shared, now);
            } else {
                index += 1;
            }
        }
        let mut output = rpc::GameplayScene::default();
        shared.apply_at(&projected.scene_key, &mut output, now);
        if output.live_plate != last_key {
            presentations.push(second);
            last_key = output.live_plate;
        }
    }
    assert_eq!(maximum_in_flight, 3);
    assert!(starts.windows(2).all(|pair| pair[1] - pair[0] >= 15));
    assert!(presentations.len() >= 7, "timeline: {presentations:?}");
    assert!(
        presentations
            .windows(2)
            .all(|pair| (25..=35).contains(&(pair[1] - pair[0]))),
        "timeline: {presentations:?}"
    );
    println!(
        "HS61 starts_s={starts:?} presented_s={presentations:?} max_in_flight={maximum_in_flight}"
    );
}

#[test]
fn hs61_plates_next_start_captures_the_new_subject_item_and_mood() {
    use crate::gameplay::scene_mood::{Expression, HealthTier};
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
    shared.poll_state_images_at(Duration::from_secs(3));
    let first = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    let target = {
        let book = locked(&shared.book);
        let model = &book
            .current
            .as_ref()
            .unwrap()
            .scene
            .as_ref()
            .unwrap()
            .snapshot
            .as_ref()
            .unwrap()
            .model;
        model
            .things
            .iter()
            .find(|thing| thing.in_scene && thing.publicly_visible)
            .unwrap()
            .id
            .clone()
    };
    locked(&shared.book).state_images.subject_selection = Some(subject::Selection {
        scene: projected.scene_key.clone(),
        hero: "Hero 1".into(),
        target: target.clone(),
    });
    let angry = Face {
        expression: Expression::Angry,
        tier: HealthTier::Healthy,
    };
    locked(&shared.inner.state)
        .plate_wants
        .insert(current.cast[0].id.clone(), angry);
    commit_beat(
        &shared,
        &current,
        &mut projected,
        2,
        Duration::from_secs(4),
        true,
    );
    shared.poll_state_images_at(Duration::from_secs(17));
    assert!(
        drain(&mut queue).is_empty(),
        "all starts respect the fifteen-second gate"
    );
    shared.poll_state_images_at(Duration::from_secs(18));
    let super::super::Job::State(Job::Bake {
        pending,
        identities,
        ..
    }) = drain(&mut queue).remove(0)
    else {
        panic!("next composition")
    };
    assert_eq!(first.pending.story.as_ref().unwrap().sequence, 1);
    assert_eq!(pending.story.as_ref().unwrap().sequence, 2);
    assert!(pending.framed_subject.is_some());
    assert_eq!(pending.focus.as_ref().unwrap().subject.id(), target);
    let identity = identities.unwrap();
    assert_eq!(identity.face, Some(angry));
    assert_eq!(
        identity.scene.story_text,
        projected.story.as_ref().unwrap().text
    );
    assert_eq!(
        identity.scene.speaker.as_ref().unwrap().id,
        projected.story.as_ref().unwrap().actor_id
    );
    locked(&shared.inner.state).plate_wants.clear();
    assert_eq!(
        identity.face,
        Some(angry),
        "later mood changes cannot mutate admitted work"
    );
    assert_eq!(locked(&shared.book).state_images.scene_in_flight(), 2);
}

#[test]
fn hs61_plates_new_beats_do_not_cancel_renders_and_out_of_order_never_regresses() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    let mut renders = Vec::new();
    for (sequence, second) in [(1, 0), (2, 15), (3, 30)] {
        commit_beat(
            &shared,
            &current,
            &mut projected,
            sequence,
            Duration::from_secs(second),
            false,
        );
        shared.poll_state_images_at(Duration::from_secs(second + 3));
        let render = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
        assert_eq!(render.pending.story.as_ref().unwrap().sequence, sequence);
        renders.push(render);
    }
    shared.poll_state_images_at(Duration::from_secs(63));
    assert!(
        drain(&mut queue).is_empty(),
        "three admitted renders bound the pipeline"
    );
    // Second render takes 60 seconds, first takes 80: the newer composition wins.
    renders.remove(1).complete(&shared, Duration::from_secs(78));
    let mut output = rpc::GameplayScene::default();
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(78));
    let newer = output.live_plate.clone();
    renders.remove(0).complete(&shared, Duration::from_secs(83));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(83));
    assert_eq!(
        output.live_plate, newer,
        "old completion cannot step backwards"
    );
    renders.remove(0).complete(&shared, Duration::from_secs(93));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(102));
    assert_eq!(
        output.live_plate, newer,
        "hold a finished plate for reading time"
    );
    shared.poll_state_images_at(Duration::from_secs(108));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(108));
    assert_ne!(
        output.live_plate, newer,
        "deliver waiting newer work within 35 seconds"
    );
    assert_eq!(locked(&shared.book).state_images.shown_sequence, 3);
}

#[test]
fn hs61_plates_scene_exit_retires_work_even_after_returning_to_the_same_scene() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, false);
    shared.poll_state_images_at(Duration::from_secs(3));
    let old = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    let original = projected.clone();
    projected.scene_key = "another-scene".into();
    {
        let mut book = locked(&shared.book);
        reset_scene(&mut book, &projected, Duration::from_secs(10));
        assert!(
            book.state_images
                .in_flight
                .values()
                .all(|composition| composition.retired)
        );
        reset_scene(&mut book, &original, Duration::from_secs(20));
    }
    projected = original;
    commit_beat(
        &shared,
        &current,
        &mut projected,
        2,
        Duration::from_secs(20),
        false,
    );
    shared.poll_state_images_at(Duration::from_secs(23));
    let newer = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    old.complete(&shared, Duration::from_secs(83));
    {
        let book = locked(&shared.book);
        assert!(book.state_images.shown.is_none());
        assert!(book.state_images.ready.is_none());
        assert!(
            book.state_images.has_in_flight(),
            "an old completion cannot release newer work"
        );
    }
    newer.complete(&shared, Duration::from_secs(84));
    assert_eq!(locked(&shared.book).state_images.shown_sequence, 2);
}

#[test]
fn hs65_local_spot_keeps_the_presented_plate_until_its_replacement_is_ready() {
    let (shared, mut queue, current, projected) = pipeline_fixture();
    let mut spot = current.clone();
    spot.title = "Oren's stall".into();
    spot.setting.as_mut().unwrap().short = "Oren's stall in the same courtyard".into();
    let at_stall = view(&spot, &[1], false);
    assert_ne!(projected.scene_key, at_stall.scene_key);
    shared.observe_with(&at_stall, Some(spot.clone()), Vec::new());
    drain(&mut queue);
    let mut output = rpc::GameplayScene::default();
    shared.apply_at(&at_stall.scene_key, &mut output, Duration::from_secs(10));
    assert_eq!(output.live_plate, key('a').path());
    assert_eq!(output.plate_state, rpc::PlateState::Ready as i32);
    assert_eq!(
        locked(&shared.book).presented.as_ref().unwrap().since,
        Duration::ZERO
    );

    spot.title = "Another place".into();
    spot.setting.as_mut().unwrap().place_id = "different-canonical-place".into();
    let elsewhere = view(&spot, &[1], false);
    shared.observe_with(&elsewhere, Some(spot), Vec::new());
    let mut output = rpc::GameplayScene::default();
    shared.apply_at(&elsewhere.scene_key, &mut output, Duration::from_secs(11));
    assert_ne!(
        output.live_plate,
        key('a').path(),
        "a real place change cannot hold the old picture"
    );
}

#[test]
fn hs65_local_spot_keeps_in_flight_cadence_and_newer_stall_work_supersedes_it() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, false);
    shared.poll_state_images_at(Duration::from_secs(3));
    let old = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    let mut spot = current.clone();
    spot.title = "Oren's stall".into();
    let at_stall = view(&spot, &[1], false);
    shared.observe_with(&at_stall, Some(spot.clone()), Vec::new());
    drain(&mut queue);
    {
        let book = locked(&shared.book);
        assert_eq!(book.state_images.scene_in_flight(), 1);
        assert!(
            book.state_images
                .in_flight
                .values()
                .all(|work| !work.retired)
        );
    }
    old.complete(&shared, Duration::from_secs(31));
    let mut output = rpc::GameplayScene::default();
    shared.apply_at(&at_stall.scene_key, &mut output, Duration::from_secs(31));
    assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 1));
    projected = at_stall;
    commit_beat(
        &shared,
        &spot,
        &mut projected,
        2,
        Duration::from_secs(33),
        false,
    );
    shared.poll_state_images_at(Duration::from_secs(36));
    let newer = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    newer.complete(&shared, Duration::from_secs(62));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(62));
    assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 2));
}

#[tokio::test]
async fn hs65_committed_ware_is_foreground_and_the_next_shot_then_returns_to_the_scene() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let (mut shared, mut queue, current, mut projected) = pipeline_fixture();
    let cache = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/tmp/lanes/HS71/item-shot")
        .join(std::process::id().to_string());
    let fixture = DirectionFixture::photographic(&cache);
    Arc::get_mut(&mut Arc::get_mut(&mut shared).unwrap().inner)
        .unwrap()
        .imagery = fixture.imagery.clone();
    let model = current.setting.as_ref().unwrap().model.as_ref().unwrap();
    let before = model.public_contents().unwrap();
    // Offer-only stock is projected for inspection, never materialized into inventory.
    let offer = &model.offers[0];
    let mut ware = model.things[0].clone();
    ware.id = offer.item_id.clone();
    ware.name = offer.name.clone();
    ware.detail = offer.description.clone();
    ware.picture_description = offer.description.clone();
    ware.owner_id = Some(offer.seller_id.clone());
    ware.holder_id = Some(offer.seller_id.clone());
    assert!(model.things.iter().all(|thing| thing.id != ware.id));
    // Unlike the old object-only card, a focus shot reads the real current plate bytes.
    let base = fixture
        .imagery
        .generate_with_priority(
            "fixture-scene",
            item_image_request(shared.inner.rating, &ware, None).unwrap(),
            ImagePriority::Foreground,
        )
        .await
        .unwrap();
    {
        let mut book = locked(&shared.book);
        let id = book.current.as_ref().unwrap().id.clone().unwrap();
        book.insert(
            id,
            super::super::Entry::Ready {
                key: base.clone(),
                placement: None,
            },
        );
        let shown = book.presented.as_mut().unwrap();
        shown.key = base.clone();
        shown.since = Duration::ZERO;
    }
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
    shared.poll_state_images_at(Duration::from_secs(3));
    let cadence = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    let plates = Plates {
        shared: Arc::clone(&shared),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    plates
        .request_committed_item(
            "Brynn",
            &ware,
            Some("The folded linen has a blue hem.".into()),
            true,
        )
        .unwrap();
    let super::super::Job::State(job) = drain(&mut queue).remove(0) else {
        panic!("item job")
    };
    assert!(matches!(
        &job,
        Job::Item {
            priority: ImagePriority::Foreground,
            ..
        }
    ));
    let done = shared.render_state_image(job).await;
    let Done::Item {
        result: Ok(item_key),
        ..
    } = &done
    else {
        panic!("fake artifact_close_up completed")
    };
    let item_key = item_key.clone();
    assert_eq!(
        fixture.calls(),
        3,
        "current plate, canonical item, then one scene-focus edit"
    );
    assert!(fixture.prompt().contains(&ware.name));
    assert_eq!(
        fixture.image_count(),
        2,
        "scene and canonical item references"
    );
    assert!(fixture.prompt().contains("centre of the frame"));
    assert!(fixture.prompt().contains("softly out of focus"));
    assert!(!fixture.prompt().contains(&ware.closer_look));
    shared.finish_state_image_at(done, Duration::from_secs(5));
    cadence.complete(&shared, Duration::from_secs(20));
    let mut output = rpc::GameplayScene::default();
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(24));
    assert_eq!(
        output.live_plate,
        base.path(),
        "keep the scene's reading hold"
    );
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(25));
    assert_eq!(
        output.live_plate,
        item_key.path(),
        "the item precedes the ready cadence picture"
    );
    assert!(
        output.plate_layout.is_none(),
        "never lend the item the scene's people"
    );
    assert_eq!(
        locked(&shared.book)
            .state_images
            .item_shot
            .as_ref()
            .unwrap()
            .hero,
        "Brynn"
    );
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(49));
    assert_eq!(output.live_plate, item_key.path());
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(50));
    assert_eq!(output.live_plate, base.path());
    assert!(locked(&shared.book).state_images.item_shot.is_none());
    shared.poll_state_images_at(Duration::from_secs(80));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(81));
    assert_eq!(output.live_plate, format!("/gameplay/imagery/{:064x}", 1));
    assert_eq!(model.public_contents().unwrap(), before);
    assert_eq!(
        locked(&shared.book)
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
            .unwrap(),
        before
    );
}

#[tokio::test]
async fn hs58_inventory_waits_for_a_plate_and_does_not_fill_the_plate_queue() {
    use super::super::tests::{drain, key, shared};
    let (shared, mut queue) = shared(16);
    let plates = Plates {
        shared: Arc::clone(&shared),
        _worker: Arc::new(WorkerGuard(tokio::spawn(async {}))),
    };
    let card = |index| rpc::SceneItemCard {
        thing_id: format!("equipment-{index}"),
        name: "chain mail".into(),
        detail: "Iron chain mail".into(),
        picture_description: "Linked iron rings".into(),
        holder: "hero".into(),
        ..Default::default()
    };
    plates
        .request_hand_item_with_priority(&card(0), false, "{}", ImagePriority::Background)
        .unwrap();
    assert!(drain(&mut queue).is_empty());
    assert!(locked(&shared.book).state_images.items.is_empty());
    locked(&shared.book).state_images.shown = Some((key('a'), None));
    for index in 0..120 {
        plates
            .request_hand_item_with_priority(&card(index), false, "{}", ImagePriority::Background)
            .unwrap();
    }
    assert_eq!(
        drain(&mut queue).len(),
        crate::gameplay::imagery::BACKGROUND_IN_FLIGHT
    );
    let first = locked(&shared.book)
        .state_images
        .items
        .keys()
        .next()
        .unwrap()
        .clone();
    shared.finish_state_image(Done::Item {
        id: first,
        result: Ok(key('b')),
    });
    plates
        .request_hand_item_with_priority(&card(119), false, "{}", ImagePriority::Background)
        .unwrap();
    assert_eq!(
        drain(&mut queue).len(),
        1,
        "later projections admit deferred cards as capacity returns"
    );
}

#[test]
fn hs41_plates_player_text_cannot_be_mistaken_for_a_cadence_completion_marker() {
    let book = Book {
        observed: "canonical player text |cadence_context=ordinary words".into(),
        ..Default::default()
    };
    assert!(book.accepts_state(&book.observed));
    assert!(book.accepts_state(&format!("{}|cadence_context=none", book.observed)));
    assert!(!book.accepts_state("other state|cadence_context=none"));
}

#[test]
fn a_fresh_conversation_plate_survives_an_older_state_render_finishing_late() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
    shared.poll_state_images_at(Duration::from_secs(3));
    let older = FakeRender::from_job(drain(&mut queue).remove(0), Duration::ZERO);
    let id = {
        let mut book = locked(&shared.book);
        let current = book.current.as_mut().unwrap();
        current.graph_revision = 2;
        current.id.clone().unwrap()
    };
    shared.finish(super::super::Done::Fresh {
        id,
        graph_revision: 2,
        ahead: false,
        elapsed: Duration::from_secs(1),
        result: Ok((key('b'), None)),
    });
    let mut output = rpc::GameplayScene::default();
    assert_eq!(
        shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(30)),
        Some(2)
    );
    assert_eq!(output.live_plate, key('b').path());
    older.complete(&shared, Duration::from_secs(65));
    shared.poll_state_images_at(Duration::from_secs(65));
    shared.apply_at(&projected.scene_key, &mut output, Duration::from_secs(65));
    assert_eq!(output.live_plate, key('b').path());
}

#[test]
fn a_person_subject_in_an_opening_scene_uses_a_conversation_camera() {
    let (shared, mut queue, current, mut projected) = pipeline_fixture();
    locked(&shared.book).state_images.subject_selection = Some(subject::Selection {
        scene: projected.scene_key.clone(),
        hero: "Hero 1".into(),
        target: current.cast[0].id.clone(),
    });
    commit_beat(&shared, &current, &mut projected, 1, Duration::ZERO, true);
    shared.poll_state_images_at(Duration::from_secs(3));
    let super::super::Job::State(Job::Bake {
        pending,
        identities,
        ..
    }) = drain(&mut queue).remove(0)
    else {
        panic!("person-subject composition")
    };
    assert!(pending.framed_subject.is_some());
    assert_eq!(
        story_camera(&pending, identities.as_deref().unwrap()),
        Some(shot::Camera::DialogueMedium)
    );
}
