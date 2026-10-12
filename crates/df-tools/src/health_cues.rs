//! How hard a hit shakes the screen: the one place the health cues' sizes and timings are
//! decided.
//!
//! Everything here is a pure function of numbers the view already carries (hit points lost,
//! the maximum, whether the blow was critical or dropped the target). Nothing reads the device,
//! its settings or the clock: the cues play for every viewer. All motion is a transform
//! (`translate`), so layout and text stay put and legible; the bounds below are the ceiling for
//! every cue.

/// The smallest shake of any hit that lost hit points, in CSS pixels at the television's
/// 1920-pixel reference width (the driver scales it to the stage's actual width).
pub(crate) const MIN_SHAKE_PX: f64 = 3.0;
/// The largest shake of any single hit, even a critical one, at the 1920-pixel reference width.
pub(crate) const MAX_SHAKE_PX: f64 = 22.0;
/// The largest shake on a phone card, in CSS pixels (a phone is held close).
pub(crate) const MAX_PHONE_SHAKE_PX: f64 = 9.0;
/// The shortest and longest a hit shake lasts.
pub(crate) const MIN_SHAKE_MS: u32 = 220;
pub(crate) const MAX_SHAKE_MS: u32 = 720;
/// Losing this share of the maximum (a half) shakes at full size; less scales down to the
/// minimum.
pub(crate) const FULL_SHAKE_PERMILLE: u32 = 500;
/// Losing less than this share of the maximum (a tenth) is a graze: one small twitch.
pub(crate) const GRAZE_BELOW_PERMILLE: u32 = 100;
/// Losing at least this share (a third of the bar) is a jolt with a vertical kick.
pub(crate) const JOLT_AT_PERMILLE: u32 = 333;
/// A critical hit shakes this much bigger (percent), still never past the maximum.
pub(crate) const CRITICAL_BOOST_PERCENT: u32 = 130;
/// How far a character slumps when dropped to 0, and how long it takes.
pub(crate) const SLUMP_PX: f64 = 16.0;
pub(crate) const SLUMP_MS: u32 = 760;
/// The Critical tier's heartbeat tremor: a slow, small double beat.
pub(crate) const TREMOR_PERIOD_MS: u32 = 1300;
pub(crate) const TREMOR_PX: f64 = 1.8;
/// The light whole-screen shake of a critical hit or a drop to 0.
pub(crate) const SCREEN_SHAKE_PX: f64 = 7.0;
pub(crate) const SCREEN_SHAKE_MS: u32 = 420;
/// A phone's vibration pulse for damage to its own hero: shortest and longest.
pub(crate) const VIBRATE_MIN_MS: u32 = 25;
pub(crate) const VIBRATE_MAX_MS: u32 = 140;
/// The stage width the pixel constants above are written for.
#[cfg(target_arch = "wasm32")]
pub(crate) const REFERENCE_STAGE_WIDTH_PX: f64 = 1920.0;

/// The shape of a shake.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pattern {
    /// A graze: one small sideways twitch.
    Twitch,
    /// A solid hit: a back-and-forth that dies away.
    Jolt,
    /// A third of the bar or more, or a critical hit: the jolt with a vertical kick.
    Heavy,
    /// A drop to 0: a stagger and then the body sinking.
    Slump,
    /// The Critical tier's slow double beat.
    Tremor,
}

/// One keyframe of a pattern: where in the duration (0 to 1) and how far along each axis, as a
/// share of the amplitude (-1 to 1; positive y is down).
pub(crate) type Frame = (f64, f64, f64);

