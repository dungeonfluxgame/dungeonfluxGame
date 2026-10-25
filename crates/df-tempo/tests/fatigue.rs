mod tempo_fixture;

use std::time::Duration;

use df_model::checkpoint::{Checkpoint, CheckpointError, ContentReference, ExecutionMode};
use df_tempo::elapsed::*;
use df_tempo::fatigue::*;
use tempo_fixture::*;

fn mode_policy(checkpoint: &Checkpoint) -> FatigueModePolicy {
    FatigueModePolicy {
        definition: checkpoint.state().tempo.policy.clone(),
        effects: checkpoint
            .state()
            .tempo
            .fatigue
            .iter()
            .map(|(definition, _)| EffectFatigueRule {
                definition: definition.clone(),
                maximum_counter_units: 100,
                recovery_units_per_presentation_tick: 2,
            })
            .collect(),
        normal_motion: MotionAllowance {
            screen_shake: true,
            forced_camera_punch: true,
            readable_emphasis: true,
        },
    }
}
fn preference(checkpoint: &Checkpoint, reduced_motion: bool) -> MotionPreference {
    MotionPreference {
        expected_basis: checkpoint.basis(),
        reduced_motion,
    }
}
fn advance(
    checkpoint: &Checkpoint,
    elapsed: Duration,
    paused: bool,
    reduced_motion: bool,
) -> Result<FatigueAdvance, FatigueAdvanceError> {
    advance_fatigue_modes(
        checkpoint,
        &pins(),
        request(checkpoint, elapsed, paused),
        Some(&policy(checkpoint)),
        limits(),
        Some(&mode_policy(checkpoint)),
        Some(preference(checkpoint, reduced_motion)),
    )
}

#[test]
fn real_elapsed_owner_recovers_fatigue_preserving_gameplay_and_readable_low_motion() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let before = checkpoint.clone();
    let first = advance(&checkpoint, Duration::from_secs(2), false, true).unwrap();
    let repeated = advance(&checkpoint, Duration::from_secs(2), false, true).unwrap();
    assert_eq!(first, repeated);
    assert_eq!(first.state.presentation_ticks, 60);
    assert_eq!(first.elapsed_ticks, 20);
    assert_eq!(first.state.fatigue, vec![(content(), 30)]);
    assert_eq!(first.state.intensity, checkpoint.state().tempo.intensity);
    assert_eq!(first.state.inertia, checkpoint.state().tempo.inertia);
    assert_eq!(first.execution_mode, ExecutionMode::Live);
    assert!(!first.motion.screen_shake);
    assert!(!first.motion.forced_camera_punch);
    assert!(first.motion.readable_emphasis);
    assert_eq!(checkpoint, before);
    let mut staged_state = checkpoint.state().clone();
    staged_state.tempo = first.state.clone();
    let staged = checkpoint_from(staged_state);
    assert_eq!(staged.state().logical_time, checkpoint.state().logical_time);
    assert_eq!(staged.state().activity, checkpoint.state().activity);
    assert_eq!(staged.state().facts, checkpoint.state().facts);
    assert_eq!(staged.state().decisions, checkpoint.state().decisions);
    assert_eq!(staged.state().timers, checkpoint.state().timers);
    assert!(!checkpoint.state().activity.is_empty());
    assert_eq!(checkpoint.state().logical_time, before.state().logical_time);
    assert_eq!(checkpoint.state().activity, before.state().activity);
    assert_eq!(checkpoint.state().facts, before.state().facts);
    assert_eq!(checkpoint.state().decisions, before.state().decisions);
}

#[test]
fn paused_campaign_preserves_exact_fatigue_and_presentation_anchor() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let result = advance(&checkpoint, Duration::MAX, true, true).unwrap();
    assert_eq!(result.disposition, ElapsedDisposition::Paused);
    assert_eq!(result.state, checkpoint.state().tempo);
    assert_eq!(result.elapsed_ticks, 0);
    assert!(!result.motion.screen_shake);
}

