//! Filter server-authored creation dependencies without inferring rules in the client.
use df_protocol::common as rpc;

pub(super) fn option_available(
    option: &rpc::JourneyOption,
    choices: &[rpc::JourneyChoice],
) -> bool {
    option.requires_group.is_empty()
        || choices.iter().any(|choice| {
            choice.group_id == option.requires_group
                && option.requires_options.contains(&choice.option_id)
        })
}

pub(super) fn group_available(group: &rpc::JourneyGroup, choices: &[rpc::JourneyChoice]) -> bool {
    group
        .options
        .iter()
        .any(|option| option_available(option, choices))
}

pub(super) fn missing_groups(
    groups: &[rpc::JourneyGroup],
    choices: &[rpc::JourneyChoice],
) -> Vec<String> {
    groups
        .iter()
        .filter(|group| group_available(group, choices))
        .filter(|group| {
            !choices.iter().any(|choice| {
                choice.group_id == group.group_id
                    && group.options.iter().any(|option| {
                        option.option_id == choice.option_id && option_available(option, choices)
                    })
            })
        })
        .map(|group| format!("Choose {}", group.label))
        .collect()
}

pub(super) fn retain_available(
    groups: &[rpc::JourneyGroup],
    choices: &mut Vec<rpc::JourneyChoice>,
) {
    let selected_choices = choices.clone();
    choices.retain(|choice| {
        groups.iter().any(|group| {
            group.group_id == choice.group_id
                && group.options.iter().any(|option| {
                    option.option_id == choice.option_id
                        && option_available(option, &selected_choices)
                })
        })
    });
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn species_groups_only_apply_to_the_selected_species_and_hidden_choices_are_removed() {
        let groups = crate::gameplay::hero_catalog::species_choice_groups();
        for (species, expected) in [
            ("dwarf", vec![]),
            ("dragonborn", vec!["draconic-ancestry"]),
            (
                "elf",
                vec!["elven-lineage", "keen-senses", "species-spellcasting"],
            ),
            ("gnome", vec!["gnomish-lineage", "species-spellcasting"]),
            ("goliath", vec!["giant-ancestry"]),
            ("human", vec!["species-size", "human-skill", "versatile"]),
            (
                "tiefling",
                vec!["species-size", "species-spellcasting", "fiendish-legacy"],
            ),
        ] {
            let mut choices = vec![rpc::JourneyChoice {
                group_id: "species".to_owned(),
                option_id: species.to_owned(),
            }];
            let applicable = groups
                .iter()
                .filter(|group| group_available(group, &choices));
            let mut actual = applicable
                .map(|group| group.group_id.as_str())
                .collect::<Vec<_>>();
            actual.sort();
            let mut expected = expected;
            expected.sort();
            assert_eq!(actual, expected, "{species}");
            assert_eq!(
                missing_groups(&groups, &choices).len(),
                expected.len(),
                "{species}"
            );
            choices.extend(groups.iter().map(|group| rpc::JourneyChoice {
                group_id: group.group_id.clone(),
                option_id: group.options[0].option_id.clone(),
            }));
            retain_available(&groups, &mut choices);
            assert_eq!(choices.len(), expected.len(), "{species}");
            assert!(
                choices
                    .iter()
                    .all(|choice| expected.contains(&choice.group_id.as_str()))
            );
        }
    }
}
