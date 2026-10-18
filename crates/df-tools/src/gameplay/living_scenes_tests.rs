use std::path::PathBuf;

use super::{
    super::{
        adventure::dossier::{Attitude, Dossier, Effect},
        imagery::{ImageKey, Imagery},
        npc_dossier_tests::{adventure, member as hero},
        scene_mood::{Expression, Mood},
        social::Approach,
        startup_configuration::ImageryOff,
    },
    *,
};

fn seconds(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn timing() -> Timing {
    Timing::from_interval(seconds(30))
}

fn key(byte: char) -> ImageKey {
    ImageKey::parse(&byte.to_string().repeat(64)).unwrap()
}

#[test]
fn image_completion_reports_elapsed_milliseconds_for_ready_and_failed_images_without_prompts() {
    let ready = image_completion_line("scene_plate", Duration::from_millis(123), Ok(&key('a')));
    assert!(ready.starts_with("IMAGERY ready kind=scene_plate id="));
    assert!(ready.contains("ms=123 status=ready"));
    let failed = image_completion_line("item", Duration::from_millis(456), Err(&ImageFailure::Off));
    assert!(failed.starts_with("IMAGERY failed kind=item id=none"));
    assert!(failed.contains("ms=456"));
    assert!(!failed.contains("status=ready"));
    assert!(!ready.contains("prompt="));
}

const SCENE: &str = "unknown|The Heron Shrine|live-room-photo";

fn stage(revision: u64) -> Stage {
    Stage {
        scene_key: SCENE.to_owned(),
        scene_asset: "live-room-photo".to_owned(),
        revision: Some((1, revision)),
        in_combat: false,
        in_roll: false,
        tempo: None,
        speaking_ids: BTreeSet::from(["merchant".to_owned()]),
        recent_kinds: Vec::new(),
        title: "The Heron Shrine".to_owned(),
        story_scene: None,
        cast: vec![StageCast {
            id: "merchant".to_owned(),
            name: "Merchant".to_owned(),
            sprite: "sprite-patron-merchant".to_owned(),
        }],
        speaker_id: Some("merchant".to_owned()),
        listener_id: None,
        line_length: 0,
        measured_speaker_layout: None,
        vitals: Vec::new(),
        heroes: Vec::new(),
    }
}

fn dossier() -> Dossier {
    let mut dossier = adventure(7).dossiers[0].clone();
    dossier.starting_attitude = Attitude::Indifferent;
    // The generator's temperaments move between releases; the tests pin the one they need.
    dossier.temperament.flirting.effect = Effect::Warm;
    dossier
        .emotional_profile
        .as_mut()
        .unwrap()
        .legacy_reactions
        .flirting
        .effect = Effect::Warm;
    dossier
}

fn facts(dossier: &Dossier) -> SceneFacts {
    SceneFacts {
        place: PlaceFacts::new("The Heron Shrine", "a shrine beside the marsh"),
        tone: SceneTone::Neutral,
        beat: Beat::Conversation,
        members: vec![Member {
            id: "merchant".to_owned(),
            role: CharacterRole::Patron,
            life_stage: LifeStage::Adult,
            name: "Brakka".to_owned(),
            sprite: "sprite-patron-merchant".to_owned(),
            dossier_id: Some(dossier.npc_id.clone()),
            opposes: dossier.role.opposes_party(),
            starting_attitude: Some(dossier.starting_attitude),
        }],
        fights: Vec::new(),
    }
}

/// A director with a ledger that knows `dossier`, observing `stage(1)` at time zero.
fn director(dossier: &Dossier) -> (Arc<Inner>, Arc<SocialLedger>) {
    let ledger = SocialLedger::open(vec![dossier.clone()], "0x07", None).unwrap();
    let inner = Arc::new(Inner {
        imagery: Imagery::disabled(ImageryOff::Qualification),
        references: ReferenceStore::new(PathBuf::new()),
        social: ledger.clone(),
        rating: ContentRating::Mature,
        timing: timing(),
        state: Mutex::new(State::new(timing())),
        wake: watch::channel(0).0,
        origin: Instant::now(),
    });
    inner.observe_at(stage(1), Duration::ZERO);
    locked(&inner.state).facts = Some(facts(dossier));
    (inner, ledger)
}

fn insult(ledger: &SocialLedger, dossier: &Dossier) {
    ledger.record_exchange(
        hero(1),
        dossier,
        "scene",
        Approach::Insult,
        "a hero line",
        "an npc line",
    );
}

fn flirt(ledger: &SocialLedger, dossier: &Dossier) {
    ledger.record_exchange(
        hero(1),
        dossier,
        "scene",
        Approach::Flirting,
        "a hero line",
        "an npc line",
    );
}

/// The jobs that paint or spend: everything but the cache look-ups, which are free and allowed
/// everywhere.
fn paint_jobs(jobs: Vec<Job>) -> Vec<Job> {
    jobs.into_iter()
        .filter(|job| !matches!(job, Job::Peek { .. }))
        .collect()
}

fn refresh_jobs(jobs: &[Job]) -> Vec<(Look, RefreshReason)> {
    jobs.iter()
        .filter_map(|job| match job {
            Job::Refresh { look, reason, .. } => Some((*look, *reason)),
            Job::Canonical(_)
            | Job::KillShot { .. }
            | Job::HeroCutout { .. }
            | Job::Expression { .. }
            | Job::Warm { .. }
            | Job::Peek { .. } => None,
        })
        .collect()
}

fn face_jobs(jobs: &[Job]) -> Vec<Face> {
    jobs.iter()
        .filter_map(|job| match job {
            Job::Expression { face, .. } => Some(*face),
            Job::Canonical(_)
            | Job::KillShot { .. }
            | Job::HeroCutout { .. }
            | Job::Refresh { .. }
            | Job::Warm { .. }
            | Job::Peek { .. } => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Pacing, which is pure.

#[test]
fn a_refresh_never_starts_inside_the_interval_the_first_seconds_or_a_roll() {
    let mut pacer = Pacer::new(timing(), Duration::ZERO);
    assert_eq!(
        pacer.refresh_due(seconds(5), false, true),
        None,
        "the first seconds of a scene"
    );
    assert_eq!(
        pacer.refresh_due(seconds(8), false, true),
        Some(RefreshReason::Change)
    );
    pacer.refresh_started(seconds(8), RefreshReason::Change);
    assert_eq!(pacer.refresh_due(seconds(20), false, true), None);
    assert_eq!(pacer.refresh_due(seconds(37), false, true), None);
    assert_eq!(
        pacer.refresh_due(seconds(38), false, true),
        Some(RefreshReason::Change)
    );
    // A roll (an accepted action) silences the next six seconds.
    pacer.note_action(seconds(100));
    assert_eq!(pacer.refresh_due(seconds(105), false, true), None);
    assert_eq!(
        pacer.refresh_due(seconds(106), false, true),
        Some(RefreshReason::Change)
    );
    // Nothing worth rendering and nothing idle: no refresh.
    assert_eq!(pacer.refresh_due(seconds(106), false, false), None);
}

#[test]
fn nothing_is_requested_or_published_in_combat() {
    let mut pacer = Pacer::new(timing(), Duration::ZERO);
    for now in [10, 100, 1000] {
        assert_eq!(pacer.refresh_due(seconds(now), true, true), None);
        assert!(!pacer.may_publish(seconds(now), true));
        assert!(pacer.may_publish(seconds(now), false));
    }
    pacer.note_action(seconds(2000));
    assert!(
        !pacer.may_publish(seconds(2003), false),
        "a finished image waits out a roll"
    );
    assert!(pacer.may_publish(seconds(2006), false));
}

#[test]
fn a_gentle_alternate_angle_comes_only_after_a_long_idle_and_then_sparingly() {
    let mut pacer = Pacer::new(timing(), Duration::ZERO);
    assert_eq!(pacer.refresh_due(seconds(119), false, false), None);
    assert_eq!(
        pacer.refresh_due(seconds(120), false, false),
        Some(RefreshReason::AlternateAngle)
    );
    pacer.refresh_started(seconds(120), RefreshReason::AlternateAngle);
    assert_eq!(pacer.alternates(), 1);
    assert_eq!(
        pacer.refresh_due(seconds(209), false, false),
        None,
        "every third interval at most"
    );
    assert_eq!(
        pacer.refresh_due(seconds(210), false, false),
        Some(RefreshReason::AlternateAngle)
    );
    // A story event ends the idleness.
    pacer.note_story_event(seconds(215));
    assert_eq!(pacer.refresh_due(seconds(300), false, false), None);
}

#[test]
fn alternate_angles_cycle_through_gentle_shots_keeping_the_mood_and_light() {
    let target = Look::of(Mood::Warm);
    let shots: Vec<_> = (0..4).map(|n| alternate_look(target, n).shot).collect();
    assert_eq!(shots.len(), 4);
    assert!(shots.iter().collect::<BTreeSet<_>>().len() == 4);
    for index in 0..8 {
        let look = alternate_look(target, index);
        assert_eq!(look.mood, Mood::Warm);
        assert_eq!(look.lighting, target.lighting);
    }
}

// ---------------------------------------------------------------------------------------------
// The director.

#[test]
fn an_exchange_asks_for_the_face_at_once_and_the_backdrop_only_when_the_interval_allows() {
    let dossier = dossier();
    let (inner, ledger) = director(&dossier);
    assert!(
        paint_jobs(inner.evaluate_at(seconds(10))).is_empty(),
        "no exchange, nothing to repaint"
    );

    insult(&ledger, &dossier);
    let jobs = inner.evaluate_at(seconds(10));
    assert_eq!(
        face_jobs(&jobs),
        vec![Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy
        }]
    );
    let refreshes = refresh_jobs(&jobs);
    assert_eq!(refreshes.len(), 1);
    assert_eq!(refreshes[0].1, RefreshReason::Change);
    assert_ne!(refreshes[0].0, Look::AUTHORED);

    // Neither is asked for again while in flight.
    assert!(inner.evaluate_at(seconds(11)).is_empty());

    // The face lands; the backdrop does too after its quiet window.
    inner.finish(Done::Expression {
        npc_id: "merchant".to_owned(),
        face: Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy,
        },
        source_sprite: "sprite-patron-merchant".to_owned(),
        result: Ok(key('a')),
    });
    inner.finish(Done::Refresh {
        look: refreshes[0].0,
        reason: RefreshReason::Change,
        scene_key: SCENE.to_owned(),
        result: Ok(key('b')),
    });
    assert!(inner.evaluate_at(seconds(12)).is_empty());
    assert_eq!(
        locked(&inner.state).backdrop.as_ref().unwrap().key,
        key('b')
    );

    // A second exchange 10 seconds after the first repaint starts: the face may follow at once
    // but the backdrop waits for the interval.
    flirt(&ledger, &dossier);
    let early = inner.evaluate_at(seconds(20));
    assert!(refresh_jobs(&early).is_empty(), "never under the interval");
    let later = inner.evaluate_at(seconds(41));
    assert_eq!(refresh_jobs(&later).len(), 1);
}

#[test]
fn a_finished_backdrop_waits_for_a_quiet_moment_and_then_wakes_the_watchers() {
    let dossier = dossier();
    let (inner, ledger) = director(&dossier);
    insult(&ledger, &dossier);
    let jobs = inner.evaluate_at(seconds(10));
    let look = refresh_jobs(&jobs)[0].0;
    let mut wakes = inner.wake.subscribe();
    wakes.borrow_and_update();

    // An action (a roll) at second 11 holds the finished image until second 17.
    let mut changed = stage(2);
    changed.revision = Some((1, 2));
    inner.observe_at(changed, seconds(11));
    inner.finish(Done::Refresh {
        look,
        reason: RefreshReason::Change,
        scene_key: SCENE.to_owned(),
        result: Ok(key('c')),
    });
    inner.evaluate_at(seconds(12));
    assert!(locked(&inner.state).backdrop.is_none());
    assert!(!wakes.has_changed().unwrap());
    inner.evaluate_at(seconds(17));
    assert_eq!(
        locked(&inner.state).backdrop.as_ref().unwrap().key,
        key('c')
    );
    assert!(
        wakes.has_changed().unwrap(),
        "watchers are woken to rebuild their view"
    );
    assert_eq!(locked(&inner.state).tracker.shown_look(), look);
}

// ---------------------------------------------------------------------------------------------
// Health faces.

const HERO_ONE_ART: &str = "assets/ui/generated/sprite-hero-1-documentary-photo";

fn hurt_stage(revision: u64, in_combat: bool, hero_hp: u32, foe_hp: u32) -> Stage {
    Stage {
        in_combat,
        heroes: vec![StageHero {
            index: 0,
            name: "Rowan".to_owned(),
            vitals: Vitals::new(hero_hp, 13),
        }],
        vitals: vec![("Brakka".to_owned(), Vitals::new(foe_hp, 13))],
        ..stage(revision)
    }
}

fn job_labels(jobs: &[Job]) -> Vec<String> {
    jobs.iter()
        .map(|job| match job {
            Job::Canonical(_) => "canonical-reference".to_owned(),
            Job::KillShot { .. } => "kill-shot".to_owned(),
            Job::HeroCutout { name, .. } => format!("hero-cutout {name}"),
            Job::Warm { character, tier } => format!("warm {} {}", character.art, tier.label()),
            Job::Peek { character } => format!("peek {}", character.art),
            Job::Expression { .. } => "expression".to_owned(),
            Job::Refresh { .. } => "refresh".to_owned(),
        })
        .collect()
}

fn learn(inner: &Inner, art: &str, tier: HealthTier, byte: char) {
    let mut state = locked(&inner.state);
    state.book.learn(art, tier, key(byte));
    refresh_health(&mut state);
}

#[test]
fn a_fight_only_looks_in_the_cache_and_asks_for_the_tier_each_character_is_at() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    let scene_facts = facts(&dossier);
    // Both are badly hurt, and nothing is cached.
    let fight = hurt_stage(2, true, 4, 2);
    inner.observe_at(fight.clone(), seconds(20));
    let jobs = inner.evaluate_at(seconds(60));
    let labels = job_labels(&jobs);
    assert_eq!(
        labels,
        [
            format!("peek {HERO_ONE_ART}"),
            "peek sprite-patron-merchant".to_owned()
        ]
    );
    // The hero is at Bloodied and the foe at Critical: those are the only variants made.
    let mut book = Book::default();
    let characters = characters_of(&fight, &scene_facts, &[]);
    assert_eq!(
        job_labels(&warm_jobs(
            &mut book,
            &fight,
            &scene_facts,
            &characters,
            &[],
        )),
        [
            format!("warm {HERO_ONE_ART} bloodied"),
            "warm sprite-patron-merchant critical".to_owned(),
        ]
    );
    let mut message = display_message(true);
    inner.apply(stage_of(&message).as_ref(), &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        panic!("display view");
    };
    // The hero is always published; with no variant and no live picture both fields are
    // empty, so every screen keeps its authored (stock) art.
    assert_eq!(
        display.journey.as_ref().unwrap().live_faces,
        [rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: String::new(),
            battle_sprite: String::new(),
        }]
    );
    assert_eq!(display.scene.as_ref().unwrap().cast[0].live_sprite, "");
}

