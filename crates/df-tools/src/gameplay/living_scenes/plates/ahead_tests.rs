use super::super::tests::{
    admit_canonical_references, drain, finish_canonical_references, key, observe, scene, shared,
    view,
};
use super::*;

fn complete(shared: &Shared, id: String) {
    shared.finish(Done::Fresh {
        id,
        graph_revision: 0,
        ahead: true,
        elapsed: Duration::from_secs(5),
        result: Ok((key('b'), None)),
    });
}

#[tokio::test]
async fn travel_plate_is_ready_before_arrival_and_reports_a_hit() {
    let (shared, mut jobs) = shared(16);
    let current = scene(SceneKey::Opening, "Departure", Some("Kestrel"));
    let destination = scene(
        SceneKey::Opening,
        "Planned travel destination",
        Some("Kestrel"),
    );
    observe(&shared, &current, &[], &[1]);
    drain(&mut jobs);
    {
        let mut book = locked(&shared.book);
        shared.queue_ahead(
            &mut book,
            PlateScene::of(&destination).unwrap(),
            Source::Travel,
            &[1],
            &[(1, "Brynn".into())],
            0,
        );
        shared.poll_ahead_locked(&mut book);
    }
    let Job::Fresh {
        id, ahead, scene, ..
    } = drain(&mut jobs).remove(0)
    else {
        panic!("travel plate queued");
    };
    assert!(ahead);
    assert_eq!(scene.title, destination.title);
    {
        let mut book = locked(&shared.book);
        let now = book.ahead.candidates[0].queued + Duration::from_secs(7);
        shared.finish_ahead(&mut book, &id, true, now);
    }
    complete(&shared, id.clone());
    assert_eq!(
        locked(&shared.book).ahead.candidates[0].ready_ms,
        Some(7000)
    );
    shared.observe_with(
        &view(&destination, &[1], false),
        Some(destination),
        Vec::new(),
    );
    let book = locked(&shared.book);
    assert!(matches!(book.current_entry(), Some(Entry::Ready { .. })));
    assert_eq!(book.ahead.candidates[0].used, Some("hit"));
    assert!(book.ahead.arrival.as_ref().unwrap().ready);
    assert!(
        drain(&mut jobs).is_empty(),
        "arrival reuses the prefetched picture"
    );
}

