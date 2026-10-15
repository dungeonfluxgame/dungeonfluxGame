pub(crate) const SHEET_ABILITIES_AND_SAVES: &str = "Abilities and saves";
pub(crate) const SHEET_SKILLS: &str = "Skills";
pub(crate) const SHEET_COMBAT: &str = "Combat";
pub(crate) const SHEET_EQUIPMENT: &str = "Equipment";
pub(crate) const SHEET_FEATURES_AND_TRAITS: &str = "Features and traits";

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

pub(crate) fn combat_attack_label(
    actor: &str,
    target: &str,
    weapon_id: &str,
    weapon_label: &str,
    attack_style: u32,
) -> String {
    // SRD 5.2.1 pp. 90-91: thrown weapons are thrown, ammunition weapons shoot,
    // and melee weapons strike.
    if attack_style == 2
        && matches!(
            weapon_id,
            "dagger" | "dart" | "handaxe" | "javelin" | "light-hammer" | "spear" | "trident"
        )
    {
        return format!(
            "{actor} throws the {} at {target}",
            weapon_label.to_lowercase()
        );
    }
    let verb = if attack_style == 2 {
        "shoots"
    } else {
        "strikes"
    };
    format!("{actor} {verb} {target} · {weapon_label}")
}

pub(crate) fn ammunition_labels(arrows: u32, bolts: u32) -> Vec<String> {
    let mut labels = Vec::new();
    if arrows > 0 {
        labels.push(format!("Arrows {arrows}"));
    }
    if bolts > 0 {
        labels.push(format!("Bolts {bolts}"));
    }
    labels
}

pub(crate) fn opportunity_attack_prompt(mover: &str) -> String {
    // SRD 5.2.1 p. 15: "You can make an Opportunity Attack when a creature that you can see leaves your reach."
    format!("{mover} is running past you: make an Opportunity Attack?")
}

pub(crate) fn reaction_waiting_prompt(reactor: &str) -> String {
    format!("Waiting for {reactor} to decide")
}

pub(crate) fn reaction_countdown(remaining_seconds: u32) -> Option<String> {
    match remaining_seconds {
        0 => None,
        1 => Some("1 second remaining".to_owned()),
        seconds => Some(format!("{seconds} seconds remaining")),
    }
}

pub(crate) fn damage_summary(damage: u32, rolls: &[(&[u32], bool)], modifier: i32) -> String {
    if damage == 0 {
        return "No damage".to_owned();
    }
    if rolls.is_empty() {
        return format!("{damage} damage");
    }
    // SRD 5.2.1 p. 87: show both complete Savage Attacker rolls, the one kept, and its modifier.
    let rolls = rolls
        .iter()
        .map(|(dice, kept)| {
            let dice = dice
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(" + ");
            if *kept { format!("kept {dice}") } else { dice }
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{damage} damage ({rolls}; modifier {modifier:+})")
}

pub(crate) fn graze_summary(weapon: &str, damage: u32, damage_type: &str) -> String {
    // SRD 5.2.1 p. 89: Graze deals damage on a missed attack roll; it is still a miss.
    let damage = if damage_type.is_empty() {
        format!("{damage} damage")
    } else {
        format!("{damage} {damage_type} damage")
    };
    format!(
        "Miss, but the {}'s Graze deals {damage}",
        weapon.to_lowercase()
    )
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

    #[test]
    fn srd_p47_p83_p84_character_sheet_group_titles_cover_the_character_facts() {
        assert_eq!(
            [
                SHEET_ABILITIES_AND_SAVES,
                SHEET_SKILLS,
                SHEET_COMBAT,
                SHEET_EQUIPMENT,
                SHEET_FEATURES_AND_TRAITS,
            ],
            [
                "Abilities and saves",
                "Skills",
                "Combat",
                "Equipment",
                "Features and traits",
            ]
        );
    }

    #[test]
    fn phone_combat_attack_labels_distinguish_thrown_shot_and_melee_weapons() {
        assert_eq!(
            combat_attack_label("Brakka", "Sigrun", "javelin", "Javelin", 2),
            "Brakka throws the javelin at Sigrun"
        );
        assert_eq!(
            combat_attack_label("Brakka", "Sigrun", "light-crossbow", "Light Crossbow", 2),
            "Brakka shoots Sigrun · Light Crossbow"
        );
        assert_eq!(
            combat_attack_label("Brakka", "Sigrun", "greatsword", "Greatsword", 1),
            "Brakka strikes Sigrun · Greatsword"
        );
    }

    #[test]
    fn phone_combat_ammunition_hides_unavailable_types() {
        assert_eq!(ammunition_labels(20, 0), ["Arrows 20"]);
        assert_eq!(ammunition_labels(0, 0), Vec::<String>::new());
        assert_eq!(ammunition_labels(20, 10), ["Arrows 20", "Bolts 10"]);
    }

    #[test]
    fn phone_opportunity_reaction_prompt_names_the_mover_and_asks_the_player() {
        assert_eq!(
            opportunity_attack_prompt("Giant Rat 1"),
            "Giant Rat 1 is running past you: make an Opportunity Attack?"
        );
    }

    #[test]
    fn phone_reaction_waiting_names_the_decision_owner() {
        assert_eq!(
            reaction_waiting_prompt("Brakka"),
            "Waiting for Brakka to decide"
        );
    }

    #[test]
    fn phone_reaction_countdown_formats_server_remaining_seconds() {
        assert_eq!(
            reaction_countdown(20).as_deref(),
            Some("20 seconds remaining")
        );
        assert_eq!(reaction_countdown(1).as_deref(), Some("1 second remaining"));
        assert_eq!(reaction_countdown(0), None);
    }

    #[test]
    fn srd_p87_phone_damage_summary_shows_both_savage_attacker_rolls_kept_roll_and_modifier() {
        let rolls = [(&[6, 4][..], false), (&[6, 5][..], true)];

        assert_eq!(
            damage_summary(14, &rolls, 3),
            "14 damage (6 + 4; kept 6 + 5; modifier +3)"
        );
    }

    #[test]
    fn srd_p89_phone_graze_calls_a_miss_a_miss_and_explains_its_damage() {
        assert_eq!(
            graze_summary("Greatsword", 3, "slashing"),
            "Miss, but the greatsword's Graze deals 3 slashing damage"
        );
        assert_eq!(
            graze_summary("Greatsword", 0, ""),
            "Miss, but the greatsword's Graze deals 0 damage"
        );
    }
}