#[test]
fn a_fresh_scene_of_full_health_characters_requests_no_health_faces() {
    let dossier = dossier();
    let scene_facts = facts(&dossier);
    // Both at full health, nothing cached: the scene makes no health face at all.
    let fresh = hurt_stage(2, false, 13, 13);
    let characters = characters_of(&fresh, &scene_facts, &[]);
    let mut book = Book::default();
    assert!(warm_jobs(&mut book, &fresh, &scene_facts, &characters, &[]).is_empty());
}

#[test]
fn a_character_asks_for_the_tier_it_reaches_and_no_other() {
    let dossier = dossier();
    let scene_facts = facts(&dossier);
    let mut book = Book::default();
    // Eight of thirteen is Hurt; the foe at full health asks for nothing.
    let hurt = hurt_stage(2, false, 8, 13);
    let characters = characters_of(&hurt, &scene_facts, &[]);
    assert_eq!(
        job_labels(&warm_jobs(&mut book, &hurt, &scene_facts, &characters, &[],)),
        [format!("warm {HERO_ONE_ART} hurt")]
    );
    // Three of thirteen is Critical: the hero asks for Critical, and nothing else is new.
    let worse = hurt_stage(3, false, 3, 13);
    let characters = characters_of(&worse, &scene_facts, &[]);
    assert_eq!(
        job_labels(&warm_jobs(
            &mut book,
            &worse,
            &scene_facts,
            &characters,
            &[],
        )),
        [format!("warm {HERO_ONE_ART} critical")]
    );
}

