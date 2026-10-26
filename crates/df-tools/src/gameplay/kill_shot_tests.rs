//! K1 checks, with fakes only: no live fal or image calls. The blow is a fake
//! committed outcome, the still is synthetic bytes, and the clip spec is keyed
//! locally. Live provider behaviour of the clip engine is covered by the
//! home-video loopback fake in `home_video_tests`.
use super::super::resources::{ResourceKey, ResourceSpec};
use super::super::rpc;
use super::super::startup_configuration::ContentRating;
use super::clip_spec::{ClipInput, ClipRecipe, KillShotSpec};
use super::kill_shot::{
    CommittedBlow, KillShotRequest, ShownShots, Skip, audit_line, decide, kill_id, motion_prompt,
    sequence_committed_outcomes, still_plan, still_prompt,
};
use crate::gameplay::imagery::NON_PHOTOGRAPHIC_WORDS;

fn blow() -> CommittedBlow {
    CommittedBlow {
        killer: "Sigrun".to_owned(),
        victim: "Brakka".to_owned(),
        weapon: "greataxe".to_owned(),
        damage_type: "slashing".to_owned(),
        place: "Marek's cloth shop".to_owned(),
        sequence: 42,
        hit: true,
        damage: 14,
        target_hit_points: 0,
        victim_dead: true,
        knocked_out: false,
    }
}

fn request() -> KillShotRequest {
    decide(&blow(), 7).expect("a killing blow")
}

fn recipe() -> ClipRecipe {
    ClipRecipe {
        model: "fal-ai/kling-video/o1/image-to-video".to_owned(),
        duration_seconds: 5,
        width: 1920,
        height: 1080,
        recipe: "df-kill-shot/1".to_owned(),
        finish: "as_delivered",
    }
}

#[test]
fn a_killing_blow_requests_one_kill_shot() {
    let request = request();
    assert_eq!(request.killer, "Sigrun");
    assert_eq!(request.victim, "Brakka");
    assert_eq!(request.weapon, "greataxe");
    assert_eq!(request.damage_type, "slashing");
    assert_eq!(request.sequence, 42);
    assert_eq!(request.epoch, 7);
    assert_eq!(request.id, kill_id(&blow()));
    // The same committed blow always gets the same id, so a replay reuses it.
    assert_eq!(decide(&blow(), 7).expect("again").id, request.id);
}

#[test]
fn a_knock_out_gets_no_kill_shot() {
    let mut blow = blow();
    blow.knocked_out = true;
    assert_eq!(decide(&blow, 7), Err(Skip::KnockedOut));
}

#[test]
fn a_downed_hero_at_zero_who_is_not_dead_gets_no_kill_shot() {
    let mut blow = blow();
    blow.victim = "Corin".to_owned();
    blow.victim_dead = false;
    assert_eq!(decide(&blow, 7), Err(Skip::DownedNotDead));
}

#[test]
fn a_miss_no_damage_and_a_survivor_get_no_kill_shot() {
    let mut miss = blow();
    miss.hit = false;
    assert_eq!(decide(&miss, 7), Err(Skip::NotAHit));
    let mut harmless = blow();
    harmless.damage = 0;
    assert_eq!(decide(&harmless, 7), Err(Skip::NoDamage));
    let mut survivor = blow();
    survivor.target_hit_points = 3;
    survivor.victim_dead = false;
    assert_eq!(decide(&survivor, 7), Err(Skip::TargetNotAtZero));
    let mut unnamed = blow();
    unnamed.weapon = "  ".to_owned();
    assert_eq!(decide(&unnamed, 7), Err(Skip::MissingFact));
}

#[test]
fn the_still_prompt_holds_only_the_committed_facts_and_no_medium_words() {
    let prompt = still_prompt(&request(), ContentRating::Mature);
    for fact in [
        "Sigrun",
        "Brakka",
        "greataxe",
        "slashing",
        "Marek's cloth shop",
    ] {
        assert!(prompt.contains(fact), "{fact} missing from {prompt}");
    }
    assert!(prompt.contains("strike is landing now"));
    assert!(prompt.contains("is falling"));
    assert!(prompt.contains("Documentary color photograph"));
    assert!(prompt.contains("ONE single continuous photographic camera frame"));
    assert!(prompt.contains("Mature content rating"));
    for word in NON_PHOTOGRAPHIC_WORDS {
        assert!(!prompt.contains(word), "{word:?} in {prompt}");
    }
    let restrained = still_prompt(&request(), ContentRating::Restrained);
    assert!(restrained.contains("No gore, no nudity, no sexual content."));
    for word in NON_PHOTOGRAPHIC_WORDS {
        assert!(!restrained.contains(word), "{word:?} in {restrained}");
    }
}

