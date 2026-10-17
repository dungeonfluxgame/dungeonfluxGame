use df_protocol::common as rpc;

/// Player-facing wording for the accepted character facts. The stored sheet stays unchanged.
pub(crate) fn player_sheet_view(sheet: &rpc::CharacterSheet) -> rpc::CharacterSheet {
    let mut view = sheet.clone();
    view.facts.retain_mut(|fact| {
        // Facts stored before the content fix may hold UTF-8 read as Latin-1 ("Â·").
        let stored = repair_double_encoded(&fact.value).into_owned();
        if let Some(value) = player_sheet_fact_value(&fact.label, &stored) {
            fact.value = value;
            true
        } else {
            false
        }
    });
    view
}

/// Undoes a UTF-8 text that was decoded as Latin-1 and stored again, such as "Â·" for "·".
/// Text is changed only when every character is a Latin-1 byte and those bytes form valid
/// UTF-8 that differs from the input, so ordinary text (and any real non-Latin-1 character)
/// is returned unchanged.
pub(crate) fn repair_double_encoded(text: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mut bytes = Vec::with_capacity(text.len());
    for character in text.chars() {
        match u8::try_from(u32::from(character)) {
            Ok(byte) => bytes.push(byte),
            Err(_) => return Cow::Borrowed(text),
        }
    }
    match String::from_utf8(bytes) {
        Ok(fixed) if fixed != text => Cow::Owned(fixed),
        _ => Cow::Borrowed(text),
    }
}

fn player_sheet_fact_value(label: &str, value: &str) -> Option<String> {
    match label {
        "Supported rules" => None,
        "Starting equipment" => {
            let Some((equipment, coins)) = value.rsplit_once(" · ") else {
                return Some(value.to_owned());
            };
            let Some(amount) = coins.strip_suffix(" GP") else {
                return Some(value.to_owned());
            };
            if matches!(equipment, "Fighter A + Soldier A" | "Fighter B + Soldier B") {
                Some(format!("{amount} gp"))
            } else {
                Some(format!("{equipment} · {amount} gp"))
            }
        }
        "First language"
        | "Second language"
        | "Base language"
        | "First class skill"
        | "Second class skill"
        | "Soldier skill proficiencies"
        | "Saving throw proficiencies"
        | "In hand"
        | "Dropped weapons"
        | "Gaming set"
        | "Alignment" => Some(display_label(value)),
        _ if value == "Held" => Some("(in hand)".to_owned()),
        _ if value.starts_with("Dropped") => Some("(dropped)".to_owned()),
        _ => Some(value.to_owned()),
    }
}

/// Converts stored ids such as `dwarvish` to a readable label without changing title case.
pub(crate) fn display_label(label: &str) -> String {
    if label.chars().any(char::is_uppercase) {
        return label.to_owned();
    }
    label
        .split(['-', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut letters = word.chars();
            letters.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(letters).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::player_sheet_view;
    use df_protocol::common as rpc;

    #[test]
    fn player_sheet_view_converts_stored_fact_values_to_player_wording() {
        let sheet = rpc::CharacterSheet {
            facts: vec![
                fact("First class skill", "insight"),
                fact("Second class skill", "persuasion"),
                fact("Base language", "common"),
                fact("First language", "dwarvish"),
                fact("Second language", "giant"),
                fact("Gaming set", "dice"),
                fact("Alignment", "lawful good"),
                fact("Starting equipment", "Fighter A + Soldier A · 18 GP"),
                fact("Greatsword", "Held"),
                fact("Supported rules", "rules-revision"),
            ],
            ..Default::default()
        };

        let view = player_sheet_view(&sheet);
        let values: Vec<_> = view
            .facts
            .iter()
            .map(|fact| (fact.label.as_str(), fact.value.as_str()))
            .collect();

        assert_eq!(
            values,
            [
                ("First class skill", "Insight"),
                ("Second class skill", "Persuasion"),
                ("Base language", "Common"),
                ("First language", "Dwarvish"),
                ("Second language", "Giant"),
                ("Gaming set", "Dice"),
                ("Alignment", "Lawful Good"),
                ("Starting equipment", "18 gp"),
                ("Greatsword", "(in hand)"),
            ]
        );
        assert_eq!(sheet.facts[0].value, "insight");
    }

    /// SRD 5.2.1 pp. 47, 83-84: show the Soldier and Fighter's stated proficiencies by name.
    #[test]
    fn player_sheet_view_formats_soldier_and_saving_throw_proficiencies() {
        let sheet = rpc::CharacterSheet {
            facts: vec![
                fact("Soldier skill proficiencies", "athletics, intimidation"),
                fact("Saving throw proficiencies", "strength, constitution"),
            ],
            ..Default::default()
        };

        let view = player_sheet_view(&sheet);

        assert_eq!(view.facts[0].value, "Athletics, Intimidation");
        assert_eq!(view.facts[1].value, "Strength, Constitution");
        assert_eq!(sheet.facts[0].value, "athletics, intimidation");
    }

    #[test]
    fn stored_double_encoded_middle_dot_is_read_back_as_a_middle_dot() {
        use super::repair_double_encoded;
        // The hero card text as a stored, double-encoded fact holds it.
        assert_eq!(
            repair_double_encoded("Fighter Â· level 1"),
            "Fighter · level 1"
        );
        // Correct text, ASCII text and a real Latin-1 word stay as they are.
        assert_eq!(
            repair_double_encoded("Fighter · level 1"),
            "Fighter · level 1"
        );
        assert_eq!(repair_double_encoded("Dwarf Fighter"), "Dwarf Fighter");
        assert_eq!(repair_double_encoded("café"), "café");
    }

    #[test]
    fn hero_card_identity_is_repaired_when_the_stored_class_is_double_encoded() {
        let sheet = rpc::CharacterSheet {
            facts: vec![
                fact("Species", "Dwarf"),
                fact("Class", "Fighter Â· level 1"),
            ],
            ..Default::default()
        };

        let view = player_sheet_view(&sheet);

        assert_eq!(view.facts[1].value, "Fighter · level 1");
        assert_eq!(sheet.facts[1].value, "Fighter Â· level 1");
    }

    #[test]
    fn shipped_merchant_pack_holds_no_double_encoded_text() {
        let pack = include_str!("../../../content/merchant-shop/pack.json");
        assert!(!pack.contains('\u{00c2}'), "merchant pack has Â mojibake");
        assert!(pack.contains("Fighter · level 1"));
    }

    fn fact(label: &str, value: &str) -> rpc::SheetFact {
        rpc::SheetFact {
            label: label.to_owned(),
            value: value.to_owned(),
        }
    }
}