#[test]
fn a_fight_shows_the_nearest_cached_variant_and_nothing_further() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    inner.observe_at(hurt_stage(2, true, 4, 1), seconds(20));
    // Only Bloodied is cached for both: the hero (Bloodied) shows it exactly, and the foe at
    // Critical (one tier away, against three from the authored art) shows it too.
    learn(&inner, HERO_ONE_ART, HealthTier::Bloodied, 'b');
    learn(&inner, "sprite-patron-merchant", HealthTier::Bloodied, 'c');
    let mut message = display_message(true);
    inner.apply(stage_of(&message).as_ref(), &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        panic!("display view");
    };
    assert_eq!(
        display.journey.as_ref().unwrap().live_faces,
        [rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: key('b').path(),
            // Hero one wears the same art on the battlefield, so the same variant.
            battle_sprite: key('b').path(),
        }]
    );
    assert_eq!(
        display.scene.as_ref().unwrap().cast[0].live_sprite,
        key('c').path()
    );
    // The phone gets the hero's face as well.
    let mut phone = rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Player(
            rpc::PlayerGameplayView {
                journey: Some(rpc::JourneyView::default()),
                ..Default::default()
            },
        )),
    };
    inner.apply(None, &mut phone);
    let Some(rpc::view_message::Audience::Player(player)) = &phone.audience else {
        panic!("player view");
    };
    assert_eq!(player.journey.as_ref().unwrap().live_faces.len(), 1);
    // The hero heals above the Hurt line and has no live picture: the face stays published
    // with empty fields, so the authored (stock) art returns on every screen.
    inner.observe_at(hurt_stage(3, true, 12, 1), seconds(30));
    let mut after = display_message(true);
    inner.apply(stage_of(&after).as_ref(), &mut after);
    let Some(rpc::view_message::Audience::Display(display)) = &after.audience else {
        panic!("display view");
    };
    assert_eq!(
        display.journey.as_ref().unwrap().live_faces,
        [rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: String::new(),
            battle_sprite: String::new(),
        }]
    );
}

fn give_live_picture(inner: &Inner, name: &str, byte: char) {
    let mut state = locked(&inner.state);
    state.hero_cutouts.insert(name.to_owned(), key(byte));
    refresh_health(&mut state);
}