#[test]
fn the_still_plan_uses_the_fight_identities_and_the_photo_check_kind() {
    let plan = still_plan(&request(), ContentRating::Mature);
    assert_eq!(
        plan.reference_roles,
        [
            "killer_canonical_photo",
            "victim_canonical_photo",
            "place_canonical_room_photo"
        ]
    );
    assert_eq!(plan.kind_label, "scene_plate");
    assert_eq!((plan.width, plan.height), (2560, 1440));
}

#[test]
fn the_motion_prompt_is_one_real_time_blow_with_a_still_camera() {
    let prompt = motion_prompt(&request());
    for phrase in [
        "Real-time motion",
        "strike lands",
        "falls",
        "Camera nearly still",
        "no cuts",
        "add no people",
    ] {
        assert!(prompt.contains(phrase), "{phrase} missing from {prompt}");
    }
    for word in NON_PHOTOGRAPHIC_WORDS {
        assert!(!prompt.contains(word), "{word:?} in {prompt}");
    }
}

#[test]
fn the_kill_shot_spec_key_covers_still_prompt_blow_and_recipe() {
    let still = b"synthetic still bytes";
    let base = KillShotSpec::new(
        "kill-42-a",
        "Sigrun",
        "Brakka",
        ClipInput::ready(still, "prompt one"),
        recipe(),
    );
    let key = ResourceKey::of(&base);
    assert_eq!(KillShotSpec::CONTRACT_VERSION, 1);
    assert!(base.subject().starts_with("video:kill:"));
    let other_prompt = KillShotSpec::new(
        "kill-42-a",
        "Sigrun",
        "Brakka",
        ClipInput::ready(still, "prompt two"),
        recipe(),
    );
    assert_ne!(ResourceKey::of(&other_prompt), key);
    let other_blow = KillShotSpec::new(
        "kill-43-a",
        "Sigrun",
        "Brakka",
        ClipInput::ready(still, "prompt one"),
        recipe(),
    );
    assert_ne!(ResourceKey::of(&other_blow), key);
    let mut other_recipe = recipe();
    other_recipe.duration_seconds = 10;
    let longer = KillShotSpec::new(
        "kill-42-a",
        "Sigrun",
        "Brakka",
        ClipInput::ready(still, "prompt one"),
        other_recipe,
    );
    assert_ne!(ResourceKey::of(&longer), key);
    assert_eq!(super::clip_spec::file_key(&base).len(), 64);
}

#[test]
fn the_view_field_carries_paths_names_and_state_only_when_ready() {
    let pending =
        super::kill_shot::view_projection(&request(), None, "pending", "/gameplay/imagery/abc");
    assert_eq!(pending.state, rpc::HomeVideoState::Pending as i32);
    assert_eq!(pending.clip_path, "");
    assert_eq!(pending.killer, "Sigrun");
    assert_eq!(pending.victim, "Brakka");
    let ready = super::kill_shot::view_projection(
        &request(),
        Some("/gameplay/home-video/abc".to_owned()),
        "ready",
        "/gameplay/imagery/abc",
    );
    assert_eq!(ready.state, rpc::HomeVideoState::Ready as i32);
    assert_eq!(ready.clip_path, "/gameplay/home-video/abc");
    assert_eq!(ready.still_path, "/gameplay/imagery/abc");
    assert_eq!(ready.id, request().id);
    assert_eq!(ready.epoch, 7);
}

#[test]
fn a_kill_shot_plays_once_per_id_and_epoch() {
    let mut shown = ShownShots::new();
    assert!(shown.claim("kill-42-a", 7));
    assert!(
        !shown.claim("kill-42-a", 7),
        "a replayed view never replays"
    );
    assert!(shown.was_shown("kill-42-a", 7));
    assert!(shown.claim("kill-42-a", 8), "a new epoch is a new showing");
    assert!(
        shown.claim("kill-43-a", 7),
        "a different blow is a new showing"
    );
}

#[test]
fn the_audit_line_names_status_names_elapsed_and_reason() {
    let line = audit_line("requested", "Sigrun", "Brakka", 12, "killing_blow");
    assert_eq!(
        line,
        "KILL_SHOT status=requested killer=Sigrun victim=Brakka elapsed_ms=12 reason=killing_blow"
    );
    let failed = audit_line("failed", "Sigrun", "Brakka", 900, "provider: refused");
    assert!(failed.starts_with("KILL_SHOT status=failed"));
    assert!(failed.contains("elapsed_ms=900"));
}

