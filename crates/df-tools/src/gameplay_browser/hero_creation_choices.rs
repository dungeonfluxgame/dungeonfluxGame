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
    unselected_groups(groups, choices)
        .into_iter()
        .map(|group| format!("Choose {}", group.label))
        .collect()
}

pub(super) fn unselected_groups<'a>(
    groups: &'a [rpc::JourneyGroup],
    choices: &[rpc::JourneyChoice],
) -> Vec<&'a rpc::JourneyGroup> {
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
    fn dwarf_fighter_missing_choices_shrink_after_each_selection_and_exclude_other_ancestries() {
        let mut groups = crate::gameplay::hero_catalog::species_choice_groups();
        groups.extend(
            [
                ("alignment", "Alignment"),
                ("language-1", "First language"),
                ("language-2", "Second language"),
                ("skill-1", "First Fighter skill"),
                ("skill-2", "Second Fighter skill"),
                ("gaming-set", "Gaming set"),
            ]
            .map(|(id, label)| rpc::JourneyGroup {
                group_id: id.to_owned(),
                label: label.to_owned(),
                options: vec![rpc::JourneyOption {
                    option_id: "chosen".to_owned(),
                    ..Default::default()
                }],
            }),
        );
        let mut choices = vec![rpc::JourneyChoice {
            group_id: "species".to_owned(),
            option_id: "dwarf".to_owned(),
        }];
        let expected = [
            "alignment",
            "language-1",
            "language-2",
            "skill-1",
            "skill-2",
            "gaming-set",
        ];
        for completed in 0..=expected.len() {
            let missing = unselected_groups(&groups, &choices);
            assert_eq!(
                missing
                    .iter()
                    .map(|group| group.group_id.as_str())
                    .collect::<Vec<_>>(),
                expected[completed..]
            );
            if let Some(id) = expected.get(completed) {
                choices.push(rpc::JourneyChoice {
                    group_id: (*id).to_owned(),
                    option_id: "chosen".to_owned(),
                });
            }
        }
    }

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