fn published_face(inner: &Inner) -> rpc::LiveFace {
    let mut message = display_message(true);
    let Some(rpc::view_message::Audience::Display(display)) = &mut message.audience else {
        panic!("display view");
    };
    display.journey.as_mut().unwrap().party = vec![rpc::PartyMemberView {
        name: "Rowan".to_owned(),
        character_ready: true,
        hit_points: 13,
        maximum_hit_points: 13,
        ..Default::default()
    }];
    inner.apply(stage_of(&message).as_ref(), &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        panic!("display view");
    };
    display.journey.as_ref().unwrap().live_faces[0].clone()
}

#[test]
fn an_unhurt_hero_publishes_their_own_live_picture_on_stage_and_battlefield() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    give_live_picture(&inner, "Rowan", 'a');
    // Full health (13 of 13): no variant exists and none is made, yet the hero's own
    // picture is published for both screens instead of the stock photo.
    inner.observe_at(hurt_stage(2, true, 13, 13), seconds(20));
    assert_eq!(
        published_face(&inner),
        rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: key('a').path(),
            battle_sprite: key('a').path(),
        }
    );
    // Generation stays on first need: a healthy hero asks for no health variant.
    let scene_facts = facts(&dossier);
    let stage = hurt_stage(2, true, 13, 13);
    let characters = characters_of(&stage, &scene_facts, &[Some(key('a'))]);
    let mut book = Book::default();
    let jobs = warm_jobs(
        &mut book,
        &stage,
        &scene_facts,
        &characters,
        &[Some(key('a'))],
    );
    assert!(jobs.is_empty());
}

#[test]
fn a_hurt_heros_variant_is_made_from_their_live_picture_not_the_stock_photo() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    give_live_picture(&inner, "Rowan", 'a');
    // Bloodied (4 of 13).
    inner.observe_at(hurt_stage(2, true, 4, 13), seconds(20));
    // While the variant is being made the hero shows their own live picture, and a variant
    // cached earlier from the stock art is not shown for them.
    learn(&inner, HERO_ONE_ART, HealthTier::Bloodied, '5');
    assert_eq!(published_face(&inner).sprite, key('a').path());
    assert_eq!(published_face(&inner).battle_sprite, key('a').path());
    // The one variant asked for is sourced from the live picture, for both screens.
    let scene_facts = facts(&dossier);
    let stage = hurt_stage(2, true, 4, 13);
    let characters = characters_of(&stage, &scene_facts, &[Some(key('a'))]);
    assert_eq!(
        characters
            .iter()
            .filter(|character| character.facts.role == CharacterRole::Hero)
            .map(|character| character.art.as_str())
            .collect::<Vec<_>>(),
        [key('a').path().as_str()]
    );
    let mut book = Book::default();
    let jobs = warm_jobs(
        &mut book,
        &stage,
        &scene_facts,
        &characters,
        &[Some(key('a'))],
    );
    assert_eq!(
        job_labels(&jobs),
        [format!("warm {} bloodied", key('a').path())]
    );
    // Once that variant exists it serves the stage sprite and the battle face alike.
    learn(&inner, &key('a').path(), HealthTier::Bloodied, 'b');
    assert_eq!(
        published_face(&inner),
        rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: key('b').path(),
            battle_sprite: key('b').path(),
        }
    );
}

#[test]
fn the_stock_photo_is_the_source_and_face_only_before_a_live_picture_exists() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    // Bloodied, nothing cached and no live picture yet.
    inner.observe_at(hurt_stage(2, true, 4, 13), seconds(20));
    assert_eq!(
        published_face(&inner),
        rpc::LiveFace {
            name: "Rowan".to_owned(),
            sprite: String::new(),
            battle_sprite: String::new(),
        },
        "empty fields keep the authored (stock) art on every screen"
    );
    let scene_facts = facts(&dossier);
    let stage = hurt_stage(2, true, 4, 13);
    let characters = characters_of(&stage, &scene_facts, &[]);
    let mut book = Book::default();
    assert_eq!(
        job_labels(&warm_jobs(
            &mut book,
            &stage,
            &scene_facts,
            &characters,
            &[],
        )),
        [format!("warm {HERO_ONE_ART} bloodied")]
    );
    // As soon as the live picture exists the face and the variant source switch to it.
    give_live_picture(&inner, "Rowan", 'a');
    assert_eq!(published_face(&inner).sprite, key('a').path());
    let characters = characters_of(&stage, &scene_facts, &[Some(key('a'))]);
    assert_eq!(characters[0].art, key('a').path());
}

#[test]
fn outside_a_fight_what_the_party_looks_like_now_is_made_first_and_only_once() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    inner.observe_at(hurt_stage(2, false, 3, 13), seconds(20));
    let labels = job_labels(&inner.evaluate_at(seconds(25)));
    assert_eq!(
        labels,
        [
            format!("peek {HERO_ONE_ART}"),
            "peek sprite-patron-merchant".to_owned(),
            format!("warm {HERO_ONE_ART} critical"),
        ]
    );
    assert!(
        job_labels(&inner.evaluate_at(seconds(26))).is_empty(),
        "asked for once while it is being made"
    );
}

