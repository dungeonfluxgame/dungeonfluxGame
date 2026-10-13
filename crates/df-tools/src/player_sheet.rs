use df_protocol::common as rpc;

/// Player-facing wording for the accepted character facts. The stored sheet stays unchanged.
pub(crate) fn player_sheet_view(sheet: &rpc::CharacterSheet) -> rpc::CharacterSheet {
    let mut view = sheet.clone();
    view.facts.retain_mut(|fact| {
        if let Some(value) = player_sheet_fact_value(&fact.label, &fact.value) {
            fact.value = value;
            true
        } else {
            false
        }
    });
    view
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
        "First language" | "Second language" | "Base language" | "First class skill"
        | "Second class skill" | "Gaming set" | "Alignment" => Some(display_label(value)),
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

    fn fact(label: &str, value: &str) -> rpc::SheetFact {
        rpc::SheetFact {
            label: label.to_owned(),
            value: value.to_owned(),
        }
    }
}
