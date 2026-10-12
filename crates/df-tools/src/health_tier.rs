//! How hurt a character is, from the server's view alone: the one definition of the health tiers.
//!
//! The display, the phones and the server's art pipeline all derive a tier through
//! [`tier_of`], so a glance at any screen agrees. A tier is a presentation of numbers the view
//! already carries; it adds no rule, no authority and no hidden state. If the tactical
//! integration later sends conditions or dying state, they arrive through [`Vitals`].
//!
//! | Tier | Hit points left |
//! | --- | --- |
//! | Healthy | more than 75% |
//! | Hurt | more than 50%, up to 75% |
//! | Bloodied | more than 25%, up to 50% (the 2024 rules' own term: half or fewer) |
//! | Critical | more than 0, up to 25% |
//! | Down | 0, or unconscious or dying |

/// Percent above which a character is [`HealthTier::Healthy`].
pub(crate) const HEALTHY_ABOVE_PERCENT: u64 = 75;
/// Percent above which a character is [`HealthTier::Hurt`] rather than worse.
pub(crate) const HURT_ABOVE_PERCENT: u64 = 50;
/// Percent above which a character is [`HealthTier::Bloodied`] rather than worse.
pub(crate) const BLOODIED_ABOVE_PERCENT: u64 = 25;

/// How hurt a character looks and reads, from least to most severe.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum HealthTier {
    Healthy,
    Hurt,
    Bloodied,
    Critical,
    Down,
}

impl HealthTier {
    pub(crate) const ALL: [Self; 5] = [
        Self::Healthy,
        Self::Hurt,
        Self::Bloodied,
        Self::Critical,
        Self::Down,
    ];

    /// The value of `data-health-tier` and the closed vocabulary label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Hurt => "hurt",
            Self::Bloodied => "bloodied",
            Self::Critical => "critical",
            Self::Down => "down",
        }
    }

    #[cfg(test)]
    pub(crate) fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tier| tier.label() == label)
    }

    /// Position from healthy (0) to down (4).
    pub(crate) const fn severity(self) -> u8 {
        match self {
            Self::Healthy => 0,
            Self::Hurt => 1,
            Self::Bloodied => 2,
            Self::Critical => 3,
            Self::Down => 4,
        }
    }

    /// How many tiers apart two tiers are.
    pub(crate) const fn distance(self, other: Self) -> u8 {
        self.severity().abs_diff(other.severity())
    }

    /// Bloodied or worse: the health face outranks the conversation's mood face.
    pub(crate) const fn is_bloodied_or_worse(self) -> bool {
        self.severity() >= Self::Bloodied.severity()
    }
}

/// What the view says about a character's health.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Vitals {
    pub(crate) current: u32,
    pub(crate) maximum: u32,
    /// The view marks the character unconscious (down whatever the number).
    pub(crate) unconscious: bool,
    /// Dying: reserved for the tactical integration's death-saving state; false today.
    pub(crate) dying: bool,
}

impl Vitals {
    #[cfg(test)]
    pub(crate) const fn new(current: u32, maximum: u32) -> Self {
        Self {
            current,
            maximum,
            unconscious: false,
            dying: false,
        }
    }
}