#[test]
fn replay_preserves_recorded_counters_and_never_replays_motion_impacts() {
    let checkpoint = checkpoint_for(ExecutionMode::Replay, vec![(content(), 70)], 40);
    let result = advance(&checkpoint, Duration::MAX, false, false).unwrap();
    assert_eq!(result.execution_mode, ExecutionMode::Replay);
    assert_eq!(result.disposition, ElapsedDisposition::Replay);
    assert_eq!(result.state, checkpoint.state().tempo);
    assert!(!result.motion.screen_shake);
    assert!(!result.motion.forced_camera_punch);
    assert!(result.motion.readable_emphasis);
}

#[test]
fn prepared_only_changes_staged_counters_without_switching_provider_execution_mode() {
    let checkpoint = checkpoint_for(ExecutionMode::PreparedOnly, vec![(content(), 70)], 40);
    let result = advance(&checkpoint, Duration::from_secs(1), false, false).unwrap();
    assert_eq!(result.state.fatigue, vec![(content(), 50)]);
    assert_eq!(result.execution_mode, ExecutionMode::PreparedOnly);
    assert_eq!(checkpoint.state().mode, ExecutionMode::PreparedOnly);
}

#[test]
fn zero_elapsed_never_changes_fatigue_and_recovery_stops_at_zero() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 5)], 40);
    let unchanged = advance(&checkpoint, Duration::ZERO, false, false).unwrap();
    assert_eq!(unchanged.disposition, ElapsedDisposition::Unchanged);
    assert_eq!(unchanged.state, checkpoint.state().tempo);
    let recovered = advance(&checkpoint, Duration::from_secs(1), false, false).unwrap();
    assert_eq!(recovered.state.fatigue, vec![(content(), 0)]);
}

#[test]
fn policy_supplied_motion_ceiling_is_preserved_for_ordinary_preferences() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![], 40);
    let mut authored = mode_policy(&checkpoint);
    authored.normal_motion.screen_shake = false;
    let result = advance_fatigue_modes(
        &checkpoint,
        &pins(),
        request(&checkpoint, Duration::from_secs(1), false),
        Some(&policy(&checkpoint)),
        limits(),
        Some(&authored),
        Some(preference(&checkpoint, false)),
    )
    .unwrap();
    assert_eq!(result.motion, authored.normal_motion);
}

#[test]
fn absent_policy_or_current_preferences_refuses_without_mutating_canonical_state() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let before = checkpoint.clone();
    for (supplied_policy, supplied_preference, expected) in [
        (
            None,
            Some(preference(&checkpoint, true)),
            FatigueAdvanceError::PolicyUnavailable,
        ),
        (
            Some(mode_policy(&checkpoint)),
            None,
            FatigueAdvanceError::PreferenceUnavailable,
        ),
    ] {
        let result = advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::from_secs(1), false),
            Some(&policy(&checkpoint)),
            limits(),
            supplied_policy.as_ref(),
            supplied_preference,
        );
        assert_eq!(result, Err(expected));
    }
    assert_eq!(checkpoint, before);
}

#[test]
fn stale_preference_and_stale_checkpoint_requests_never_stage_motion_or_recovery() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut old_preference = preference(&checkpoint, true);
    old_preference.expected_basis.revision = revision(7);
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::from_secs(1), false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&mode_policy(&checkpoint)),
            Some(old_preference),
        ),
        Err(FatigueAdvanceError::StalePreference)
    );
    let mut stale = request(&checkpoint, Duration::from_secs(1), false);
    stale.expected_basis.revision = revision(7);
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            stale,
            Some(&policy(&checkpoint)),
            limits(),
            Some(&mode_policy(&checkpoint)),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::Elapsed(TempoAdvanceError::Binding(
            CheckpointError::StaleBasis
        )))
    );
}