#[tokio::test]
async fn late_plate_reports_miss_and_measures_the_arrival_wait() {
    let (shared, mut jobs) = shared(16);
    let current = scene(SceneKey::Opening, "Late plate", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    let Job::Fresh { id, .. } = drain(&mut jobs).remove(0) else {
        panic!("current plate");
    };
    assert!(!locked(&shared.book).ahead.arrival.as_ref().unwrap().ready);
    let now = locked(&shared.book).ahead.arrival.as_ref().unwrap().started + Duration::from_secs(9);
    locked(&shared.book).entries.insert(
        id.clone(),
        Entry::Ready {
            key: key('b'),
            placement: None,
        },
    );
    shared.note_arrival_ready(&mut locked(&shared.book), &id, now);
    let book = locked(&shared.book);
    let arrival = book.ahead.arrival.as_ref().unwrap();
    assert!(arrival.ready);
    let (line, fields) = arrival_event(
        &arrival.scene,
        "miss",
        now.duration_since(arrival.started).as_millis(),
        "ready",
    );
    assert!(line.contains("used=miss wait_ms=9000 status=ready"));
    assert_eq!(fields["wait_ms"], 9000);
    assert_eq!(fields["used"], "miss");
    let (line, fields) = arrival_event(&arrival.scene, "hit", 0, "ready");
    assert!(line.contains("used=hit wait_ms=0"));
    assert_eq!(fields["used"], "hit");
}

#[tokio::test]
async fn abandoned_branch_cannot_render_or_publish_its_late_completion() {
    let (shared, mut jobs) = shared(16);
    let current = scene(SceneKey::Opening, "Departure", Some("Kestrel"));
    let branch = scene(SceneKey::DialogueSeal, "Abandoned branch", Some("Kestrel"));
    let elsewhere = scene(SceneKey::DialogueEscort, "Elsewhere", Some("Kestrel"));
    observe(&shared, &current, &[branch], &[1]);
    let (id, job) = drain(&mut jobs)
        .into_iter()
        .find_map(|job| {
            let Job::Fresh {
                id, ahead: true, ..
            } = &job
            else {
                return None;
            };
            Some((id.clone(), job))
        })
        .unwrap();
    assert!(shared.ahead_wanted(&id));
    shared.observe_with(&view(&elsewhere, &[1], false), Some(elsewhere), Vec::new());
    assert!(!shared.ahead_wanted(&id));
    let mut tasks = JoinSet::new();
    shared.spawn(&mut tasks, job);
    let done = tokio::time::timeout(Duration::from_secs(1), tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(done, Done::Cancelled),
        "stale work never reaches the renderer"
    );
    shared.finish(done);
    complete(&shared, id.clone());
    assert!(!locked(&shared.book).entries.contains_key(&id));
}

#[tokio::test]
async fn speculative_queue_is_bounded_and_concrete_plans_replace_static_predictions() {
    let (shared, mut jobs) = shared(16);
    let current = scene(SceneKey::Opening, "Departure", Some("Kestrel"));
    observe(&shared, &current, &[], &[1]);
    drain(&mut jobs);
    let mut book = locked(&shared.book);
    for index in 0..20 {
        let candidate = scene(
            SceneKey::Opening,
            &format!("Branch {index}"),
            Some("Kestrel"),
        );
        shared.queue_ahead(
            &mut book,
            PlateScene::of(&candidate).unwrap(),
            Source::Beat,
            &[1],
            &[],
            0,
        );
    }
    assert_eq!(book.ahead.candidates.len(), MAXIMUM_SUCCESSORS);
    let made = scene(SceneKey::Opening, "Written place", Some("Kestrel"));
    shared.queue_ahead(
        &mut book,
        PlateScene::of(&made).unwrap(),
        Source::Generated,
        &[1],
        &[],
        0,
    );
    shared.poll_ahead_locked(&mut book);
    assert_eq!(book.ahead.candidates.len(), MAXIMUM_SUCCESSORS);
    assert!(
        book.ahead
            .candidates
            .iter()
            .any(|candidate| candidate.source == Source::Generated)
    );
    assert_eq!(drain(&mut jobs).len(), MAXIMUM_SUCCESSORS);
}

#[tokio::test]
async fn lobby_warms_accepted_heroes_and_room_before_the_opening_plate() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../../../content/harbor-journey/pack.json"),
        include_bytes!("../../../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let (shared, mut jobs) = shared(16);
        let checkpoint = crate::gameplay::beat_script_tests::prepared_story();
        assert_eq!(
            journey::phase(&checkpoint).unwrap(),
            rpc::JourneyPhase::Room
        );
        assert!(journey::party_ready(&checkpoint));
        shared.plan_checkpoint(&checkpoint);
        assert_eq!(
            shared.inner.canonical_hero_status(1),
            super::super::super::canonical::ReferenceStatus::Pending
        );
        assert_eq!(
            shared.inner.canonical_hero_status(2),
            super::super::super::canonical::ReferenceStatus::Pending
        );
        assert!(
            drain(&mut jobs).is_empty(),
            "plate waits for its real identities and room"
        );
        let candidate = locked(&shared.book).ahead.candidates[0].scene.clone();
        assert_eq!(candidate.key, SceneKey::Opening);
        if let Some(revision) = &candidate.room_revision {
            assert_eq!(
                shared
                    .inner
                    .canonical_room_status_at(&candidate.place_id, revision),
                super::super::super::canonical::ReferenceStatus::Pending
            );
        }
        finish_canonical_references(&shared.inner);
        shared.poll_ahead_locked(&mut locked(&shared.book));
        assert!(
            matches!(drain(&mut jobs).first(), Some(Job::Fresh { scene, ahead: true, .. }) if scene.key == SceneKey::Opening)
        );
    });
}

#[tokio::test]
async fn ahead_candidate_survives_pending_canonical_references_and_retries_without_a_view() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../../../content/harbor-journey/pack.json"),
        include_bytes!("../../../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let (shared, mut jobs) = shared(16);
        let checkpoint = crate::gameplay::beat_script_tests::prepared_story();
        shared.inner.observe_checkpoint(&checkpoint);
        let planned = pack::scene(SceneKey::Opening);
        admit_canonical_references(&shared.inner, &planned);
        let mut book = locked(&shared.book);
        shared.queue_ahead(
            &mut book,
            PlateScene::of(&planned).unwrap(),
            Source::Travel,
            &[1, 2],
            &[],
            0,
        );
        shared.poll_ahead_locked(&mut book);
        assert!(drain(&mut jobs).is_empty());
        drop(book);
        finish_canonical_references(&shared.inner);
        shared.poll_ahead_locked(&mut locked(&shared.book));
        assert!(
            drain(&mut jobs)
                .iter()
                .any(|job| matches!(job, Job::Fresh { ahead: true, .. }))
        );
    });
}