#[test]
fn opening_scene_requests_each_hero_cutout_once_and_caches_successes() {
    let timing = timing();
    let inner = Arc::new(Inner {
        imagery: Imagery::disabled(ImageryOff::Qualification),
        references: ReferenceStore::new(PathBuf::new()),
        social: SocialLedger::open(Vec::new(), "0x07", None).unwrap(),
        rating: ContentRating::Mature,
        timing,
        state: Mutex::new(State::new(timing)),
        wake: watch::channel(0).0,
        origin: Instant::now(),
    });
    let opening = serde_json::from_value::<pack::Scene>(serde_json::json!({
        "key": "opening",
        "title": "The Heron Shrine",
        "narration": "",
        "cast": [],
        "speaker": "",
        "dialogue": "",
        "asset": "live-room-photo",
        "setting": {
            "place_id": "temple",
            "name": "The Heron Shrine",
            "short": "a shrine beside the marsh",
            "kind": "temple",
            "exterior": false,
            "art_key": "temple"
        },
        "speaker_portrait": "",
        "findings": []
    }))
    .unwrap();
    let mut opening_stage = stage(2);
    opening_stage.story_scene = Some(opening);
    opening_stage.heroes = vec![
        StageHero {
            index: 0,
            name: "Brakka".to_owned(),
            vitals: Vitals {
                current: 10,
                maximum: 10,
                unconscious: false,
                dying: false,
            },
        },
        StageHero {
            index: 1,
            name: "Sigrun".to_owned(),
            vitals: Vitals {
                current: 10,
                maximum: 10,
                unconscious: false,
                dying: false,
            },
        },
    ];
    inner.observe_at(opening_stage, Duration::ZERO);

    let cutout_names = |jobs: Vec<Job>| {
        jobs.into_iter()
            .filter_map(|job| match job {
                Job::HeroCutout { name, .. } => Some(name),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        cutout_names(inner.evaluate_at(seconds(20))),
        vec!["Brakka".to_owned(), "Sigrun".to_owned()]
    );
    assert!(cutout_names(inner.evaluate_at(seconds(21))).is_empty());

    inner.finish(Done::HeroCutout {
        name: "Brakka".to_owned(),
        result: Ok(key('a')),
    });
    inner.finish(Done::HeroCutout {
        name: "Sigrun".to_owned(),
        result: Ok(key('b')),
    });
    assert!(cutout_names(inner.evaluate_at(seconds(22))).is_empty());
    assert_eq!(locked(&inner.state).hero_cutouts.len(), 2);
}

#[test]
fn the_shipped_harbor_scene_builds_a_graph_from_its_painted_backdrop() {
    let scene = serde_json::from_value::<pack::Scene>(serde_json::json!({
        "key": "opening",
        "title": "The Broken Seal",
        "narration": "Rain moves over the Lantern Wharf.",
        "rules_note": "",
        "cast": [{
            "id": "courier",
            "name": "Courier",
            "role": "patron",
            "sprite": "assets/ui/generated/sprite-courier",
            "portrait": "assets/ui/generated/portrait-courier",
            "voice": "hurried and anxious",
            "gender": "he",
            "age": "young"
        }],
        "narration_rested": null,
        "speaker": "Courier",
        "dialogue": "Please help me.",
        "asset": "assets/ui/scenes/mara-harbor-v4.png",
        "speaker_portrait": "",
        "findings": []
    }))
    .unwrap();
    let mut stage = stage(2);
    stage.story_scene = Some(scene);
    stage.cast = vec![StageCast {
        id: "courier".to_owned(),
        name: "Courier".to_owned(),
        sprite: "assets/ui/generated/sprite-courier".to_owned(),
    }];
    stage.speaker_id = Some("courier".to_owned());
    stage.heroes = vec![StageHero {
        index: 0,
        name: "Brakka".to_owned(),
        vitals: Vitals::new(13, 13),
    }];

    let mut state = State::new(timing());
    state.hero_faces.insert(
        "Brakka".to_owned(),
        HeroFace {
            stage: Some(key('b')),
            battle: Some(key('b')),
        },
    );
    let graph = graph_of(&stage, Some(&state)).expect("painted harbor scene graph");
    assert_eq!(graph.set.place_id, "lantern-wharf");
    assert_eq!(graph.set.place_name, "Lantern Wharf");
    assert_eq!(
        graph.set.fallback_asset,
        "/assets/ui/scenes/mara-harbor-v4.png"
    );
    assert!(
        graph
            .actors
            .iter()
            .any(|actor| actor.id == "hero-1" && actor.art == key('b').path())
    );
    assert!(graph.actors.iter().any(|actor| actor.id == "courier"));
}

#[test]
fn bloodied_or_worse_outranks_a_mood_face_and_hurt_only_fills_in_where_there_is_none() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    inner.observe_at(stage(2), seconds(20));
    locked(&inner.state).sprites.insert(
        "merchant".to_owned(),
        SpriteOverlay {
            source_sprite: "sprite-patron-merchant".to_owned(),
            expression: Expression::Neutral,
            key: key('a'),
        },
    );
    learn(&inner, "sprite-patron-merchant", HealthTier::Hurt, 'e');
    learn(&inner, "sprite-patron-merchant", HealthTier::Bloodied, 'b');
    // Hurt: the angry face stays.
    inner.observe_at(hurt_stage(3, false, 13, 8), seconds(30));
    assert!(locked(&inner.state).health_sprites.is_empty());
    assert!(locked(&inner.state).sprites.contains_key("merchant"));
    // Bloodied: the health face takes over and the mood face is dropped.
    inner.observe_at(hurt_stage(4, false, 13, 5), seconds(40));
    let state = locked(&inner.state);
    assert_eq!(state.health_sprites["merchant"].key, key('b'));
    assert!(state.sprites.is_empty());
}

#[test]
fn the_foes_of_the_adventures_fights_are_looked_for_after_the_party_and_the_stage() {
    let dossier = dossier();
    let mut scene_facts = facts(&dossier);
    let foe = |sprite: &str| Member {
        id: sprite.to_owned(),
        sprite: sprite.to_owned(),
        name: "Foe".to_owned(),
        ..scene_facts.members[0].clone()
    };
    scene_facts.fights = vec![
        foe("sprite-ant-bandit"),
        // Already on this stage: listed once, with the stage.
        foe("sprite-patron-merchant"),
    ];
    let characters = characters_of(&hurt_stage(2, false, 13, 13), &scene_facts, &[]);
    let arts: Vec<&str> = characters
        .iter()
        .map(|character| character.art.as_str())
        .collect();
    assert_eq!(
        arts,
        [HERO_ONE_ART, "sprite-patron-merchant", "sprite-ant-bandit"]
    );
}

#[test]
fn the_second_hero_gets_variants_of_the_art_the_battlefield_dresses_the_seat_in() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    let mut stage = hurt_stage(2, true, 4, 13);
    stage.heroes.push(StageHero {
        index: 1,
        name: "Thrain".to_owned(),
        vitals: Vitals::new(4, 13),
    });
    inner.observe_at(stage, seconds(20));
    // The exploration stage shows hero two as `sprite-hero-2`, the fight as the archer
    // `sprite-hero-3`: both are looked for, and a fight shows the variant of the fight's art.
    let labels = job_labels(&inner.evaluate_at(seconds(60)));
    assert!(
        labels.contains(&"peek assets/ui/generated/sprite-hero-2-documentary-photo".to_owned())
    );
    assert!(
        labels.contains(&"peek assets/ui/generated/sprite-hero-3-documentary-photo".to_owned())
    );
    learn(
        &inner,
        "assets/ui/generated/sprite-hero-2-documentary-photo",
        HealthTier::Bloodied,
        'c',
    );
    learn(
        &inner,
        "assets/ui/generated/sprite-hero-3-documentary-photo",
        HealthTier::Bloodied,
        'd',
    );
    let mut message = display_message(true);
    inner.apply(stage_of(&message).as_ref(), &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        panic!("display view");
    };
    let faces = &display.journey.as_ref().unwrap().live_faces;
    let thrain = faces.iter().find(|face| face.name == "Thrain").unwrap();
    assert_eq!(thrain.sprite, key('c').path());
    assert_eq!(thrain.battle_sprite, key('d').path());
}

