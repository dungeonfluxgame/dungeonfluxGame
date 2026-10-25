#[path = "../src/recovery_policy.rs"]
mod recovery_policy;
mod tempo_fixture;

use df_model::checkpoint::{Checkpoint, CheckpointError, ContentReference, ExecutionMode};
use df_tempo::elapsed::*;
use df_tempo::fatigue::*;
use recovery_policy::*;
use std::time::Duration;
use tempo_fixture::*;

fn effect(entry: &str) -> ContentReference {
    ContentReference {
        entry: label(entry),
        ..content()
    }
}
fn mode_policy(
    current: &Checkpoint,
    maximum_counter_units: u64,
    recovery_units_per_presentation_tick: u64,
) -> FatigueModePolicy {
    FatigueModePolicy {
        definition: current.state().tempo.policy.clone(),
        effects: current
            .state()
            .tempo
            .fatigue
            .iter()
            .map(|(definition, _)| EffectFatigueRule {
                definition: definition.clone(),
                maximum_counter_units,
                recovery_units_per_presentation_tick,
            })
            .collect(),
        normal_motion: MotionAllowance {
            screen_shake: true,
            forced_camera_punch: true,
            readable_emphasis: true,
        },
    }
}
fn advance(
    current: &Checkpoint,
    pressure_units: Option<u64>,
    recovery_units_per_presentation_tick: u64,
    reduced_motion: bool,
) -> Result<RecoveryProposal, RecoveryError> {
    let elapsed = policy(current);
    let fatigue_policy = mode_policy(current, 10, recovery_units_per_presentation_tick);
    advance_recovery(
        current,
        current.pins(),
        RecoveryRequest {
            elapsed: request(current, Duration::from_millis(100), false),
            elapsed_policy: Some(&elapsed),
            fatigue_policy: Some(&fatigue_policy),
            preference: Some(MotionPreference {
                expected_basis: current.basis(),
                reduced_motion,
            }),
            pressure_units,
        },
        limits(),
    )
}
fn staged(current: &Checkpoint, proposal: &RecoveryProposal) -> Checkpoint {
    let mut state = current.state().clone();
    state.tempo = proposal.advance.state.clone();
    checkpoint_from(state)
}

#[test]
fn repeated_intensity_rotates_then_exhausts_and_explicit_recovery_restores_eligibility() {
    // Fixture coefficients are source-supplied presentation units, not calibrated safety caps.
    let original = checkpoint_for(
        ExecutionMode::Live,
        vec![(effect("a"), 0), (effect("b"), 0)],
        40,
    );
    let first = advance(&original, Some(10), 0, false).unwrap();
    assert_eq!(first.rotation, RotationDisposition::Selected(effect("a")));
    let second_current = staged(&original, &first);
    let second = advance(&second_current, Some(10), 0, false).unwrap();
    assert_eq!(second.rotation, RotationDisposition::Selected(effect("b")));
    let exhausted_current = staged(&second_current, &second);
    let exhausted = advance(&exhausted_current, Some(1), 0, false).unwrap();
    assert_eq!(exhausted.rotation, RotationDisposition::Exhausted);
    assert_eq!(
        exhausted.advance.state.fatigue,
        vec![(effect("a"), 10), (effect("b"), 10)]
    );
    let recovery = advance(&exhausted_current, None, 10, false).unwrap();
    assert_eq!(recovery.rotation, RotationDisposition::RecoveryOnly);
    assert_eq!(
        recovery.advance.state.fatigue,
        vec![(effect("a"), 0), (effect("b"), 0)]
    );
    let recovered_current = staged(&exhausted_current, &recovery);
    let resumed = advance(&recovered_current, Some(10), 0, false).unwrap();
    assert_eq!(resumed.rotation, RotationDisposition::Selected(effect("a")));
    assert_eq!(
        original.state().tempo.fatigue,
        vec![(effect("a"), 0), (effect("b"), 0)]
    );
}

#[test]
fn canonical_recovery_and_rotation_preserve_all_mechanics_and_game_time() {
    let current = checkpoint_for(ExecutionMode::PreparedOnly, vec![(effect("a"), 7)], 40);
    let before = current.clone();
    let result = advance(&current, Some(2), 3, true).unwrap();
    assert_eq!(result.advance.state.fatigue, vec![(effect("a"), 6)]);
    assert_eq!(result.advance.execution_mode, ExecutionMode::PreparedOnly);
    assert_eq!(current, before);
    let next = staged(&current, &result);
    let mut expected = current.state().clone();
    expected.tempo = result.advance.state;
    assert_eq!(next.state(), &expected);
    assert_eq!(next.pins(), current.pins());
    assert_eq!(next.basis(), current.basis());
}

#[test]
fn reduced_motion_retains_equivalent_readable_emphasis_without_changing_fatigue_policy() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(effect("a"), 5)], 40);
    let ordinary = advance(&current, Some(2), 1, false).unwrap();
    let reduced_motion = advance(&current, Some(2), 1, true).unwrap();
    assert_eq!(ordinary.advance.state, reduced_motion.advance.state);
    assert_eq!(ordinary.rotation, reduced_motion.rotation);
    assert!(
        !reduced_motion.advance.motion.screen_shake
            && !reduced_motion.advance.motion.forced_camera_punch
            && reduced_motion.advance.motion.readable_emphasis
    );
    assert!(ordinary.advance.motion.screen_shake && ordinary.advance.motion.forced_camera_punch);
}