#[tokio::test]
async fn offered_known_travel_is_primed_from_the_committed_place_journal() {
    use crate::gameplay::{
        adventure::{RulesMode, record, shop},
        beat_script_variety_tests as native, place_creation,
        startup_configuration::ContentRating,
    };
    let generated = shop::generate(42, RulesMode::Standard, ContentRating::default()).unwrap();
    let (loaded, _) = record::reload(&generated.documents(true).unwrap()).unwrap();
    pack::with_test_pack(loaded, || {
        let opening = native::act(&native::prepared(), native::first(), 5, "begin-story");
        let base = beat_script_runtime::scene(opening.state()).unwrap();
        let home = base.setting.as_ref().unwrap().place_id.clone();
        let model = base.setting.as_ref().unwrap().model.as_ref().unwrap();
        let mut request = place_creation::tests::request();
        request.time_of_day = model.time_of_day.clone();
        request.weather = model.weather.clone();
        request.origin_name = base.setting.as_ref().unwrap().name.clone();
        let mut place = place_creation::tests::place_slot();
        place["name"] = serde_json::json!("TESTSC Known Travel Street");
        let street = place_creation::build(
            &request,
            &place.to_string(),
            &place_creation::tests::people_slot().to_string(),
            &place_creation::tests::things_slot().to_string(),
        )
        .unwrap();
        let operation = |number| df_types::OperationId::from_bytes(&[number; 16]).unwrap();
        let moved = place::plan(
            &opening,
            "Brynn",
            operation(40),
            &home,
            "street-door",
            &street.place_id,
            Some(&street),
        )
        .unwrap_or_else(|failure| match failure {
            place::PlaceFailure::Refused(reason) => {
                panic!("fixture move refused: {}", reason.label())
            }
            place::PlaceFailure::State(error) => panic!("fixture move state: {error:?}"),
        })
        .candidate;
        let returned = place::plan(
            &moved,
            "Brynn",
            operation(41),
            &street.place_id,
            place::WAY_BACK,
            &home,
            None,
        )
        .unwrap_or_else(|failure| match failure {
            place::PlaceFailure::Refused(reason) => {
                panic!("fixture move refused: {}", reason.label())
            }
            place::PlaceFailure::State(error) => panic!("fixture move state: {error:?}"),
        })
        .candidate;
        assert!(place_move::travel_options(&returned).unwrap().iter().any(|option| matches!(&option.target, place_move::MoveTarget::Exit(id) if id == "street-door")));
        let (shared, mut jobs) = shared(16);
        shared.plan_checkpoint(&returned);
        assert!(
            locked(&shared.book)
                .ahead
                .candidates
                .iter()
                .any(|candidate| candidate.source == Source::Travel
                    && candidate.scene.title == street.name)
        );
        finish_canonical_references(&shared.inner);
        shared.poll_ahead_locked(&mut locked(&shared.book));
        let id = drain(&mut jobs)
            .into_iter()
            .find_map(|job| match job {
                Job::Fresh {
                    id,
                    scene,
                    ahead: true,
                    ..
                } if scene.title == street.name => Some(id),
                _ => None,
            })
            .expect("offered destination's plate is queued while still at home");
        complete(&shared, id);
        let arrived = place::scene_of(&street, &base).unwrap();
        shared.observe_with(&view(&arrived, &[1, 2], false), Some(arrived), Vec::new());
        assert!(matches!(
            locked(&shared.book).current_entry(),
            Some(Entry::Ready { .. })
        ));
        assert_eq!(
            locked(&shared.book)
                .ahead
                .candidates
                .iter()
                .find(|candidate| candidate.scene.title == street.name)
                .unwrap()
                .used,
            Some("hit")
        );
    });
}

#[test]
fn committed_beat_successors_survive_a_generated_place_overlay() {
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../../../content/harbor-journey/pack.json"),
        include_bytes!("../../../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let mut overlay = pack::scene(SceneKey::Opening);
        overlay.title = "A generated place beyond the wharf".into();
        assert!(successors(&overlay).is_empty());
        pack::with_content(|content| {
            let script = beat_script::canonical();
            let beat = script
                .beats
                .iter()
                .find(|beat| beat.scene == SceneKey::Opening)
                .unwrap();
            let next = successors_in_from(script, &content.scenes, &overlay, Some(&beat.id));
            assert!(next.iter().any(|scene| scene.key == SceneKey::DialogueSeal));
            assert!(
                next.iter()
                    .any(|scene| scene.key == SceneKey::DialogueEscort)
            );
        });
    });
}

