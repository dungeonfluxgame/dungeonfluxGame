use std::time::Duration;

use super::*;

fn seconds(count: u64) -> Duration {
    Duration::from_secs(count)
}

fn signal(
    sequence: u64,
    approach: Approach,
    effect: Effect,
    attitude: Attitude,
    opposes: bool,
) -> ExchangeSignal {
    ExchangeSignal {
        sequence,
        npc_id: "merchant".to_owned(),
        approach,
        effect,
        attitude,
        opposes,
        emotional_state: EmotionalState::default(),
    }
}

fn member(speaking: bool, opposes: bool, tier: HealthTier) -> CastState {
    CastState {
        id: "merchant".to_owned(),
        attitude: Some(Attitude::Indifferent),
        opposes,
        speaking,
        tier,
    }
}

fn inputs<'a>(
    cast: &'a [CastState],
    exchange: Option<&'a ExchangeSignal>,
    tempo: Option<Tempo>,
    recent: &'a [TableKind],
) -> MoodInputs<'a> {
    MoodInputs {
        tempo,
        tone: SceneTone::Neutral,
        beat: Beat::Conversation,
        cast,
        exchange,
        recent_kinds: recent,
    }
}

fn tempo(phase: TempoLevel) -> Option<Tempo> {
    Some(Tempo {
        phase,
        intensity_permille: 500,
    })
}

/// The mood and expression one exchange gives, with no earlier approaches.
fn reading(
    approach: Approach,
    effect: Effect,
    attitude: Attitude,
    opposes: bool,
) -> (Mood, Expression) {
    exchange_reading(&signal(1, approach, effect, attitude, opposes), &[])
}

#[test]
fn an_insult_makes_the_speaker_angry_and_a_friend_pained() {
    for attitude in [Attitude::Hostile, Attitude::Indifferent] {
        let (_, expression) = reading(Approach::Insult, Effect::Cool, attitude, false);
        assert_eq!(expression, Expression::Angry, "{attitude:?}");
    }
    let (mood, expression) = reading(Approach::Insult, Effect::Cool, Attitude::Friendly, false);
    assert_eq!(expression, Expression::Pained);
    assert_eq!(mood, Mood::Tense);
    let (mood, _) = reading(Approach::Insult, Effect::Cool, Attitude::Hostile, true);
    assert_eq!(mood, Mood::Menacing);
}

#[test]
fn the_characters_persistent_emotion_drives_the_expression_before_the_line_approach() {
    let mut signal = signal(
        1,
        Approach::Question,
        Effect::Steady,
        Attitude::Friendly,
        false,
    );
    signal.emotional_state.afraid = EmotionLevel::Much;
    assert_eq!(
        exchange_reading(&signal, &[]),
        (Mood::Fearful, Expression::Afraid)
    );

    signal.emotional_state.afraid = EmotionLevel::None;
    signal.emotional_state.drawn_to = EmotionLevel::ALittle;
    assert_eq!(
        exchange_reading(&signal, &[]),
        (Mood::Flirtatious, Expression::Flirtatious)
    );
}

#[test]
fn intimidation_frightens_or_enrages_by_the_characters_temperament() {
    assert_eq!(
        reading(
            Approach::Intimidation,
            Effect::Warm,
            Attitude::Indifferent,
            false
        ),
        (Mood::Fearful, Expression::Afraid)
    );
    assert_eq!(
        reading(
            Approach::Intimidation,
            Effect::Cool,
            Attitude::Hostile,
            true
        ),
        (Mood::Menacing, Expression::Angry)
    );
    assert_eq!(
        reading(
            Approach::Intimidation,
            Effect::Steady,
            Attitude::Indifferent,
            false
        )
        .1,
        Expression::Afraid
    );
    assert_eq!(
        reading(
            Approach::Intimidation,
            Effect::Steady,
            Attitude::Hostile,
            true
        )
        .1,
        Expression::Contemptuous
    );
}

#[test]
fn a_second_intimidation_turns_the_scene_brutal() {
    let again = exchange_reading(
        &signal(
            2,
            Approach::Intimidation,
            Effect::Warm,
            Attitude::Indifferent,
            false,
        ),
        &[Approach::Intimidation],
    );
    assert_eq!(again.0, Mood::Brutal);
    let after_other = exchange_reading(
        &signal(
            2,
            Approach::Intimidation,
            Effect::Warm,
            Attitude::Indifferent,
            false,
        ),
        &[Approach::Question],
    );
    assert_eq!(after_other.0, Mood::Fearful);
}