#[test]
fn a_combat_view_neither_triggers_nor_shows_generated_art() {
    let dossier = dossier();
    let (inner, ledger) = director(&dossier);
    insult(&ledger, &dossier);
    let mut fighting = stage(2);
    fighting.in_combat = true;
    inner.observe_at(fighting, seconds(20));
    assert!(paint_jobs(inner.evaluate_at(seconds(60))).is_empty());
    locked(&inner.state).backdrop = Some(BackdropOverlay { key: key('d') });
    let mut message = display_message(true);
    inner.apply(stage_of(&message).as_ref(), &mut message);
    let Some(rpc::view_message::Audience::Display(display)) = &message.audience else {
        panic!("display view");
    };
    assert_eq!(display.scene.as_ref().unwrap().live_backdrop, "");
}

#[test]
fn a_neutral_face_reverts_to_the_authored_cutout_for_free() {
    let dossier = dossier();
    let (inner, ledger) = director(&dossier);
    insult(&ledger, &dossier);
    inner.evaluate_at(seconds(10));
    inner.finish(Done::Expression {
        npc_id: "merchant".to_owned(),
        face: Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy,
        },
        source_sprite: "sprite-patron-merchant".to_owned(),
        result: Ok(key('e')),
    });
    assert!(locked(&inner.state).sprites.contains_key("merchant"));
    // After the hold the expression relaxes: the overlay is dropped and nothing is requested.
    let jobs = inner.evaluate_at(seconds(100));
    assert!(face_jobs(&jobs).is_empty());
    assert!(!locked(&inner.state).sprites.contains_key("merchant"));
}

#[test]
fn a_failed_face_is_not_asked_for_again() {
    let dossier = dossier();
    let (inner, ledger) = director(&dossier);
    insult(&ledger, &dossier);
    let jobs = inner.evaluate_at(seconds(10));
    assert_eq!(face_jobs(&jobs).len(), 1);
    inner.finish(Done::Expression {
        npc_id: "merchant".to_owned(),
        face: face_jobs(&jobs)[0],
        source_sprite: "sprite-patron-merchant".to_owned(),
        result: Err(ImageFailure::Refused),
    });
    assert!(face_jobs(&inner.evaluate_at(seconds(12))).is_empty());
}

#[test]
fn a_new_scene_drops_the_art_of_the_old_one_and_wakes_the_watchers() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    {
        let mut state = locked(&inner.state);
        state.backdrop = Some(BackdropOverlay { key: key('f') });
        state.sprites.insert(
            "merchant".to_owned(),
            SpriteOverlay {
                source_sprite: "sprite-patron-merchant".to_owned(),
                expression: Expression::Neutral,
                key: key('1'),
            },
        );
    }
    let mut wakes = inner.wake.subscribe();
    wakes.borrow_and_update();
    let mut next = stage(2);
    next.scene_key = "Another place|assets/ui/generated/scene-other.webp".to_owned();
    inner.observe_at(next, seconds(50));
    let state = locked(&inner.state);
    assert!(state.backdrop.is_none() && state.sprites.is_empty());
    drop(state);
    assert!(wakes.has_changed().unwrap());
}

// ---------------------------------------------------------------------------------------------
// The view.

