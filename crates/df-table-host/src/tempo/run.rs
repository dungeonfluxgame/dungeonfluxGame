//! The journey's tempo: one `TempoState` that `df_tempo::presentation::advance_presentation`
//! moves at the times it is told.
//!
//! The run keeps no clock. Every call carries `now_ms`, so a test drives it with a number and the
//! service drives it with the host's monotonic clock. Impulses and the elapsed time go through the
//! engine's own types; the run adds only the memory of recent impulses that a profile's `recent`
//! condition reads, and the classification of the inertia as a momentum.

use std::collections::VecDeque;

use df_model::checkpoint::{ContentReference, TempoState};
use df_tempo::presentation::{
    PresentationInput, PresentationPolicy, TempoPhase, advance_presentation,
};
use df_types::RevisionLabel;

use super::limits::{MAXIMUM_PERMILLE, MAXIMUM_RECENT_IMPULSES};
use super::request::{EngineCause, Momentum, PhaseKey, TempoError};
use super::table::{ImpulseRule, MusicTable};

/// An impulse the run remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Recent {
    pub(super) kind: String,
    pub(super) intensity: u16,
    pub(super) at_ms: u64,
    pub(super) note: Option<String>,
}

/// The tempo at one time, as the director reads it.
pub(super) struct Moment<'run> {
    pub(super) now_ms: u64,
    pub(super) phase: PhaseKey,
    pub(super) intensity: u16,
    pub(super) inertia: i16,
    pub(super) momentum: Momentum,
    pub(super) target: u16,
    pub(super) fatigue: u16,
    pub(super) recent: &'run VecDeque<Recent>,
}

/// One journey's tempo.
#[derive(Clone)]
pub(super) struct TempoRun {
    policy: PresentationPolicy,
    deadband: i64,
    state: TempoState,
    phase: TempoPhase,
    target: u16,
    recent: VecDeque<Recent>,
}

impl TempoRun {
    /// A journey at rest at `now_ms`.
    pub(super) fn new(table: &MusicTable, now_ms: u64) -> Result<Self, TempoError> {
        let unreachable_label = |_| TempoError::Engine {
            cause: EngineCause::InvalidState,
        };
        let label = |text| RevisionLabel::new(Some(text)).map_err(unreachable_label);
        let policy_reference = ContentReference {
            package: label("tempo")?,
            entry: label("music")?,
        };
        let rest = table.policy.rest_permille;
        Ok(Self {
            policy: table.policy.presentation(),
            deadband: i64::from(table.policy.momentum_deadband_permille),
            state: TempoState {
                policy: policy_reference.clone(),
                presentation_ticks: now_ms,
                intensity: i64::from(rest),
                inertia: 0,
                fatigue: vec![(policy_reference, 0)],
            },
            phase: TempoPhase::Calm,
            target: rest,
            recent: VecDeque::new(),
        })
    }

    /// Moves the tempo to `now_ms` with no impulse: the intensity settles toward its target.
    pub(super) fn advance(&mut self, now_ms: u64) -> Result<(), TempoError> {
        let input = PresentationInput {
            target_permille: self.target,
            impulse_permille: 0,
            release: false,
        };
        self.step(now_ms, input)
    }

    /// Moves the tempo to `now_ms` and applies `rule` at `strength` permille of its push.
    pub(super) fn impulse(
        &mut self,
        rule: &ImpulseRule,
        strength: u16,
        note: Option<String>,
        now_ms: u64,
    ) -> Result<(), TempoError> {
        let scaled = i32::from(rule.impulse_permille) * i32::from(strength.min(MAXIMUM_PERMILLE))
            / i32::from(MAXIMUM_PERMILLE);
        let input = PresentationInput {
            target_permille: rule.target_permille,
            // The push is within the rule's own bounds, which the engine takes.
            impulse_permille: i16::try_from(scaled).unwrap_or_default(),
            release: rule.release,
        };
        self.step(now_ms, input)?;
        self.target = rule.target_permille;
        if self.recent.len() == MAXIMUM_RECENT_IMPULSES {
            self.recent.pop_front();
        }
        self.recent.push_back(Recent {
            kind: rule.kind.clone(),
            intensity: strength,
            at_ms: now_ms,
            note,
        });
        Ok(())
    }

    fn step(&mut self, now_ms: u64, input: PresentationInput) -> Result<(), TempoError> {
        let frame = advance_presentation(
            &self.state,
            self.phase,
            self.target,
            now_ms,
            input,
            self.policy,
        )
        .map_err(|error| TempoError::Engine {
            cause: EngineCause::from(error),
        })?;
        self.state = frame.state;
        self.phase = frame.phase;
        Ok(())
    }

    /// The tempo as last advanced, for the director to read at `now_ms`.
    pub(super) fn moment(&self, now_ms: u64) -> Moment<'_> {
        let inertia = self.state.inertia;
        let momentum = if inertia > self.deadband {
            Momentum::Rising
        } else if inertia < -self.deadband {
            Momentum::Falling
        } else {
            Momentum::Stable
        };
        let fatigue = self.state.fatigue.first().map_or(0, |(_, charge)| *charge);
        Moment {
            now_ms,
            phase: PhaseKey::of(self.phase),
            intensity: permille(self.state.intensity),
            inertia: i16::try_from(inertia).unwrap_or_default(),
            momentum,
            target: self.target,
            fatigue: permille(i64::try_from(fatigue).unwrap_or(i64::MAX)),
            recent: &self.recent,
        }
    }
}

/// A value the engine keeps within 0 to 1000, as a `u16`.
fn permille(value: i64) -> u16 {
    u16::try_from(value.clamp(0, i64::from(MAXIMUM_PERMILLE))).unwrap_or_default()
}