impl Pattern {
    pub(crate) const fn frames(self) -> &'static [Frame] {
        match self {
            Self::Twitch => &[
                (0.0, 0.0, 0.0),
                (0.25, 1.0, 0.0),
                (0.55, -0.7, 0.0),
                (0.8, 0.3, 0.0),
                (1.0, 0.0, 0.0),
            ],
            Self::Jolt => &[
                (0.0, 0.0, 0.0),
                (0.12, -1.0, 0.1),
                (0.3, 0.85, -0.2),
                (0.5, -0.6, 0.1),
                (0.7, 0.35, 0.0),
                (0.88, -0.15, 0.0),
                (1.0, 0.0, 0.0),
            ],
            Self::Heavy => &[
                (0.0, 0.0, 0.0),
                (0.08, -1.0, 0.35),
                (0.2, 1.0, -0.4),
                (0.34, -0.9, 0.3),
                (0.48, 0.7, -0.2),
                (0.62, -0.5, 0.15),
                (0.78, 0.3, 0.0),
                (0.9, -0.12, 0.0),
                (1.0, 0.0, 0.0),
            ],
            Self::Slump => &[
                (0.0, 0.0, 0.0),
                (0.1, -0.6, 0.0),
                (0.22, 0.6, -0.1),
                (0.45, 0.0, 0.4),
                (0.8, 0.0, 1.0),
                (1.0, 0.0, 0.7),
            ],
            Self::Tremor => &[
                (0.0, 0.0, 0.0),
                (0.08, 1.0, 0.3),
                (0.16, -0.8, -0.2),
                (0.24, 0.6, 0.2),
                (0.32, 0.0, 0.0),
                (0.55, 0.0, 0.0),
                (0.62, 0.5, 0.1),
                (0.7, -0.4, 0.0),
                (0.78, 0.0, 0.0),
                (1.0, 0.0, 0.0),
            ],
        }
    }
}

/// One shake: a pattern at an amplitude over a time.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shake {
    pub(crate) pattern: Pattern,
    pub(crate) amplitude_px: f64,
    pub(crate) duration_ms: u32,
}

/// Where a shake plays, which sets its ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Place {
    /// The shared television, at its reference width.
    Display,
    /// A phone's own hero card.
    Phone,
}

impl Place {
    const fn ceiling_px(self) -> f64 {
        match self {
            Self::Display => MAX_SHAKE_PX,
            Self::Phone => MAX_PHONE_SHAKE_PX,
        }
    }
}

/// A blow, as the view's numbers describe it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Blow {
    /// Hit points the blow took (after temporary hit points, if any: the view's own numbers).
    pub(crate) lost: u32,
    pub(crate) maximum: u32,
    pub(crate) critical: bool,
    /// The blow dropped its target to 0.
    pub(crate) dropped: bool,
}

/// The share of the maximum a blow took, in thousandths, at most 1000.
pub(crate) fn lost_permille(lost: u32, maximum: u32) -> u32 {
    let share = u64::from(lost) * 1000 / u64::from(maximum.max(1));
    u32::try_from(share.min(1000)).unwrap_or(1000)
}

fn lerp(from: f64, to: f64, at: f64) -> f64 {
    from + (to - from) * at.clamp(0.0, 1.0)
}

/// The shake of one blow on a character, or `None` for a blow that took nothing and dropped no
/// one. A bigger share of the bar shakes bigger and longer; a critical hit shakes bigger and
/// sharper; a drop to 0 slumps. Never beyond the place's ceiling.
pub(crate) fn blow_shake(blow: Blow, place: Place) -> Option<Shake> {
    if blow.lost == 0 && !blow.dropped {
        return None;
    }
    let permille = lost_permille(blow.lost, blow.maximum);
    let scale = f64::from(permille) / f64::from(FULL_SHAKE_PERMILLE);
    let ceiling = place.ceiling_px();
    let mut amplitude = lerp(MIN_SHAKE_PX.min(ceiling), ceiling, scale);
    let mut duration = lerp(f64::from(MIN_SHAKE_MS), f64::from(MAX_SHAKE_MS), scale);
    if blow.critical {
        amplitude *= f64::from(CRITICAL_BOOST_PERCENT) / 100.0;
        duration *= f64::from(CRITICAL_BOOST_PERCENT) / 100.0;
    }
    let pattern = if blow.dropped {
        amplitude = amplitude.max(SLUMP_PX.min(ceiling));
        duration = duration.max(f64::from(SLUMP_MS));
        Pattern::Slump
    } else if blow.critical || permille >= JOLT_AT_PERMILLE {
        Pattern::Heavy
    } else if permille < GRAZE_BELOW_PERMILLE {
        Pattern::Twitch
    } else {
        Pattern::Jolt
    };
    Some(Shake {
        pattern,
        amplitude_px: amplitude.min(ceiling),
        duration_ms: duration.round().clamp(
            f64::from(MIN_SHAKE_MS),
            f64::from(MAX_SHAKE_MS.max(SLUMP_MS)),
        ) as u32,
    })
}