#[test]
fn repeated_proposal_against_advanced_anchor_refuses_old_elapsed_request() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let first = advance(&checkpoint, Duration::from_secs(1), false, true).unwrap();
    let mut staged_state = checkpoint.state().clone();
    staged_state.tempo = first.state;
    let staged = checkpoint_from(staged_state);
    assert_eq!(
        advance_fatigue_modes(
            &staged,
            &pins(),
            request(&checkpoint, Duration::from_secs(1), false),
            Some(&policy(&staged)),
            limits(),
            Some(&mode_policy(&staged)),
            Some(preference(&staged, true)),
        ),
        Err(FatigueAdvanceError::Elapsed(
            TempoAdvanceError::StalePresentationAnchor
        ))
    );
}

#[test]
fn overflow_and_counter_bounds_refuse_instead_of_silently_clipping() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut supplied = mode_policy(&checkpoint);
    supplied.effects[0].recovery_units_per_presentation_tick = u64::MAX;
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::from_secs(1), false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&supplied),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::RecoveryOverflow)
    );
    supplied.effects[0].maximum_counter_units = 69;
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::ZERO, false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&supplied),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::CounterLimit)
    );
}

#[test]
fn missing_extra_or_duplicate_effect_rules_refuse_ambiguous_counter_semantics() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut missing = mode_policy(&checkpoint);
    missing.effects.clear();
    let mut duplicate = mode_policy(&checkpoint);
    duplicate.effects.push(duplicate.effects[0].clone());
    let mut extra = mode_policy(&checkpoint);
    extra.effects.push(EffectFatigueRule {
        definition: ContentReference {
            package: content().package,
            entry: label("extra"),
        },
        maximum_counter_units: 100,
        recovery_units_per_presentation_tick: 1,
    });
    for (supplied, expected) in [
        (missing, FatigueAdvanceError::MissingEffectRule),
        (duplicate, FatigueAdvanceError::DuplicateRule),
        (extra, FatigueAdvanceError::UnexpectedEffectRule),
    ] {
        assert_eq!(
            advance_fatigue_modes(
                &checkpoint,
                &pins(),
                request(&checkpoint, Duration::from_secs(1), false),
                Some(&policy(&checkpoint)),
                limits(),
                Some(&supplied),
                Some(preference(&checkpoint, true)),
            ),
            Err(expected)
        );
    }
}

#[test]
fn canonical_duplicate_fatigue_and_oversized_policy_are_typed_refusals() {
    let duplicate = checkpoint_for(
        ExecutionMode::Live,
        vec![(content(), 70), (content(), 20)],
        40,
    );
    assert_eq!(
        advance(&duplicate, Duration::ZERO, false, true),
        Err(FatigueAdvanceError::DuplicateFatigue)
    );
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut bounded = limits();
    bounded.maximum_fatigue_entries = 0;
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::ZERO, false),
            Some(&policy(&checkpoint)),
            bounded,
            Some(&mode_policy(&checkpoint)),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::Capacity)
    );
}

#[test]
fn accessibility_toggle_preserves_counter_result_and_never_infers_boredom_from_activity() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let ordinary = advance(&checkpoint, Duration::from_secs(1), false, false).unwrap();
    let reduced_motion = advance(&checkpoint, Duration::from_secs(1), false, true).unwrap();
    assert_eq!(ordinary.state, reduced_motion.state);
    assert_eq!(ordinary.elapsed_ticks, reduced_motion.elapsed_ticks);
    assert!(ordinary.motion.screen_shake);
    assert!(!reduced_motion.motion.screen_shake);
    assert!(!checkpoint.state().activity[0].spotlight_opt_in);
}

#[test]
fn policy_cannot_disable_equivalent_readability() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![], 40);
    let mut supplied = mode_policy(&checkpoint);
    supplied.normal_motion.readable_emphasis = false;
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::ZERO, false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&supplied),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::InvalidMotionPolicy)
    );
}