#[test]
fn kill_shot_committed_outcomes_use_rules_death_for_heroes_and_lethality_for_foes() {
    use super::kill_shot::committed_blow;
    let lethal = rpc::CombatOutcome {
        actor_name: "Sigrun".into(),
        target_name: "Bandit".into(),
        weapon: "greataxe".into(),
        damage_type: "slashing".into(),
        kind: rpc::OutcomeKind::Attack as i32,
        hit: true,
        damage: 15,
        target_hit_points: 0,
        sequence: 81,
        ..Default::default()
    };
    let blow = committed_blow(&lethal, "Market street", None);
    assert!(blow.victim_dead);
    assert_eq!(decide(&blow, 3).unwrap().sequence, 81);
    let downed = committed_blow(&lethal, "Market street", Some(false));
    assert_eq!(decide(&downed, 3), Err(Skip::DownedNotDead));
    let dead_hero = committed_blow(&lethal, "Market street", Some(true));
    assert!(decide(&dead_hero, 3).is_ok());
    let mut knocked_out = lethal.clone();
    knocked_out.knocked_out = true;
    let blow = committed_blow(&knocked_out, "Market street", None);
    assert!(!blow.victim_dead);
    assert_eq!(decide(&blow, 3), Err(Skip::KnockedOut));
    let mut miss = lethal;
    miss.hit = false;
    miss.damage = 0;
    assert_eq!(
        decide(&committed_blow(&miss, "Market street", None), 3),
        Err(Skip::NotAHit)
    );
}

#[test]
fn a_finishing_attack_uses_the_committed_history_sequence_for_its_kill_shot() {
    let mut history = vec![rpc::CombatOutcome::default(); 8];
    let finishing_attack = rpc::CombatOutcome {
        actor_name: "Sigrun".into(),
        target_name: "Jessa".into(),
        weapon: "javelin".into(),
        damage_type: "piercing".into(),
        kind: rpc::OutcomeKind::Attack as i32,
        hit: true,
        damage: 8,
        target_hit_points: 0,
        ..Default::default()
    };
    history.push(finishing_attack.clone());

    let sequenced = sequence_committed_outcomes(&history, &[finishing_attack])
        .expect("the finished encounter retains its committed history");
    assert_eq!(sequenced[0].sequence, 9, "the TV's latest_sequence");
    let blow = super::kill_shot::committed_blow(&sequenced[0], "Harbor", Some(true));
    let request = decide(&blow, 5).expect("the finishing attack is a kill shot");
    assert_eq!(request.sequence, 9);
}

#[test]
fn non_finishing_and_knockout_attacks_keep_their_existing_kill_shot_decisions() {
    let attack = rpc::CombatOutcome {
        actor_name: "Sigrun".into(),
        target_name: "Bandit".into(),
        weapon: "javelin".into(),
        damage_type: "piercing".into(),
        kind: rpc::OutcomeKind::Attack as i32,
        hit: true,
        damage: 8,
        target_hit_points: 0,
        ..Default::default()
    };
    let mut history = vec![rpc::CombatOutcome::default(); 3];
    history.push(attack.clone());
    let sequenced = sequence_committed_outcomes(&history, std::slice::from_ref(&attack))
        .expect("the committed history includes this non-finishing attack");
    assert_eq!(
        decide(
            &super::kill_shot::committed_blow(&sequenced[0], "Harbor", None),
            5
        )
        .expect("non-finishing kill remains eligible")
        .sequence,
        4
    );

    let mut knockout = attack;
    knockout.knocked_out = true;
    let mut finished_history = history;
    finished_history.push(knockout.clone());
    let sequenced = sequence_committed_outcomes(&finished_history, &[knockout])
        .expect("the knockout is committed to the finished history");
    assert_eq!(
        decide(
            &super::kill_shot::committed_blow(&sequenced[0], "Harbor", None),
            5
        ),
        Err(Skip::KnockedOut)
    );
}

#[test]
fn kill_shot_credits_only_the_last_lethal_hero_hit_and_never_a_later_death_save() {
    use super::kill_shot::hero_death_at_blow;
    let first = rpc::CombatOutcome {
        target_id: vec![7; 16],
        actor_id: vec![9; 16],
        hit: true,
        damage: 6,
        target_hit_points: 0,
        kind: rpc::OutcomeKind::Attack as i32,
        ..Default::default()
    };
    let later = first.clone();
    assert!(!hero_death_at_blow(&first, &[later], true));
    assert!(hero_death_at_blow(&first, &[], true));
    assert!(!hero_death_at_blow(&first, &[], false));
    let death_save = rpc::CombatOutcome {
        actor_id: first.target_id.clone(),
        kind: rpc::OutcomeKind::DeathSave as i32,
        death_failures: 3,
        ..Default::default()
    };
    assert!(!hero_death_at_blow(&first, &[death_save], true));
}