#[test]
fn flirting_charms_or_amuses_or_repels_by_the_characters_temperament() {
    assert_eq!(
        reading(Approach::Flirting, Effect::Warm, Attitude::Friendly, false),
        (Mood::Flirtatious, Expression::Flirtatious)
    );
    assert_eq!(
        reading(
            Approach::Flirting,
            Effect::Warm,
            Attitude::Indifferent,
            false
        ),
        (Mood::Playful, Expression::Amused)
    );
    assert_eq!(
        reading(
            Approach::Flirting,
            Effect::Steady,
            Attitude::Indifferent,
            false
        ),
        (Mood::Playful, Expression::Amused)
    );
    assert_eq!(
        reading(Approach::Flirting, Effect::Cool, Attitude::Hostile, true),
        (Mood::Tense, Expression::Contemptuous)
    );
    let again = exchange_reading(
        &signal(
            2,
            Approach::Flirting,
            Effect::Warm,
            Attitude::Friendly,
            false,
        ),
        &[Approach::Flirting],
    );
    assert_eq!(again, (Mood::Seductive, Expression::Seductive));
}

#[test]
fn kindness_warms_the_scene_and_smiles_unless_the_character_is_hostile() {
    assert_eq!(
        reading(
            Approach::Kindness,
            Effect::Warm,
            Attitude::Indifferent,
            false
        ),
        (Mood::Warm, Expression::Smiling)
    );
    assert_eq!(
        reading(Approach::Kindness, Effect::Warm, Attitude::Hostile, true),
        (Mood::Warm, Expression::Neutral)
    );
}

#[test]
fn flattery_bribery_and_deception_read_by_temperament() {
    assert_eq!(
        reading(Approach::Flattery, Effect::Warm, Attitude::Friendly, false),
        (Mood::Warm, Expression::Smiling)
    );
    assert_eq!(
        reading(
            Approach::Flattery,
            Effect::Cool,
            Attitude::Indifferent,
            false
        )
        .1,
        Expression::Contemptuous
    );
    assert_eq!(
        reading(Approach::Bribe, Effect::Cool, Attitude::Hostile, true).1,
        Expression::Angry
    );
    assert_eq!(
        reading(
            Approach::Deception,
            Effect::Cool,
            Attitude::Indifferent,
            false
        )
        .1,
        Expression::Contemptuous
    );
    assert_eq!(
        reading(
            Approach::Question,
            Effect::Steady,
            Attitude::Friendly,
            false
        ),
        (Mood::Calm, Expression::Smiling)
    );
}

#[test]
fn every_mood_has_a_look_and_every_expression_a_label() {
    for mood in Mood::ALL {
        let look = Look::of(mood);
        assert_eq!(look.mood, mood);
        assert!(!mood.label().is_empty());
    }
    for expression in Expression::ALL {
        assert!(!expression.label().is_empty());
    }
    assert_eq!(Look::of(Mood::Calm), Look::AUTHORED);
}