#[test]
fn mismatched_content_and_build_pins_refuse_current_looking_requests() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut wrong_policy = mode_policy(&checkpoint);
    wrong_policy.definition.entry = label("other-policy");
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &pins(),
            request(&checkpoint, Duration::ZERO, false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&wrong_policy),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::PolicyMismatch)
    );
    let mut wrong_pins = pins();
    wrong_pins.content.package_digest = df_model::checkpoint::ContentDigest([99; 32]);
    assert_eq!(
        advance_fatigue_modes(
            &checkpoint,
            &wrong_pins,
            request(&checkpoint, Duration::ZERO, false),
            Some(&policy(&checkpoint)),
            limits(),
            Some(&mode_policy(&checkpoint)),
            Some(preference(&checkpoint, true)),
        ),
        Err(FatigueAdvanceError::Elapsed(TempoAdvanceError::Binding(
            CheckpointError::ContentMismatch
        )))
    );
}

#[test]
fn exact_elapsed_errors_propagate_without_advancing_counters() {
    let checkpoint = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    assert_eq!(
        advance(&checkpoint, Duration::from_nanos(1), false, true),
        Err(FatigueAdvanceError::Elapsed(
            TempoAdvanceError::FractionalTicks
        ))
    );
    assert_eq!(
        advance(&checkpoint, Duration::from_secs(11), false, true),
        Err(FatigueAdvanceError::Elapsed(
            TempoAdvanceError::ElapsedLimit
        ))
    );
    let near_overflow = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], u64::MAX);
    assert_eq!(
        advance(&near_overflow, Duration::from_secs(1), false, true),
        Err(FatigueAdvanceError::Elapsed(
            TempoAdvanceError::TimeOverflow
        ))
    );
}

#[test]
fn activity_observations_never_become_automatic_fatigue_or_motion_policy() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let mut other_state = current.state().clone();
    other_state.activity.clear();
    let other = checkpoint_from(other_state);
    assert_eq!(
        advance(&current, Duration::from_secs(1), false, true),
        advance(&other, Duration::from_secs(1), false, true)
    );
}

fn director_limits() -> df_engine::director_staging::DirectorLimits {
    df_engine::director_staging::DirectorLimits {
        maximum_checkpoint_bytes: 1024 * 1024,
        maximum_pass_bytes: 5 * 1024 * 1024,
        maximum_relationships: 10,
        world: df_world::DueSelectionLimits {
            queue_events: 10,
            selected_events: 10,
            output_bytes: 1024 * 1024,
        },
    }
}

#[test]
fn real_engine_director_staging_then_fatigue_uses_current_elapsed_and_keeps_gameplay_exact() {
    use df_engine::director_staging::{
        DirectorCandidates, DirectorStaging, compose_director_candidates,
    };

    let current = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let definition = content();
    let staged = compose_director_candidates(
        &current,
        current.pins(),
        df_world::DueSelectionRequest {
            expected_basis: current.basis(),
            target_time: current.state().logical_time,
            paused: false,
            deadline_remaining: Duration::from_secs(1),
            policy: &definition,
        },
        DirectorCandidates {
            interaction: None,
            narrative: None,
        },
        director_limits(),
    )
    .unwrap();
    let DirectorStaging::Staged(staged) = staged else {
        panic!("an empty world reaches downstream tempo staging")
    };
    assert_eq!(staged.basis(), current.basis());
    assert_eq!(staged.pins(), current.pins());
    assert!(staged.world().events.is_empty());
    let selected = staged.candidate();
    let fatigue = advance_fatigue_modes(
        selected,
        selected.pins(),
        request(selected, Duration::from_secs(1), false),
        Some(&policy(selected)),
        limits(),
        Some(&mode_policy(selected)),
        Some(preference(selected, true)),
    )
    .unwrap();
    assert_eq!(fatigue.disposition, ElapsedDisposition::Advanced);
    assert_eq!(fatigue.elapsed_ticks, 10);
    assert!(!fatigue.motion.screen_shake);
    assert!(!fatigue.motion.forced_camera_punch);
    assert!(fatigue.motion.readable_emphasis);
    let mut state = selected.state().clone();
    state.tempo = fatigue.state;
    let candidate = checkpoint_from(state);
    assert_eq!(candidate.state().tempo.presentation_ticks, 50);
    assert_eq!(candidate.state().tempo.fatigue, vec![(content(), 50)]);
    assert_eq!(candidate.basis(), current.basis());
    assert_eq!(candidate.pins(), current.pins());
    let mut restored = candidate.state().clone();
    restored.tempo = current.state().tempo.clone();
    assert_eq!(restored, *current.state());
    assert_eq!(candidate.state().activity, current.state().activity);
    assert_eq!(candidate.state().logical_time, current.state().logical_time);
    assert_eq!(candidate.state().timers, current.state().timers);
    assert_eq!(candidate.state().decisions, current.state().decisions);
}