/// The tier of a character, or `None` for one with no hit points to show (maximum 0). Current
/// hit points above the maximum count as the maximum.
pub(crate) fn tier_of(vitals: Vitals) -> Option<HealthTier> {
    if vitals.maximum == 0 {
        return None;
    }
    if vitals.unconscious || vitals.dying || vitals.current == 0 {
        return Some(HealthTier::Down);
    }
    let scaled = u64::from(vitals.current.min(vitals.maximum)) * 100;
    let maximum = u64::from(vitals.maximum);
    Some(if scaled > HEALTHY_ABOVE_PERCENT * maximum {
        HealthTier::Healthy
    } else if scaled > HURT_ABOVE_PERCENT * maximum {
        HealthTier::Hurt
    } else if scaled > BLOODIED_ABOVE_PERCENT * maximum {
        HealthTier::Bloodied
    } else {
        HealthTier::Critical
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tier(current: u32, maximum: u32) -> Option<HealthTier> {
        tier_of(Vitals::new(current, maximum))
    }

    #[test]
    fn every_boundary_of_a_round_maximum_falls_on_the_stated_side() {
        // Maximum 100: the percent is the hit-point count.
        for (current, expected) in [
            (100, HealthTier::Healthy),
            (76, HealthTier::Healthy),
            (75, HealthTier::Hurt),
            (51, HealthTier::Hurt),
            (50, HealthTier::Bloodied),
            (26, HealthTier::Bloodied),
            (25, HealthTier::Critical),
            (1, HealthTier::Critical),
            (0, HealthTier::Down),
        ] {
            assert_eq!(tier(current, 100), Some(expected), "{current}/100");
        }
    }

    #[test]
    fn boundaries_hold_for_the_odd_maximums_a_party_actually_has() {
        // 13: 75% is 9.75, 50% is 6.5, 25% is 3.25.
        let expected = [
            (13, HealthTier::Healthy),
            (10, HealthTier::Healthy),
            (9, HealthTier::Hurt),
            (7, HealthTier::Hurt),
            (6, HealthTier::Bloodied),
            (4, HealthTier::Bloodied),
            (3, HealthTier::Critical),
            (1, HealthTier::Critical),
            (0, HealthTier::Down),
        ];
        for (current, tier_expected) in expected {
            assert_eq!(tier(current, 13), Some(tier_expected), "{current}/13");
        }
        // 8: 75% is exactly 6, 50% exactly 4, 25% exactly 2.
        for (current, tier_expected) in [
            (7, HealthTier::Healthy),
            (6, HealthTier::Hurt),
            (5, HealthTier::Hurt),
            (4, HealthTier::Bloodied),
            (3, HealthTier::Bloodied),
            (2, HealthTier::Critical),
            (1, HealthTier::Critical),
        ] {
            assert_eq!(tier(current, 8), Some(tier_expected), "{current}/8");
        }
    }

    #[test]
    fn a_maximum_of_one_is_healthy_at_one_and_down_at_zero() {
        assert_eq!(tier(1, 1), Some(HealthTier::Healthy));
        assert_eq!(tier(0, 1), Some(HealthTier::Down));
    }

    #[test]
    fn no_maximum_means_no_tier_and_excess_hit_points_count_as_full() {
        assert_eq!(tier(0, 0), None);
        assert_eq!(tier(5, 0), None);
        assert_eq!(tier(50, 13), Some(HealthTier::Healthy));
        assert_eq!(tier(u32::MAX, 1), Some(HealthTier::Healthy));
        assert_eq!(tier(u32::MAX, u32::MAX), Some(HealthTier::Healthy));
        assert_eq!(tier(1, u32::MAX), Some(HealthTier::Critical));
    }

    #[test]
    fn healing_walks_back_up_through_every_tier_in_order() {
        let walk: Vec<HealthTier> = (0..=13).filter_map(|hp| tier(hp, 13)).collect();
        assert!(walk.windows(2).all(|pair| pair[0] >= pair[1]));
        assert_eq!(walk.first(), Some(&HealthTier::Down));
        assert_eq!(walk.last(), Some(&HealthTier::Healthy));
        for step in HealthTier::ALL {
            assert!(walk.contains(&step), "{step:?} is reachable by healing");
        }
    }

    #[test]
    fn conditions_override_the_number_when_the_view_carries_them() {
        let mut vitals = Vitals::new(9, 13);
        assert_eq!(tier_of(vitals), Some(HealthTier::Hurt));
        vitals.unconscious = true;
        assert_eq!(tier_of(vitals), Some(HealthTier::Down));
        let dying = Vitals {
            dying: true,
            ..Vitals::new(13, 13)
        };
        assert_eq!(tier_of(dying), Some(HealthTier::Down));
        // A character with no hit points to show stays tier-less whatever the flags say.
        let flagged = Vitals {
            unconscious: true,
            ..Vitals::new(0, 0)
        };
        assert_eq!(tier_of(flagged), None);
    }

    #[test]
    fn labels_round_trip_and_severity_orders_the_tiers() {
        for tier in HealthTier::ALL {
            assert_eq!(HealthTier::from_label(tier.label()), Some(tier));
        }
        assert_eq!(HealthTier::from_label("wounded"), None);
        assert_eq!(HealthTier::Healthy.distance(HealthTier::Critical), 3);
        assert_eq!(HealthTier::Critical.distance(HealthTier::Healthy), 3);
        assert!(!HealthTier::Hurt.is_bloodied_or_worse());
        assert!(HealthTier::Bloodied.is_bloodied_or_worse());
        assert!(HealthTier::Down.is_bloodied_or_worse());
    }
}