fn display_message(in_combat: bool) -> rpc::ViewMessage {
    rpc::ViewMessage {
        revision: Some(rpc::SessionRevision {
            epoch: Some(rpc::RecoveryEpoch { value: Some(1) }),
            sequence: Some(1),
        }),
        audience: Some(rpc::view_message::Audience::Display(
            rpc::DisplayGameplayView {
                narration: "The shrine is quiet.".to_owned(),
                scene_asset: "live-room-photo".to_owned(),
                scene: Some(rpc::GameplayScene {
                    destination: 0,
                    source_scene_key: String::new(),
                    source_scene_variant: String::new(),
                    title: "The Heron Shrine".to_owned(),
                    scene_asset: "live-room-photo".to_owned(),
                    speaker_portrait: "/assets/ui/generated/portrait-patron-merchant".to_owned(),
                    cast: vec![rpc::SceneActor {
                        id: "merchant".to_owned(),
                        name: "Brakka".to_owned(),
                        role: rpc::SceneRole::Patron as i32,
                        sprite: "sprite-patron-merchant".to_owned(),
                        portrait: "portrait-patron-merchant".to_owned(),
                        speaking: true,
                        live_sprite: String::new(),
                    }],
                    scene_video_state: 0,
                    scene_video_path: String::new(),
                    rules_note: String::new(),
                    place: String::new(),
                    location_id: String::new(),
                    live_backdrop: String::new(),
                    live_plate: String::new(),
                    plate_layout: None,
                    plate_state: 0,
                    plate_update_pending: false,
                    scene_contents_json: String::new(),
                    item: None,
                    hand_items: Vec::new(),
                    interior: false,
                    floor: None,
                }),
                journey: Some(rpc::JourneyView {
                    phase: if in_combat {
                        rpc::JourneyPhase::Combat as i32
                    } else {
                        rpc::JourneyPhase::Dialogue as i32
                    },
                    speaker: "Brakka".to_owned(),
                    dialogue: "Welcome.".to_owned(),
                    tempo: Some(rpc::TempoCue {
                        intensity_permille: 640,
                        phase: rpc::TempoPhase::Build as i32,
                        sequence: 3,
                        hold_millis: 1000,
                    }),
                    table_log: vec![
                        rpc::TableLine {
                            roll: None,
                            actor_id: String::new(),
                            sequence: 1,
                            speaker: "Hero".to_owned(),
                            role: rpc::TableRole::Hero as i32,
                            hero_index: 0,
                            kind: rpc::TableKind::Speech as i32,
                            text: "private words never read".to_owned(),
                        },
                        rpc::TableLine {
                            roll: None,
                            actor_id: String::new(),
                            sequence: 2,
                            speaker: "Brakka".to_owned(),
                            role: rpc::TableRole::Npc as i32,
                            hero_index: 0,
                            kind: rpc::TableKind::Action as i32,
                            text: "grows wary".to_owned(),
                        },
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    }
}

#[test]
fn a_display_view_gives_the_stage_its_scene_cast_tempo_and_table_kinds() {
    let message = display_message(false);
    let stage = stage_of(&message).unwrap();
    assert_eq!(stage.scene_key, SCENE);
    assert_eq!(stage.revision, Some((1, 1)));
    assert!(!stage.in_combat);
    assert_eq!(
        stage.tempo,
        Some(Tempo {
            phase: TempoLevel::Build,
            intensity_permille: 640
        })
    );
    assert_eq!(stage.cast.len(), 1);
    assert_eq!(stage.cast[0].id, "merchant");
    assert!(stage.speaking_ids.contains("merchant"));
    assert_eq!(
        stage.recent_kinds,
        vec![TableKind::Speech, TableKind::Action]
    );
    assert!(stage_of(&display_message(true)).unwrap().in_combat);
}

#[test]
fn a_player_view_is_never_observed_or_changed() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    let player = rpc::ViewMessage {
        revision: None,
        audience: Some(rpc::view_message::Audience::Player(
            rpc::PlayerGameplayView {
                scene: Some(rpc::GameplayScene {
                    title: "The Heron Shrine".to_owned(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )),
    };
    assert!(stage_of(&player).is_none());
    locked(&inner.state).backdrop = Some(BackdropOverlay { key: key('9') });
    let living = LivingScenes {
        inner: Some(inner),
        _worker: None,
        plates: None,
        last_graph_revision: Arc::new(Mutex::new(None)),
    };
    let mut after = player.clone();
    living.apply(&mut after);
    assert_eq!(after, player);
}

#[test]
fn published_art_changes_only_the_art_fields_of_the_display_view() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    {
        let mut state = locked(&inner.state);
        state.backdrop = Some(BackdropOverlay { key: key('2') });
        state.sprites.insert(
            "merchant".to_owned(),
            SpriteOverlay {
                source_sprite: "sprite-patron-merchant".to_owned(),
                expression: Expression::Neutral,
                key: key('3'),
            },
        );
    }
    let living = LivingScenes {
        inner: Some(inner),
        _worker: None,
        plates: None,
        last_graph_revision: Arc::new(Mutex::new(None)),
    };
    let before = display_message(false);
    let mut after = before.clone();
    living.apply(&mut after);
    assert_ne!(after, before);

    let Some(rpc::view_message::Audience::Display(display)) = &mut after.audience else {
        panic!("display view");
    };
    let scene = display.scene.as_mut().unwrap();
    assert_eq!(
        scene.live_backdrop,
        format!("/gameplay/imagery/{}", "2".repeat(64))
    );
    assert_eq!(
        scene.cast[0].live_sprite,
        format!("/gameplay/imagery/{}", "3".repeat(64))
    );
    // Put the two art fields back: every other field, the revision included, is untouched.
    scene.live_backdrop.clear();
    scene.cast[0].live_sprite.clear();
    assert_eq!(after, before);
}

#[test]
fn a_sprite_overlay_applies_only_to_the_cutout_it_was_made_from() {
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    locked(&inner.state).sprites.insert(
        "merchant".to_owned(),
        SpriteOverlay {
            source_sprite: "sprite-someone-else".to_owned(),
            expression: Expression::Neutral,
            key: key('4'),
        },
    );
    let living = LivingScenes {
        inner: Some(inner),
        _worker: None,
        plates: None,
        last_graph_revision: Arc::new(Mutex::new(None)),
    };
    let before = display_message(false);
    let mut after = before.clone();
    living.apply(&mut after);
    assert_eq!(after, before);
}

#[tokio::test]
async fn an_expression_is_made_from_the_art_the_stage_shows_for_that_character() {
    let directory = std::env::temp_dir().join(format!("df-living-art-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for name in [
        "sprite-hero-1-documentary-photo.webp",
        "figure-hero-1-documentary-photo.webp",
        "sprite-shop-soldier-documentary-photo.webp",
        // A painted file on disk is never read.
        "sprite-ant-bandit.webp",
        "figure-ant-bandit.webp",
    ] {
        std::fs::write(directory.join(name), name.as_bytes()).unwrap();
    }
    let dossier = dossier();
    let (inner, _ledger) = director(&dossier);
    let inner = Arc::new(Inner {
        references: ReferenceStore::new(directory),
        imagery: Imagery::disabled(ImageryOff::Qualification),
        social: inner.social.clone(),
        rating: ContentRating::Mature,
        timing: timing(),
        state: Mutex::new(State::new(timing())),
        wake: watch::channel(0).0,
        origin: Instant::now(),
    });
    // A hero has a full-body twin and the stage shows it wherever feet are on screen.
    let figure = inner
        .load_figure("assets/ui/generated/sprite-hero-1-documentary-photo")
        .await
        .unwrap();
    assert_eq!(&figure.bytes[..], b"figure-hero-1-documentary-photo.webp");
    // A shop character has only the knees-up cut-out.
    let cutout = inner
        .load_figure("assets/ui/generated/sprite-shop-soldier-documentary-photo")
        .await
        .unwrap();
    assert_eq!(
        &cutout.bytes[..],
        b"sprite-shop-soldier-documentary-photo.webp"
    );
    // A painted stem and a reviewed photograph that is absent from the directory both fail.
    for refused in [
        "sprite-ant-bandit",
        "sprite-shop-adventurer-documentary-photo",
    ] {
        assert!(
            inner
                .load_figure(&format!("assets/ui/generated/{refused}"))
                .await
                .is_err(),
            "{refused}"
        );
    }
}

#[test]
fn disabled_living_scenes_observe_and_change_nothing() {
    let living = LivingScenes::disabled();
    let before = display_message(false);
    let mut after = before.clone();
    living.observe(&after);
    living.apply(&mut after);
    assert_eq!(after, before);
    assert!(living.subscribe().is_none());
}

fn kill_shot_request() -> kill_shot::KillShotRequest {
    let outcome = rpc::CombatOutcome {
        actor_name: "Sigrun".into(),
        target_name: "Bandit".into(),
        weapon: "greataxe".into(),
        damage_type: "slashing".into(),
        sequence: 81,
        kind: rpc::OutcomeKind::Attack as i32,
        hit: true,
        damage: 15,
        target_hit_points: 0,
        ..Default::default()
    };
    kill_shot::decide(
        &kill_shot::committed_blow(&outcome, "Market street", None),
        3,
    )
    .unwrap()
}

fn kill_shot_still(references: [Option<ImageKey>; 3]) -> KillStill {
    KillStill {
        request: kill_shot_request(),
        subjects: [
            KillSubject::Npc("sigrun".into()),
            KillSubject::Npc("bandit".into()),
        ],
        room: "market-street".into(),
        references,
        started: Instant::now(),
    }
}

#[test]
fn kill_shot_still_carries_both_named_identities_and_the_room_in_reference_order() {
    use bytes::Bytes;
    use df_providers::ImageFormat;
    let references = ["killer photograph", "victim photograph", "room photograph"]
        .map(|bytes| ReferenceImage::new(ImageFormat::Jpeg, Bytes::from_static(bytes.as_bytes())));
    let image = kill_still_request(
        &kill_shot_request(),
        ContentRating::Restrained,
        references.to_vec(),
    );
    assert_eq!(image.kind, ImageKind::ScenePlate);
    assert_eq!(
        (
            image.output.width,
            image.output.height,
            image.output.transparent
        ),
        (2560, 1440, false)
    );
    assert_eq!(
        image
            .references
            .iter()
            .map(|reference| &reference.sha256)
            .collect::<Vec<_>>(),
        references
            .iter()
            .map(|reference| &reference.sha256)
            .collect::<Vec<_>>()
    );
    let prompt = image.prompt.as_str();
    assert!(prompt.contains("Image 1") && prompt.contains("named Sigrun"));
    assert!(prompt.contains("Image 2") && prompt.contains("named Bandit"));
    assert!(prompt.contains("Image 3 is the canonical photograph"));
    assert!(prompt.contains("strike is landing now"));
    assert!(prompt.contains("No gore"));
}

#[tokio::test]
async fn kill_shot_lethal_blow_generates_one_still_even_after_duplicate_delivery() {
    use crate::gameplay::imagery::tests::DirectionFixture;
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../artifacts/tmp/lanes/K2/kill-still-test-{}",
        std::process::id()
    ));
    let fixture = DirectionFixture::photographic(&directory);
    let (original, _) = director(&dossier());
    let inner = Arc::new(Inner {
        imagery: fixture.imagery.clone(),
        references: ReferenceStore::new(PathBuf::new()),
        social: original.social.clone(),
        rating: ContentRating::Restrained,
        timing: timing(),
        state: Mutex::new(State::new(timing())),
        wake: watch::channel(0).0,
        origin: Instant::now(),
    });
    // The canonical inputs are registered imagery bytes; the fake accepts every request locally.
    let mut keys = Vec::new();
    for name in ["Sigrun", "Bandit", "Market street"] {
        let mut request = kill_shot_request();
        request.killer = name.into();
        let image = kill_still_request(&request, ContentRating::Restrained, Vec::new());
        keys.push(
            fixture
                .imagery
                .generate("canonical-fixture", image)
                .await
                .unwrap(),
        );
    }
    let keys: [ImageKey; 3] = keys.try_into().unwrap();
    let still = kill_shot_still(keys.clone().map(Some));
    {
        let mut state = locked(&inner.state);
        inner.queue_kill_still(&mut state, still.clone());
        inner.queue_kill_still(&mut state, still.clone());
        assert_eq!(state.kill_stills_pending.len(), 1);
    }
    let mut jobs = inner.take_kill_stills(Instant::now(), 2);
    assert_eq!(jobs.len(), 1);
    let Job::KillShot {
        still: job,
        references,
    } = jobs.remove(0)
    else {
        panic!("kill still job")
    };
    assert_eq!(references, keys);
    let before = fixture.calls();
    let (bytes, path) = inner
        .render_kill_still(&job.request, &references)
        .await
        .unwrap();
    assert!(!bytes.is_empty());
    assert!(path.starts_with("/gameplay/imagery/"));
    assert_eq!(fixture.calls(), before + 1);
    assert_eq!(fixture.image_count(), 3);
    assert!(fixture.prompt().contains("named Sigrun"));
    assert!(fixture.prompt().contains("named Bandit"));
    assert!(
        fixture
            .prompt()
            .contains("Image 3 is the canonical photograph")
    );
    inner.queue_kill_still(&mut locked(&inner.state), still);
    assert!(
        inner.take_kill_stills(Instant::now(), 2).is_empty(),
        "completed blow is never requested again"
    );
}

#[test]
fn kill_shot_waits_for_canonical_references_without_spawning_or_retrying_a_failed_blow() {
    let (inner, _) = director(&dossier());
    let still = kill_shot_still([None, None, None]);
    inner.queue_kill_still(&mut locked(&inner.state), still.clone());
    assert!(inner.take_kill_stills(Instant::now(), 2).is_empty());
    assert_eq!(locked(&inner.state).kill_stills_pending.len(), 1);
    assert!(
        inner
            .take_kill_stills(Instant::now() + KILL_REFERENCE_DEADLINE, 2)
            .is_empty()
    );
    assert!(locked(&inner.state).kill_stills_pending.is_empty());
    inner.queue_kill_still(&mut locked(&inner.state), still);
    assert!(locked(&inner.state).kill_stills_pending.is_empty());
}

#[test]
fn kill_shot_rejected_committed_blows_never_queue_still_work() {
    let (inner, _) = director(&dossier());
    let outcome = rpc::CombatOutcome {
        actor_name: "Sigrun".into(),
        target_name: "Bandit".into(),
        weapon: "greataxe".into(),
        damage_type: "slashing".into(),
        hit: true,
        damage: 15,
        target_hit_points: 0,
        sequence: 81,
        kind: rpc::OutcomeKind::Attack as i32,
        ..Default::default()
    };
    let mut knock_out = outcome.clone();
    knock_out.knocked_out = true;
    let mut miss = outcome.clone();
    miss.hit = false;
    miss.damage = 0;
    for (outcome, hero_dead) in [(knock_out, None), (outcome, Some(false)), (miss, None)] {
        let blow = kill_shot::committed_blow(&outcome, "Market street", hero_dead);
        if let Ok(request) = kill_shot::decide(&blow, 3) {
            let mut still = kill_shot_still([Some(key('a')), Some(key('b')), Some(key('c'))]);
            still.request = request;
            inner.queue_kill_still(&mut locked(&inner.state), still);
        }
    }
    assert!(inner.take_kill_stills(Instant::now(), 2).is_empty());
    assert!(locked(&inner.state).kill_stills_pending.is_empty());
}
