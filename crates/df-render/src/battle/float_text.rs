//! The floating result at a target: what it says, how long it stays and where it sits. It names
//! the attack and its result ("Greatsword · 9 slashing", "Longbow · miss", "Bite · 7 piercing, critical"), rises to
//! rest above the target, holds there long enough to read across a room, then fades. Several
//! results close together (enemies acting in a row) are stacked so none covers another. Pure
//! presentation: the numbers and the attack's name come from the server's outcome, and nothing
//! here decides a result.

use super::{model::FloatTone, speech::Rect};

/// How long the number takes to rise to its resting height, in seconds.
pub const RISE_S: f32 = 0.6;
/// How long it then stays, fully shown, in seconds: at least two and a half.
pub const HOLD_S: f32 = 2.6;
/// A critical hit or a knockout stays longer.
pub const HOLD_EMPHASIS_S: f32 = 3.6;
/// How long it fades out, in seconds.
pub const FADE_S: f32 = 0.5;
/// How long it fades in, in seconds.
pub const FADE_IN_S: f32 = 0.12;

/// How long a result stays at rest, in seconds.
pub fn hold_for(tone: FloatTone, downed: bool) -> f32 {
    if downed || tone == FloatTone::Critical {
        HOLD_EMPHASIS_S
    } else {
        HOLD_S
    }
}

/// Its whole life, in seconds.
pub fn lifetime(hold_s: f32) -> f32 {
    RISE_S + hold_s + FADE_S
}

/// Metres above the head at `age_s`: eased up over [`RISE_S`], then still.
pub fn rise_m(age_s: f32) -> f32 {
    let t = (age_s / RISE_S).clamp(0.0, 1.0);
    let eased = 1.0 - (1.0 - t) * (1.0 - t);
    0.2 + 0.7 * eased
}

/// How opaque it is at `age_s` for a result that holds `hold_s`.
pub fn opacity(age_s: f32, hold_s: f32) -> f32 {
    let appear = (age_s / FADE_IN_S).clamp(0.0, 1.0);
    let leave = ((lifetime(hold_s) - age_s) / FADE_S).clamp(0.0, 1.0);
    appear.min(leave)
}

/// The words of a result. `base` is the plain result ("Miss" or the damage number); with an
/// attack's name it reads "Name · result", the result lower-cased for a miss, the damage type
/// after the number ("Greatsword · 9 slashing"), with ", critical" and ", down" after a damaging
/// blow that was one. Without a name it is just `base`. `attack` is the name and the damage type
/// (empty when the server sent none).
pub fn compose(attack: Option<(&str, &str)>, base: &str, tone: FloatTone, downed: bool) -> String {
    let Some((attack, damage_type)) = attack
        .map(|(name, kind)| (name.trim(), kind.trim()))
        .filter(|(name, _)| !name.is_empty())
    else {
        return base.to_owned();
    };
    match tone {
        FloatTone::Miss => format!("{attack} \u{b7} {}", base.to_lowercase()),
        FloatTone::Heal => base.to_owned(),
        FloatTone::Damage | FloatTone::Critical => {
            let mut text = format!("{attack} \u{b7} {base}");
            if !damage_type.is_empty() {
                text.push(' ');
                text.push_str(damage_type);
            }
            if tone == FloatTone::Critical {
                text.push_str(", critical");
            }
            if downed {
                text.push_str(", down");
            }
            text
        }
    }
}