#[test]
fn real_engine_paused_schedule_staging_keeps_fatigue_and_readable_accessibility_fixed() {
    use df_engine::director_staging::{
        DirectorCandidates, DirectorStaging, EnvironmentalRequest, ScheduleDirectorLimits,
        ScheduleDirectorRequest, compose_schedule_candidates,
    };
    use df_model::checkpoint::ReferenceInventory;

    let current = checkpoint_for(ExecutionMode::Live, vec![(content(), 70)], 40);
    let definition = content();
    let contents = vec![definition.clone()];
    let directors = director_limits();
    let staged = compose_schedule_candidates(
        &current,
        current.pins(),
        ScheduleDirectorRequest {
            accepted_time: Some(df_world::DueSelectionRequest {
                expected_basis: current.basis(),
                target_time: current.state().logical_time,
                paused: true,
                deadline_remaining: Duration::from_secs(1),
                policy: &definition,
            }),
            destinations: &[],
            environmental: EnvironmentalRequest::NotApplicable,
        },
        DirectorCandidates {
            interaction: None,
            narrative: None,
        },
        ReferenceInventory {
            rules: &[],
            content: &contents,
            resources: &[],
            assets: &[],
        },
        ScheduleDirectorLimits {
            directors,
            schedule: df_world::ScheduleAdvancementLimits {
                selection: directors.world,
                entities: 10,
                destinations: 10,
                movements: 10,
                output_bytes: 1024 * 1024,
            },
            checkpoint: checkpoint_limits(),
        },
    )
    .unwrap();
    let DirectorStaging::Staged(staged) = staged else {
        panic!("an empty paused world reaches downstream tempo staging")
    };
    let selected = staged.candidate();
    let fatigue = advance_fatigue_modes(
        selected,
        selected.pins(),
        request(selected, Duration::MAX, true),
        Some(&policy(selected)),
        limits(),
        Some(&mode_policy(selected)),
        Some(preference(selected, true)),
    )
    .unwrap();
    assert_eq!(fatigue.disposition, ElapsedDisposition::Paused);
    assert_eq!(fatigue.elapsed_ticks, 0);
    assert_eq!(fatigue.state, current.state().tempo);
    assert_eq!(fatigue.execution_mode, ExecutionMode::Live);
    assert!(!fatigue.motion.screen_shake);
    assert!(!fatigue.motion.forced_camera_punch);
    assert!(fatigue.motion.readable_emphasis);
    let mut state = selected.state().clone();
    state.tempo = fatigue.state;
    let candidate = checkpoint_from(state);
    assert_eq!(candidate, *selected);
    assert_eq!(candidate.state().activity, current.state().activity);
    assert_eq!(candidate.state().logical_time, current.state().logical_time);
    assert_eq!(candidate.state().timers, current.state().timers);
    assert_eq!(candidate.state().decisions, current.state().decisions);
}