/// The light whole-screen shake, only for a critical hit or a drop to 0.
pub(crate) fn screen_shake(blow: Blow) -> Option<Shake> {
    (blow.critical || blow.dropped).then_some(Shake {
        pattern: if blow.dropped {
            Pattern::Jolt
        } else {
            Pattern::Heavy
        },
        amplitude_px: SCREEN_SHAKE_PX,
        duration_ms: SCREEN_SHAKE_MS,
    })
}

/// The Critical tier's heartbeat tremor on a card or figure.
pub(crate) const fn tremor() -> Shake {
    Shake {
        pattern: Pattern::Tremor,
        amplitude_px: TREMOR_PX,
        duration_ms: TREMOR_PERIOD_MS,
    }
}

/// How long a phone vibrates for damage to its own hero, scaled by the share of the bar lost.
pub(crate) fn vibration_ms(blow: Blow) -> u32 {
    let scale = f64::from(lost_permille(blow.lost, blow.maximum)) / f64::from(FULL_SHAKE_PERMILLE);
    let milliseconds = lerp(f64::from(VIBRATE_MIN_MS), f64::from(VIBRATE_MAX_MS), scale);
    milliseconds.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blow(lost: u32, maximum: u32) -> Blow {
        Blow {
            lost,
            maximum,
            critical: false,
            dropped: false,
        }
    }

    #[test]
    fn a_blow_that_took_nothing_does_not_shake_but_a_drop_always_does() {
        assert_eq!(blow_shake(blow(0, 13), Place::Display), None);
        let dropped = Blow {
            dropped: true,
            ..blow(0, 13)
        };
        assert_eq!(
            blow_shake(dropped, Place::Display).unwrap().pattern,
            Pattern::Slump
        );
    }

    #[test]
    fn a_bigger_share_of_the_bar_shakes_bigger_and_longer_up_to_the_ceiling() {
        let mut last = (0.0, 0);
        for lost in 1..=13 {
            let shake = blow_shake(blow(lost, 13), Place::Display).unwrap();
            assert!(shake.amplitude_px >= last.0, "lost {lost}");
            assert!(shake.duration_ms >= last.1, "lost {lost}");
            assert!(shake.amplitude_px >= MIN_SHAKE_PX);
            assert!(shake.amplitude_px <= MAX_SHAKE_PX);
            assert!(shake.duration_ms >= MIN_SHAKE_MS && shake.duration_ms <= MAX_SHAKE_MS);
            last = (shake.amplitude_px, shake.duration_ms);
        }
        // Half the bar or more is the full size, and no more.
        assert_eq!(
            blow_shake(blow(7, 13), Place::Display)
                .unwrap()
                .amplitude_px,
            MAX_SHAKE_PX
        );
        assert_eq!(
            blow_shake(blow(13, 13), Place::Display)
                .unwrap()
                .amplitude_px,
            MAX_SHAKE_PX
        );
    }

    #[test]
    fn a_graze_twitches_a_third_of_the_bar_jolts_and_a_critical_hit_is_sharper() {
        // 1 of 20 is a twentieth: a graze.
        assert_eq!(
            blow_shake(blow(1, 20), Place::Display).unwrap().pattern,
            Pattern::Twitch
        );
        // 3 of 20 is 150 permille: a jolt.
        assert_eq!(
            blow_shake(blow(3, 20), Place::Display).unwrap().pattern,
            Pattern::Jolt
        );
        // 7 of 20 is a third of the bar and more: the heavy jolt.
        assert_eq!(
            blow_shake(blow(7, 20), Place::Display).unwrap().pattern,
            Pattern::Heavy
        );
        let plain = blow_shake(blow(3, 20), Place::Display).unwrap();
        let critical = blow_shake(
            Blow {
                critical: true,
                ..blow(3, 20)
            },
            Place::Display,
        )
        .unwrap();
        assert_eq!(critical.pattern, Pattern::Heavy);
        assert!(critical.amplitude_px > plain.amplitude_px);
        assert!(critical.duration_ms > plain.duration_ms);
    }

    #[test]
    fn nothing_ever_exceeds_the_ceiling_even_a_critical_blow_on_a_tiny_maximum() {
        // Maximum 1: the one hit point is the whole bar.
        for critical in [false, true] {
            for dropped in [false, true] {
                let blow = Blow {
                    lost: 1,
                    maximum: 1,
                    critical,
                    dropped,
                };
                let display = blow_shake(blow, Place::Display).unwrap();
                assert!(display.amplitude_px <= MAX_SHAKE_PX);
                assert!(display.duration_ms <= MAX_SHAKE_MS.max(SLUMP_MS));
                let phone = blow_shake(blow, Place::Phone).unwrap();
                assert!(phone.amplitude_px <= MAX_PHONE_SHAKE_PX);
            }
        }
        // More lost than the maximum (an overkill the view clamps) is the whole bar, no more.
        assert_eq!(lost_permille(40, 13), 1000);
        assert_eq!(lost_permille(5, 0), 1000, "no maximum: still bounded");
        const { assert!(TREMOR_PX < MIN_SHAKE_PX) };
        const { assert!(SCREEN_SHAKE_PX < MAX_SHAKE_PX) };
    }

    #[test]
    fn the_whole_screen_shakes_only_for_a_critical_hit_or_a_drop_to_zero() {
        assert_eq!(screen_shake(blow(9, 13)), None);
        assert!(
            screen_shake(Blow {
                critical: true,
                ..blow(9, 13)
            })
            .is_some()
        );
        let dropped = screen_shake(Blow {
            dropped: true,
            ..blow(4, 13)
        })
        .unwrap();
        assert_eq!(dropped.amplitude_px, SCREEN_SHAKE_PX);
        assert_eq!(dropped.duration_ms, SCREEN_SHAKE_MS);
    }

    #[test]
    fn the_heartbeat_is_slow_small_and_every_pattern_starts_and_ends_at_rest() {
        let tremor = tremor();
        assert_eq!(tremor.duration_ms, TREMOR_PERIOD_MS);
        assert!(tremor.duration_ms >= 1000, "slower than a second");
        for pattern in [
            Pattern::Twitch,
            Pattern::Jolt,
            Pattern::Heavy,
            Pattern::Slump,
            Pattern::Tremor,
        ] {
            let frames = pattern.frames();
            assert_eq!(frames.first(), Some(&(0.0, 0.0, 0.0)), "{pattern:?}");
            assert!(frames.windows(2).all(|pair| pair[0].0 < pair[1].0));
            assert_eq!(frames.last().map(|frame| frame.0), Some(1.0));
            assert!(
                frames
                    .iter()
                    .all(|frame| frame.1.abs() <= 1.0 && frame.2.abs() <= 1.0)
            );
        }
        // Everything but the slump comes back to rest.
        for pattern in [
            Pattern::Twitch,
            Pattern::Jolt,
            Pattern::Heavy,
            Pattern::Tremor,
        ] {
            let last = pattern.frames().last().unwrap();
            assert_eq!((last.1, last.2), (0.0, 0.0), "{pattern:?}");
        }
    }

    #[test]
    fn a_phone_vibrates_longer_for_a_bigger_hit_within_its_bounds() {
        let small = vibration_ms(blow(1, 20));
        let big = vibration_ms(blow(12, 20));
        assert!(small >= VIBRATE_MIN_MS && small < big);
        assert_eq!(big, VIBRATE_MAX_MS);
        assert!(vibration_ms(blow(99, 1)) <= VIBRATE_MAX_MS);
    }
}