#[test]
fn the_same_inputs_and_clock_give_the_same_verdict() {
    let cast = [member(true, false, HealthTier::Healthy)];
    let exchange = signal(
        1,
        Approach::Insult,
        Effect::Cool,
        Attitude::Indifferent,
        false,
    );
    let run = || {
        let mut tracker = MoodTracker::new(Hysteresis::default());
        (0..5)
            .map(|step| {
                tracker.update(
                    seconds(step * 3),
                    &inputs(&cast, Some(&exchange), tempo(TempoLevel::Build), &[]),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn an_exchange_changes_the_look_at_once_and_is_handled_only_once() {
    let cast = [member(true, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let calm = tracker.update(seconds(0), &inputs(&cast, None, None, &[]));
    assert!(!calm.scene_worth_rendering);
    assert!(calm.face_changes.is_empty());
    let insult = signal(
        1,
        Approach::Insult,
        Effect::Cool,
        Attitude::Indifferent,
        false,
    );
    let verdict = tracker.update(seconds(1), &inputs(&cast, Some(&insult), None, &[]));
    assert_eq!(verdict.look, Look::of(Mood::Tense));
    assert!(verdict.scene_worth_rendering);
    assert_eq!(
        verdict.face_changes,
        vec![(
            "merchant".to_owned(),
            Face {
                expression: Expression::Angry,
                tier: HealthTier::Healthy
            }
        )]
    );
    // The same recorded exchange is not read again after the display shows it.
    tracker.mark_scene_shown(verdict.look);
    tracker.mark_face_shown("merchant", verdict.face_changes[0].1);
    let again = tracker.update(seconds(2), &inputs(&cast, Some(&insult), None, &[]));
    assert!(!again.scene_worth_rendering);
    assert!(again.face_changes.is_empty());
}

#[test]
fn an_ambient_change_must_persist_before_it_becomes_the_target() {
    let cast = [member(false, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let tense = tempo(TempoLevel::Anticipation);
    let first = tracker.update(seconds(0), &inputs(&cast, None, tense, &[]));
    assert_eq!(first.look.mood, Mood::Calm, "one observation is not enough");
    let early = tracker.update(seconds(9), &inputs(&cast, None, tense, &[]));
    assert_eq!(early.look.mood, Mood::Calm);
    let settled = tracker.update(seconds(10), &inputs(&cast, None, tense, &[]));
    assert_eq!(settled.look.mood, Mood::Tense);
    assert!(settled.scene_worth_rendering);
}

#[test]
fn a_flickering_input_never_flaps_the_look() {
    let cast = [member(false, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    for step in 0..60 {
        let phase = if step % 2 == 0 {
            TempoLevel::Anticipation
        } else {
            TempoLevel::Calm
        };
        let verdict = tracker.update(seconds(step), &inputs(&cast, None, tempo(phase), &[]));
        assert_eq!(verdict.look, Look::AUTHORED, "step {step}");
    }
}

#[test]
fn an_exchange_look_is_held_against_tempo_drift_then_released() {
    let cast = [member(true, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let flirt = signal(
        1,
        Approach::Flirting,
        Effect::Warm,
        Attitude::Friendly,
        false,
    );
    tracker.update(
        seconds(0),
        &inputs(&cast, Some(&flirt), tempo(TempoLevel::Calm), &[]),
    );
    let held = tracker.update(
        seconds(44),
        &inputs(&cast, Some(&flirt), tempo(TempoLevel::Peak), &[]),
    );
    assert_eq!(held.look.mood, Mood::Flirtatious);
    // After the hold the ambient mood must still persist for the settle time.
    let released = tracker.update(
        seconds(45),
        &inputs(&cast, Some(&flirt), tempo(TempoLevel::Peak), &[]),
    );
    assert_eq!(released.look.mood, Mood::Flirtatious);
    let settled = tracker.update(
        seconds(55),
        &inputs(&cast, Some(&flirt), tempo(TempoLevel::Peak), &[]),
    );
    assert_eq!(settled.look.mood, Mood::Tense);
}

#[test]
fn an_expression_relaxes_to_the_authored_face_after_the_hold() {
    let cast = [member(true, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let insult = signal(
        1,
        Approach::Insult,
        Effect::Cool,
        Attitude::Indifferent,
        false,
    );
    let verdict = tracker.update(seconds(0), &inputs(&cast, Some(&insult), None, &[]));
    tracker.mark_face_shown("merchant", verdict.face_changes[0].1);
    let held = tracker.update(seconds(89), &inputs(&cast, Some(&insult), None, &[]));
    assert!(held.face_changes.is_empty());
    let relaxed = tracker.update(seconds(90), &inputs(&cast, Some(&insult), None, &[]));
    assert_eq!(
        relaxed.face_changes,
        vec![("merchant".to_owned(), Face::AUTHORED)]
    );
}

#[test]
fn only_a_difference_from_what_is_on_screen_is_worth_a_rerender() {
    let cast = [member(false, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let flirt = signal(
        1,
        Approach::Flirting,
        Effect::Warm,
        Attitude::Friendly,
        false,
    );
    let verdict = tracker.update(seconds(0), &inputs(&cast, Some(&flirt), None, &[]));
    assert!(verdict.scene_worth_rendering);
    tracker.mark_scene_shown(verdict.look);
    let same = tracker.update(seconds(1), &inputs(&cast, Some(&flirt), None, &[]));
    assert!(!same.scene_worth_rendering);
    assert_eq!(tracker.shown_look(), Look::of(Mood::Flirtatious));
    // A lighting-only difference does not repaint: the lighting follows the mood.
    assert!(!Look::of(Mood::Playful).differs_from(Look {
        lighting: Lighting::Flicker,
        ..Look::of(Mood::Playful)
    }));
}

#[test]
fn a_new_scene_starts_from_the_authored_look_and_replays_no_exchange() {
    let cast = [member(true, false, HealthTier::Healthy)];
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let insult = signal(
        1,
        Approach::Insult,
        Effect::Cool,
        Attitude::Indifferent,
        false,
    );
    tracker.update(seconds(0), &inputs(&cast, Some(&insult), None, &[]));
    tracker.reset_scene();
    let fresh = tracker.update(seconds(1), &inputs(&cast, Some(&insult), None, &[]));
    assert_eq!(fresh.look, Look::AUTHORED);
    assert!(fresh.face_changes.is_empty());
    assert_eq!(tracker.handled_sequence(), 1);
}

#[test]
fn recent_actions_and_the_setting_colour_the_ambient_mood() {
    let cast = [member(false, false, HealthTier::Healthy)];
    let actions = [
        TableKind::Action,
        TableKind::Speech,
        TableKind::Action,
        TableKind::Action,
    ];
    let busy = inputs(&cast, None, tempo(TempoLevel::Build), &actions);
    assert_eq!(ambient_mood(&busy), Mood::Tense);
    let quiet = inputs(&cast, None, tempo(TempoLevel::Build), &[TableKind::Speech]);
    assert_eq!(ambient_mood(&quiet), Mood::Calm);
    let grim = MoodInputs {
        tone: SceneTone::Grim,
        ..inputs(&cast, None, None, &[])
    };
    assert_eq!(ambient_mood(&grim), Mood::Grim);
    let cozy = MoodInputs {
        tone: SceneTone::Cozy,
        ..inputs(&cast, None, None, &[])
    };
    assert_eq!(ambient_mood(&cozy), Mood::Warm);
}

#[test]
fn a_peak_with_an_enemy_speaking_is_menacing_and_the_story_beats_set_the_end_mood() {
    let enemy = [member(true, true, HealthTier::Healthy)];
    assert_eq!(
        ambient_mood(&inputs(&enemy, None, tempo(TempoLevel::Peak), &[])),
        Mood::Menacing
    );
    assert_eq!(
        ambient_mood(&inputs(&[], None, tempo(TempoLevel::Release), &[])),
        Mood::Warm
    );
    let victory = MoodInputs {
        beat: Beat::Victory,
        ..inputs(&[], None, None, &[])
    };
    assert_eq!(ambient_mood(&victory), Mood::Triumphant);
    let defeat = MoodInputs {
        beat: Beat::Defeat,
        ..inputs(&[], None, None, &[])
    };
    assert_eq!(ambient_mood(&defeat), Mood::Grim);
}

#[test]
fn bloodied_or_worse_shows_the_health_face_and_above_it_the_mood_face_wins() {
    for tier in [HealthTier::Bloodied, HealthTier::Critical, HealthTier::Down] {
        // Priority, not multiplication: the mood does not combine with the health face.
        for mood in Expression::ALL {
            assert_eq!(
                Face::resolve(mood, tier),
                Face {
                    expression: Expression::Neutral,
                    tier
                },
                "{tier:?} {mood:?}"
            );
        }
    }
    assert_eq!(
        Face::resolve(Expression::Angry, HealthTier::Hurt),
        Face {
            expression: Expression::Angry,
            tier: HealthTier::Healthy
        }
    );
    assert_eq!(
        Face::resolve(Expression::Neutral, HealthTier::Hurt).tier,
        HealthTier::Hurt
    );
    assert_eq!(
        Face::resolve(Expression::Angry, HealthTier::Healthy).expression,
        Expression::Angry
    );
    assert_eq!(
        Face::resolve(Expression::Neutral, HealthTier::Healthy),
        Face::AUTHORED
    );
}

#[test]
fn a_hurt_character_shows_the_health_face_and_the_end_beats_show_relief_or_grief() {
    let mut tracker = MoodTracker::new(Hysteresis::default());
    let bloodied = [member(true, true, HealthTier::Bloodied)];
    let verdict = tracker.update(seconds(0), &inputs(&bloodied, None, None, &[]));
    assert_eq!(
        verdict.face_changes[0].1,
        Face {
            expression: Expression::Neutral,
            tier: HealthTier::Bloodied
        }
    );
    let wounded = [member(true, true, HealthTier::Hurt)];
    let verdict = tracker.update(seconds(1), &inputs(&wounded, None, None, &[]));
    assert_eq!(verdict.face_changes[0].1.tier, HealthTier::Hurt);

    let patron = [member(true, false, HealthTier::Healthy)];
    let mut sorrow = MoodTracker::new(Hysteresis::default());
    let defeat = MoodInputs {
        beat: Beat::Defeat,
        ..inputs(&patron, None, None, &[])
    };
    assert_eq!(
        sorrow.update(seconds(0), &defeat).face_changes[0]
            .1
            .expression,
        Expression::Sorrowful
    );
    let mut relief = MoodTracker::new(Hysteresis::default());
    let victory = MoodInputs {
        beat: Beat::Victory,
        ..inputs(&patron, None, None, &[])
    };
    assert_eq!(
        relief.update(seconds(0), &victory).face_changes[0]
            .1
            .expression,
        Expression::Smiling
    );
    // A character who is not speaking shows nothing at the end beats.
    let silent = [member(false, false, HealthTier::Healthy)];
    let mut quiet = MoodTracker::new(Hysteresis::default());
    let victory = MoodInputs {
        beat: Beat::Victory,
        ..inputs(&silent, None, None, &[])
    };
    assert!(quiet.update(seconds(0), &victory).face_changes.is_empty());
}
