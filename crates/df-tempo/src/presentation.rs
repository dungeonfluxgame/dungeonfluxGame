//! State-only presentation policy for live owners that must not alter durable gameplay pins.
//! Units are permille and milliseconds. Numeric coefficients are tuning, never rules mechanics.
use df_model::checkpoint::TempoState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TempoPhase {
    Calm,
    Build,
    Anticipation,
    Peak,
    Release,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationPolicy {
    pub inertia_half_life_millis: u32,
    pub decay_half_life_millis: u32,
    pub fatigue_half_life_millis: u32,
    pub fatigue_charge_permille: u16,
    pub fatigue_attenuation_permille: u16,
    pub thresholds: [u16; 3],
    pub hysteresis_permille: u16,
}

/// Producer supplies only a public target and named, bounded presentation impulse.
/// No canonical fact, entity statistics, rules expiry, or secret can cross this boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationInput {
    pub target_permille: u16,
    pub impulse_permille: i16,
    pub release: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TempoFrame {
    pub state: TempoState,
    pub phase: TempoPhase,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationError {
    InvalidPolicy,
    InvalidState,
    InvalidInput,
    BackwardsTime,
}

impl PresentationPolicy {
    pub fn validate(self) -> Result<(), PresentationError> {
        let [build, anticipation, peak] = self.thresholds;
        if self.inertia_half_life_millis == 0
            || self.decay_half_life_millis == 0
            || self.fatigue_half_life_millis == 0
            || self.fatigue_charge_permille > 1000
            || self.fatigue_attenuation_permille > 1000
            || !(0 < build && build < anticipation && anticipation < peak && peak <= 1000)
            || self.hysteresis_permille >= build
            || self.hysteresis_permille >= anticipation - build
            || self.hysteresis_permille >= peak - anticipation
        {
            return Err(PresentationError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Half-life advancement in constant space; the clock is supplied, and the borrowed state is
/// unchanged on refusal. A single fatigue counter attenuates repeated musical impulses, while
/// negative release impulses always retain their strength. It selects no assets or mechanics.
/// `previous_target` applies to the elapsed interval; a new target starts at `now_millis`.
pub fn advance_presentation(
    current: &TempoState,
    previous_phase: TempoPhase,
    previous_target: u16,
    now_millis: u64,
    input: PresentationInput,
    policy: PresentationPolicy,
) -> Result<TempoFrame, PresentationError> {
    policy.validate()?;
    if input.target_permille > 1000
        || previous_target > 1000
        || !(-1000..=1000).contains(&input.impulse_permille)
    {
        return Err(PresentationError::InvalidInput);
    }
    if !(0..=1000).contains(&current.intensity)
        || !(-1000..=1000).contains(&current.inertia)
        || current.fatigue.len() != 1
        || current.fatigue[0].1 > 1000
    {
        return Err(PresentationError::InvalidState);
    }
    let elapsed_ms = now_millis
        .checked_sub(current.presentation_ticks)
        .ok_or(PresentationError::BackwardsTime)?;
    let mut state = current.clone();
    let retained_share =
        |half_life_ms: u32| 2_f64.powf(-(elapsed_ms as f64) / f64::from(half_life_ms));
    let inertia_retained_share = retained_share(policy.inertia_half_life_millis);
    // Integrate the decaying impulse with the exponential approach to the base. This avoids
    // changing the curve when extra same-phase views sample between committed events.
    let decay_rate_per_ms = std::f64::consts::LN_2 / f64::from(policy.decay_half_life_millis);
    let inertia_rate_per_ms = std::f64::consts::LN_2 / f64::from(policy.inertia_half_life_millis);
    let decay_retained_share = retained_share(policy.decay_half_life_millis);
    let impulse_curve = if policy.decay_half_life_millis == policy.inertia_half_life_millis {
        decay_rate_per_ms * elapsed_ms as f64 * decay_retained_share
    } else {
        decay_rate_per_ms * (inertia_retained_share - decay_retained_share)
            / (decay_rate_per_ms - inertia_rate_per_ms)
    };
    state.intensity = (f64::from(previous_target)
        + (current.intensity as f64 - f64::from(previous_target)) * decay_retained_share
        + current.inertia as f64 * impulse_curve)
        .round()
        .clamp(0.0, 1000.0) as i64;
    state.inertia = (current.inertia as f64 * inertia_retained_share).round() as i64;
    let fatigue_permille = (current.fatigue[0].1 as f64
        * retained_share(policy.fatigue_half_life_millis))
    .round() as u64;
    let impulse_attenuation_permille =
        1000 - fatigue_permille * u64::from(policy.fatigue_attenuation_permille) / 1000;
    let impulse_permille = if input.impulse_permille > 0 {
        i64::from(input.impulse_permille) * impulse_attenuation_permille as i64 / 1000
    } else {
        i64::from(input.impulse_permille)
    };
    if input.release {
        state.inertia = state.inertia.min(0);
    }
    // Release cannot undershoot the newly selected calm/rest base and build back upward.
    let minimum_intensity_permille = if input.release {
        i64::from(input.target_permille)
    } else {
        0
    };
    state.intensity = (state.intensity + impulse_permille).clamp(minimum_intensity_permille, 1000);
    state.inertia = (state.inertia + impulse_permille).clamp(-1000, 1000);
    state.fatigue[0].1 = if impulse_permille > 0 {
        fatigue_permille
            .saturating_add(u64::from(policy.fatigue_charge_permille))
            .min(1000)
    } else {
        fatigue_permille
    };
    state.presentation_ticks = now_millis;
    let phase = classify_phase(
        state.intensity as u16,
        previous_phase,
        input.release,
        policy,
    );
    Ok(TempoFrame { state, phase })
}

fn classify_phase(
    intensity: u16,
    previous_phase: TempoPhase,
    release: bool,
    policy: PresentationPolicy,
) -> TempoPhase {
    if release && intensity >= policy.thresholds[0] {
        return TempoPhase::Release;
    }
    let phases = [
        TempoPhase::Calm,
        TempoPhase::Build,
        TempoPhase::Anticipation,
        TempoPhase::Peak,
    ];
    let mut intensity_level = policy
        .thresholds
        .iter()
        .filter(|threshold| intensity >= **threshold)
        .count();
    if let Some(prior) = phases.iter().position(|phase| *phase == previous_phase)
        && prior > intensity_level
        && intensity.saturating_add(policy.hysteresis_permille) >= policy.thresholds[prior - 1]
    {
        intensity_level = prior;
    }
    phases[intensity_level]
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_model::checkpoint::ContentReference;
    fn state() -> TempoState {
        let policy = ContentReference {
            package: df_types::RevisionLabel::new(Some("tempo")).unwrap(),
            entry: df_types::RevisionLabel::new(Some("music")).unwrap(),
        };
        TempoState {
            policy: policy.clone(),
            presentation_ticks: 0,
            intensity: 800,
            inertia: 0,
            fatigue: vec![(policy, 0)],
        }
    }
    fn policy() -> PresentationPolicy {
        PresentationPolicy {
            inertia_half_life_millis: 2000,
            decay_half_life_millis: 2000,
            fatigue_half_life_millis: 10000,
            fatigue_charge_permille: 200,
            fatigue_attenuation_permille: 600,
            thresholds: [200, 450, 650],
            hysteresis_permille: 20,
        }
    }
    #[test]
    fn half_life_and_release_are_deterministic_and_clock_is_validated() {
        let input = PresentationInput {
            target_permille: 0,
            impulse_permille: 0,
            release: true,
        };
        let frame =
            advance_presentation(&state(), TempoPhase::Peak, 0, 2000, input, policy()).unwrap();
        assert_eq!(frame.state.intensity, 400);
        assert_eq!(frame.phase, TempoPhase::Release);
        assert_eq!(
            frame,
            advance_presentation(&state(), TempoPhase::Peak, 0, 2000, input, policy()).unwrap()
        );
        assert_eq!(
            advance_presentation(&frame.state, frame.phase, 0, 1, input, policy()),
            Err(PresentationError::BackwardsTime)
        );
    }
    #[test]
    fn phase_hysteresis_and_repeated_shocks_remain_bounded() {
        assert_eq!(
            classify_phase(640, TempoPhase::Peak, false, policy()),
            TempoPhase::Peak
        );
        assert_eq!(
            classify_phase(620, TempoPhase::Peak, false, policy()),
            TempoPhase::Anticipation
        );
        let mut frame = TempoFrame {
            state: state(),
            phase: TempoPhase::Peak,
        };
        for at in 1..=20 {
            frame = advance_presentation(
                &frame.state,
                frame.phase,
                800,
                at,
                PresentationInput {
                    target_permille: 800,
                    impulse_permille: 1000,
                    release: false,
                },
                policy(),
            )
            .unwrap();
            assert!((0..=1000).contains(&frame.state.intensity));
            assert!((-1000..=1000).contains(&frame.state.inertia));
            assert!(frame.state.fatigue[0].1 <= 1000);
        }
        let mut invalid = policy();
        invalid.thresholds = [650, 450, 200];
        assert_eq!(invalid.validate(), Err(PresentationError::InvalidPolicy));
    }

    #[test]
    fn repeated_positive_impulses_fatigue_but_release_remains_effective() {
        let input = PresentationInput {
            target_permille: 100,
            impulse_permille: 100,
            release: false,
        };
        let mut initial = state();
        initial.intensity = 100;
        let first =
            advance_presentation(&initial, TempoPhase::Calm, 100, 0, input, policy()).unwrap();
        let second =
            advance_presentation(&first.state, first.phase, 100, 0, input, policy()).unwrap();
        assert!(second.state.intensity - first.state.intensity < 100);
        let release = advance_presentation(
            &second.state,
            second.phase,
            100,
            0,
            PresentationInput {
                impulse_permille: -100,
                release: true,
                ..input
            },
            policy(),
        )
        .unwrap();
        assert_eq!(release.state.intensity, second.state.intensity - 100);
    }
}
