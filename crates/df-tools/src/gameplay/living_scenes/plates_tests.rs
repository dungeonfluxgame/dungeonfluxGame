use std::path::PathBuf;

use tokio::time::Instant;

use super::{
    super::{
        super::{
            adventure::setting::{SceneSetting, SettingKind},
            imagery::{ImageKey, Imagery},
            scene_mood::{Appearance, Expression},
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
        }),
        speaker_portrait: String::new(),
    }
}

fn view(scene: &pack::Scene, heroes: &[u8], in_combat: bool) -> View {
    View {
        scene_key: format!("{}|{}", scene.title, scene.asset),
        title: scene.title.clone(),
        asset: scene.asset.clone(),
        in_combat,
        heroes: heroes.to_vec(),
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

fn observe(shared: &Shared, current: &pack::Scene, next: &[pack::Scene], heroes: &[u8]) {
    shared.observe_with(
        &view(current, heroes, false),
        Some(current.clone()),
        next.to_vec(),
    );
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
            Job::Edit { .. } => None,
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
        elapsed: Duration::from_secs(14),
        result: Ok((key(plate), Some(layout()))),
    });
}

fn layout() -> PlateLayout {
    shot::direct(&ShotInput {
        scene_key: SceneKey::Opening,
        place_id: "p",
        place_kind: "bell-chamber",
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
        assert_eq!(layout.head_y_permille, 320);
    }
    assert!(locked(&shared.inner.state).plate_covers);
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
        appearance: Appearance::Unhurt,
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
        appearance: Appearance::Unhurt,
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
async fn a_creature_speaker_means_no_plate_and_no_failure() {
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
    assert_eq!(shown_scene(&message).plate_state, 0);
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
    assert_eq!(view_of(&message).unwrap().heroes, [1, 3]);
    assert!(view_of(&player_message(&scene(SceneKey::Opening, "T", None))).is_none());
}