/// An attack's name from the server's weapon id: "hand-axe" reads "Hand axe". Presentation
/// only; the id is what the server sent.
pub fn attack_name(weapon_id: &str) -> String {
    let words = weapon_id.trim().replace(['-', '_'], " ");
    let mut characters = words.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// A guess at a result's size in screen fractions, for stacking: wider with its words.
pub fn estimate_size(text: &str, aspect: f32) -> (f32, f32) {
    let characters = text.chars().count().max(1) as f32;
    let line_width = 0.03 + characters * 0.0145 * (16.0 / 9.0) / aspect.max(0.1);
    let max_width = 0.51;
    let lines = (line_width / max_width).ceil().max(1.0);
    (line_width.min(max_width), 0.05 * lines)
}

/// Keeps the whole floating result inside the clear middle used by the resting shot.
pub fn clamp_to_rest_area(x: f32, y: f32, width: f32, height: f32) -> (f32, f32) {
    let (left, top, right, bottom) = super::rest_shot::REST_AREA;
    (
        x.clamp(left + width * 0.5, right - width * 0.5),
        y.clamp(top + height * 0.5, bottom - height * 0.5),
    )
}

/// Moves `wanted` (a result's resting box) clear of the boxes already `placed`: up by whole steps
/// first (newer results stack above), then down if the top of the picture is reached. Returns the
/// vertical shift to apply. A box that finds no clear place keeps its own position.
pub fn clear_shift(placed: &[Rect], wanted: Rect, step: f32, top: f32, bottom: f32) -> f32 {
    let clear = |shift: f32| {
        let moved = Rect {
            top: wanted.top + shift,
            bottom: wanted.bottom + shift,
            ..wanted
        };
        placed.iter().all(|other| moved.overlap(other) <= 1e-6)
    };
    if clear(0.0) {
        return 0.0;
    }
    let mut up = 1;
    while wanted.top - step * up as f32 >= top {
        if clear(-step * up as f32) {
            return -step * up as f32;
        }
        up += 1;
    }
    let mut down = 1;
    while wanted.bottom + step * down as f32 <= bottom {
        if clear(step * down as f32) {
            return step * down as f32;
        }
        down += 1;
    }
    0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_stays_readable_for_at_least_two_and_a_half_seconds_at_rest() {
        const { assert!(HOLD_S >= 2.5) };
        const { assert!(HOLD_EMPHASIS_S > HOLD_S) };
        for tone in [FloatTone::Damage, FloatTone::Miss, FloatTone::Heal] {
            assert_eq!(hold_for(tone, false), HOLD_S);
            assert_eq!(hold_for(tone, true), HOLD_EMPHASIS_S);
        }
        assert_eq!(hold_for(FloatTone::Critical, false), HOLD_EMPHASIS_S);
        let hold = HOLD_S;
        // Fully shown from the end of the rise for the whole hold.
        assert_eq!(opacity(RISE_S, hold), 1.0);
        assert_eq!(opacity(RISE_S + hold - 0.01, hold), 1.0);
        assert!(opacity(RISE_S + hold + FADE_S * 0.5, hold) < 1.0);
        assert_eq!(opacity(lifetime(hold), hold), 0.0);
        // At rest it no longer rises.
        assert_eq!(rise_m(RISE_S), rise_m(RISE_S + 2.0));
        assert!(rise_m(0.0) < rise_m(RISE_S));
    }

    #[test]
    fn srd_p87_and_p90_a_result_names_critical_and_graze_damage() {
        use FloatTone::*;
        assert_eq!(
            compose(Some(("Greatsword", "slashing")), "9", Damage, false),
            "Greatsword \u{b7} 9 slashing"
        );
        assert_eq!(
            compose(Some(("Longbow", "piercing")), "Miss", Miss, false),
            "Longbow \u{b7} miss"
        );
        assert_eq!(
            compose(Some(("Bite", "piercing")), "7", Critical, false),
            "Bite \u{b7} 7 piercing, critical"
        );
        assert_eq!(
            compose(Some(("Graze", "slashing")), "3", Damage, false),
            "Graze \u{b7} 3 slashing"
        );
        assert_eq!(
            compose(Some(("Greatsword", "slashing")), "9", Critical, true),
            "Greatsword \u{b7} 9 slashing, critical, down"
        );
        assert_eq!(
            compose(Some(("Dagger", "")), "4", Damage, true),
            "Dagger \u{b7} 4, down"
        );
        // No name: exactly what the stage showed before.
        assert_eq!(compose(None, "Miss", Miss, false), "Miss");
        assert_eq!(compose(Some(("  ", "")), "5", Damage, false), "5");
        assert_eq!(compose(Some(("Healing word", "")), "5", Heal, false), "5");
    }

    #[test]
    fn an_attack_is_named_from_the_servers_weapon_id() {
        assert_eq!(attack_name("greatsword"), "Greatsword");
        assert_eq!(attack_name("hand-axe"), "Hand axe");
        assert_eq!(attack_name("light_crossbow"), "Light crossbow");
        assert_eq!(attack_name(""), "");
    }

    #[test]
    fn srd_p90_a_graze_result_stays_inside_the_clear_middle_of_the_picture() {
        let (x, y) = clamp_to_rest_area(0.0, 1.0, 0.4, 0.1);
        let (left, top, right, bottom) = super::super::rest_shot::REST_AREA;
        assert!(x - 0.2 >= left && x + 0.2 <= right);
        assert!(y - 0.05 >= top && y + 0.05 <= bottom);
    }

    #[test]
    fn srd_p90_a_long_graze_result_has_a_wrapped_height_estimate() {
        let (width, height) = estimate_size(
            "Greatsword · 3 slashing, graze, disadvantage from target dodging",
            16.0 / 9.0,
        );
        assert!(width <= 0.51);
        assert!(height > 0.05);
    }

    fn at(left: f32, top: f32, width: f32) -> Rect {
        Rect {
            left,
            top,
            right: left + width,
            bottom: top + 0.05,
        }
    }

    #[test]
    fn results_that_would_overlap_are_stacked_not_drawn_over_each_other() {
        let first = at(0.4, 0.5, 0.2);
        let second = at(0.45, 0.5, 0.2);
        let shift = clear_shift(&[first], second, 0.055, 0.27, 0.92);
        assert!(shift < 0.0, "the newer result goes above: {shift}");
        let moved = Rect {
            top: second.top + shift,
            bottom: second.bottom + shift,
            ..second
        };
        assert_eq!(moved.overlap(&first), 0.0);
        // Three in a row stack three high, none over another.
        let third = at(0.42, 0.5, 0.2);
        let placed = [first, moved];
        let shift = clear_shift(&placed, third, 0.055, 0.27, 0.92);
        let moved_third = Rect {
            top: third.top + shift,
            bottom: third.bottom + shift,
            ..third
        };
        assert!(placed.iter().all(|other| moved_third.overlap(other) == 0.0));
        // Apart already: no shift.
        assert_eq!(
            clear_shift(&[first], at(0.7, 0.5, 0.2), 0.055, 0.27, 0.92),
            0.0
        );
    }

    #[test]
    fn at_the_top_of_the_picture_a_result_stacks_downward_instead() {
        let first = at(0.4, 0.28, 0.2);
        let second = at(0.4, 0.28, 0.2);
        let shift = clear_shift(&[first], second, 0.055, 0.27, 0.92);
        assert!(shift > 0.0, "{shift}");
    }
}
