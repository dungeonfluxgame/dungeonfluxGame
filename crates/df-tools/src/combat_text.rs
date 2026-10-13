pub(crate) const fn second_wind_full_hp_label() -> &'static str {
    "Second Wind (you are at full hit points)"
}

pub(crate) const fn second_wind_full_hp_confirmation() -> &'static str {
    "You are at full hit points. Spend a use of Second Wind anyway?"
}

pub(crate) const fn second_wind_confirmation_unavailable() -> &'static str {
    "Second Wind was not used because the confirmation prompt could not be shown."
}

pub(crate) const fn at_full_hit_points(hit_points: u32, maximum_hit_points: u32) -> bool {
    maximum_hit_points > 0 && hit_points >= maximum_hit_points
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srd_p48_full_hit_point_second_wind_label_and_confirmation_are_explicit() {
        assert_eq!(
            second_wind_full_hp_label(),
            "Second Wind (you are at full hit points)"
        );
        assert_eq!(
            second_wind_full_hp_confirmation(),
            "You are at full hit points. Spend a use of Second Wind anyway?"
        );
        assert_eq!(
            second_wind_confirmation_unavailable(),
            "Second Wind was not used because the confirmation prompt could not be shown."
        );
    }

    #[test]
    fn srd_p48_full_hit_point_warning_requires_a_nonzero_maximum() {
        assert!(at_full_hit_points(10, 10));
        assert!(at_full_hit_points(11, 10));
        assert!(!at_full_hit_points(9, 10));
        assert!(!at_full_hit_points(0, 0));
    }
}