#[test]
fn replay_pause_and_zero_elapsed_never_charge_or_rotate_fresh_effects() {
    for (mode, paused, duration) in [
        (ExecutionMode::Replay, false, Duration::MAX),
        (ExecutionMode::Live, true, Duration::MAX),
        (ExecutionMode::Live, false, Duration::ZERO),
    ] {
        let current = checkpoint_for(mode, vec![(effect("a"), 5)], 40);
        let elapsed = policy(&current);
        let fatigue_policy = mode_policy(&current, 10, 2);
        let result = advance_recovery(
            &current,
            current.pins(),
            RecoveryRequest {
                elapsed: request(&current, duration, paused),
                elapsed_policy: Some(&elapsed),
                fatigue_policy: Some(&fatigue_policy),
                preference: Some(MotionPreference {
                    expected_basis: current.basis(),
                    reduced_motion: false,
                }),
                pressure_units: Some(1),
            },
            limits(),
        )
        .unwrap();
        assert_eq!(result.rotation, RotationDisposition::Suppressed);
        assert_eq!(result.advance.state, current.state().tempo);
        if mode == ExecutionMode::Replay {
            assert!(
                !result.advance.motion.screen_shake && !result.advance.motion.forced_camera_punch
            );
        }
    }
}

#[test]
fn lexical_rotation_ties_ignore_candidate_order_and_choose_least_fatigue() {
    let current = checkpoint_for(
        ExecutionMode::Live,
        vec![(effect("b"), 1), (effect("a"), 1)],
        40,
    );
    assert_eq!(
        advance(&current, Some(1), 0, false).unwrap().rotation,
        RotationDisposition::Selected(effect("a"))
    );
    let current = checkpoint_for(
        ExecutionMode::Live,
        vec![(effect("a"), 5), (effect("b"), 1)],
        40,
    );
    assert_eq!(
        advance(&current, Some(1), 0, false).unwrap().rotation,
        RotationDisposition::Selected(effect("b"))
    );
}

#[test]
fn capped_and_overflowing_pressure_declines_without_clipping_or_partial_charge() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(effect("a"), 10)], 40);
    let result = advance(&current, Some(u64::MAX), 0, false).unwrap();
    assert_eq!(result.rotation, RotationDisposition::Exhausted);
    assert_eq!(result.advance.state.fatigue, current.state().tempo.fatigue);
    assert_eq!(
        advance(&current, Some(0), 0, false),
        Err(RecoveryError::ZeroPressure)
    );
    assert_eq!(current.state().tempo.fatigue, vec![(effect("a"), 10)]);
}

#[test]
fn current_basis_policy_preferences_and_limits_are_required_before_rotation() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(effect("a"), 5)], 40);
    let elapsed = policy(&current);
    let fatigue_policy = mode_policy(&current, 10, 1);
    for case in ["policy", "preferences", "basis", "capacity"] {
        let mut elapsed_request = request(&current, Duration::from_millis(100), false);
        let mut bound = limits();
        if case == "basis" {
            elapsed_request.expected_basis.revision = revision(7);
        }
        if case == "capacity" {
            bound.maximum_fatigue_entries = 0;
        }
        let result = advance_recovery(
            &current,
            current.pins(),
            RecoveryRequest {
                elapsed: elapsed_request,
                elapsed_policy: Some(&elapsed),
                fatigue_policy: if case == "policy" {
                    None
                } else {
                    Some(&fatigue_policy)
                },
                preference: if case == "preferences" {
                    None
                } else {
                    Some(MotionPreference {
                        expected_basis: current.basis(),
                        reduced_motion: false,
                    })
                },
                pressure_units: Some(1),
            },
            bound,
        );
        let expected = match case {
            "policy" => FatigueAdvanceError::PolicyUnavailable,
            "preferences" => FatigueAdvanceError::PreferenceUnavailable,
            "basis" => FatigueAdvanceError::Elapsed(TempoAdvanceError::Binding(
                CheckpointError::StaleBasis,
            )),
            _ => FatigueAdvanceError::Capacity,
        };
        assert_eq!(result, Err(RecoveryError::Fatigue(expected)));
        assert_eq!(current.state().tempo.fatigue, vec![(effect("a"), 5)]);
    }
}

#[test]
fn stale_anchor_and_recovery_overflow_preserve_original_checkpoint() {
    let current = checkpoint_for(ExecutionMode::Live, vec![(effect("a"), 5)], 40);
    let original = current.clone();
    let elapsed = policy(&current);
    let fatigue_policy = mode_policy(&current, 10, u64::MAX);
    let mut elapsed_request = request(&current, Duration::from_secs(1), false);
    assert_eq!(
        advance_recovery(
            &current,
            current.pins(),
            RecoveryRequest {
                elapsed: elapsed_request,
                elapsed_policy: Some(&elapsed),
                fatigue_policy: Some(&fatigue_policy),
                preference: Some(MotionPreference {
                    expected_basis: current.basis(),
                    reduced_motion: false
                }),
                pressure_units: Some(1)
            },
            limits()
        ),
        Err(RecoveryError::Fatigue(
            FatigueAdvanceError::RecoveryOverflow
        ))
    );
    elapsed_request.from_presentation_ticks = 39;
    assert_eq!(
        advance_recovery(
            &current,
            current.pins(),
            RecoveryRequest {
                elapsed: elapsed_request,
                elapsed_policy: Some(&elapsed),
                fatigue_policy: Some(&fatigue_policy),
                preference: Some(MotionPreference {
                    expected_basis: current.basis(),
                    reduced_motion: false
                }),
                pressure_units: Some(1)
            },
            limits()
        ),
        Err(RecoveryError::Fatigue(FatigueAdvanceError::Elapsed(
            TempoAdvanceError::StalePresentationAnchor
        )))
    );
    assert_eq!(current, original);
}