#[tokio::test]
async fn confirmed_hero_warms_before_the_rest_of_the_lobby_is_ready() {
    use crate::gameplay::beat_script_tests::{build, input};
    let loaded = pack::load_from_bytes(
        include_bytes!("../../../../../../content/harbor-journey/pack.json"),
        include_bytes!("../../../../../../content/harbor-journey/tuning.json"),
    )
    .unwrap();
    pack::with_test_pack(loaded, || {
        let mut current = journey::initial().unwrap();
        for operation in [1, 2] {
            let request = input(
                &current,
                journey::bootstrap_member().unwrap(),
                operation,
                "join-room",
                vec![],
            );
            current = journey::stage_join(&current, &request).unwrap();
        }
        let member = df_types::MemberId::from_bytes(&[0x61; 16]).unwrap();
        let request = input(&current, member, 3, "create-character", build("Brynn"));
        current = journey::stage_with_supplier(&current, &request, &mut |_| {
            panic!("creation cannot draw")
        })
        .unwrap();
        assert!(!journey::party_ready(&current));
        let (shared, mut jobs) = shared(16);
        shared.plan_checkpoint(&current);
        assert_eq!(
            shared.inner.canonical_hero_status(1),
            super::super::super::canonical::ReferenceStatus::Pending
        );
        assert_eq!(
            shared.inner.canonical_hero_status(2),
            super::super::super::canonical::ReferenceStatus::Missing
        );
        assert!(locked(&shared.book).ahead.candidates.is_empty());
        assert!(drain(&mut jobs).is_empty());
        assert_eq!(locked(&shared.inner.state).canonical.take_jobs().len(), 1);
    });
}

#[tokio::test]
async fn changed_item_positions_invalidate_a_prefetched_plate_before_arrival() {
    use crate::gameplay::adventure::scene_model::{self, SceneModel, SceneNow};
    let (shared, mut jobs) = shared(16);
    let current = scene(SceneKey::Opening, "Departure", Some("Kestrel"));
    let mut branch = scene(
        SceneKey::DialogueSeal,
        "Changed destination",
        Some("Kestrel"),
    );
    let model: SceneModel = serde_json::from_slice(include_bytes!(
        "../../../../../../content/merchant-shop/scene-model.json"
    ))
    .unwrap();
    let setting = branch.setting.as_mut().unwrap();
    setting.place_id = "hs32-stale-item-destination".into();
    setting.model = Some(model.clone());
    observe(&shared, &current, &[branch.clone()], &[1]);
    let (id, job) = drain(&mut jobs)
        .into_iter()
        .find_map(|job| {
            let Job::Fresh {
                id, ahead: true, ..
            } = &job
            else {
                return None;
            };
            Some((id.clone(), job))
        })
        .unwrap();
    complete(&shared, id.clone());
    let mut moved = model;
    moved.things[0].position = "on the far window ledge".into();
    scene_model::publish_scene(
        "hs32-stale-item-destination",
        SceneNow {
            model: moved,
            changes: vec![],
        },
    );
    shared.observe_with(&view(&branch, &[1], false), Some(branch), Vec::new());
    assert!(
        matches!(locked(&shared.book).current_entry(), Some(Entry::Pending)),
        "the obsolete picture cannot count as a ready arrival"
    );
    let current_jobs = drain(&mut jobs);
    assert!(current_jobs.iter().any(|job| matches!(job, Job::Fresh { scene, ahead: false, .. } if scene.snapshot.as_ref().unwrap().model.things[0].position == "on the far window ledge")));
    let mut tasks = JoinSet::new();
    shared.spawn(&mut tasks, job);
    let done = tokio::time::timeout(Duration::from_secs(1), tasks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(done, Done::Cancelled),
        "an old completion cannot overwrite the new request with the same plate id"
    );
    shared.finish(done);
    assert!(matches!(
        locked(&shared.book).current_entry(),
        Some(Entry::Pending)
    ));
}

#[tokio::test]
async fn run_scope_change_clears_previous_presentation_state() {
    let (shared, _) = shared(16);
    {
        let mut book = locked(&shared.book);
        book.ahead.scope = "previous session/run".into();
        book.direct.shown = Some((key('a'), None));
        book.state_images.shown = Some((key('b'), None));
        book.insert(
            "old plate".into(),
            Entry::Ready {
                key: key('a'),
                placement: None,
            },
        );
    }
    shared.plan_checkpoint(&journey::initial().unwrap());
    let book = locked(&shared.book);
    assert!(book.current.is_none());
    assert!(book.direct.shown.is_none());
    assert!(book.state_images.shown.is_none());
    assert!(book.entries.is_empty());
    assert!(matches!(book.edit, Edit::Idle));
}
