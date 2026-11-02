use crate::choices::{MAXIMUM_SUGGESTED, Warning, build};
use crate::fixture::{Choice, ChoiceGroup, PhoneScene, Status, UnavailableChoice};

fn choice(label: &str) -> Choice {
    Choice {
        label: label.to_owned(),
        explain: None,
    }
}

fn group(name: &str, labels: &[&str]) -> ChoiceGroup {
    ChoiceGroup {
        name: name.to_owned(),
        choices: labels.iter().map(|label| choice(label)).collect(),
    }
}

fn scene() -> PhoneScene {
    PhoneScene {
        hero: "Pip".to_owned(),
        goal: None,
        status: Status::Ok,
        narration: Vec::new(),
        suggested: Vec::new(),
        groups: Vec::new(),
        unavailable: Vec::new(),
        note: None,
        image: None,
    }
}

#[test]
fn at_most_four_suggested_choices_are_shown_and_the_rest_are_counted_and_warned() {
    let mut scene = scene();
    scene.suggested = (0..6)
        .map(|index| choice(&format!("Option {index}")))
        .collect();
    let view = build(&scene, false);
    assert_eq!(view.suggested.len(), MAXIMUM_SUGGESTED);
    assert_eq!(view.left_out, 2);
    assert_eq!(view.warnings, vec![Warning::TooManySuggested { given: 6 }]);
}

#[test]
fn a_label_of_more_than_four_words_is_warned_wherever_it_appears() {
    let mut scene = scene();
    scene.suggested = vec![choice("Greet the baker warmly now")];
    scene.groups = vec![group("Talk", &["Ask about the missing courier today"])];
    scene.unavailable = vec![UnavailableChoice {
        label: "Cast a spell on the whole room".to_owned(),
        reason: "No slots".to_owned(),
    }];
    let view = build(&scene, true);
    assert_eq!(
        view.warnings
            .iter()
            .filter(|warning| matches!(warning, Warning::LabelTooLong { .. }))
            .count(),
        3
    );
}

#[test]
fn the_fixed_groups_come_in_their_fixed_order_and_unknown_ones_last_with_a_warning() {
    let mut scene = scene();
    scene.groups = vec![
        group("Look", &["Study the runes"]),
        group("Sneak", &["Hide"]),
        group("Attack", &["Strike"]),
    ];
    let view = build(&scene, false);
    let names: Vec<&str> = view
        .groups
        .iter()
        .map(|group| group.name.as_str())
        .collect();
    assert_eq!(names, vec!["Attack", "Look", "Sneak"]);
    assert_eq!(
        view.warnings,
        vec![Warning::UnknownGroup {
            name: "Sneak".into()
        }]
    );
}

#[test]
fn an_empty_group_is_left_out_and_a_repeated_group_is_ignored_with_a_warning() {
    let mut scene = scene();
    scene.groups = vec![
        group("Talk", &[]),
        group("Move", &["Step closer"]),
        group("Move", &["Run away"]),
    ];
    let view = build(&scene, false);
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].choices[0].label, "Step closer");
    assert!(view.warnings.contains(&Warning::EmptyGroup {
        name: "Talk".into()
    }));
    assert!(view.warnings.contains(&Warning::DuplicateGroup {
        name: "Move".into()
    }));
}

#[test]
fn unavailable_choices_are_hidden_until_asked_for() {
    let mut scene = scene();
    scene.unavailable = vec![UnavailableChoice {
        label: "Fire Bolt".to_owned(),
        reason: "Nothing here to aim at".to_owned(),
    }];
    assert!(build(&scene, false).unavailable.is_empty());
    assert_eq!(build(&scene, true).unavailable.len(), 1);
}

#[test]
fn a_clean_scene_has_no_warnings() {
    let mut scene = scene();
    scene.suggested = vec![choice("Greet Mara"), choice("Look around")];
    scene.groups = vec![group("Talk", &["Ask about work"])];
    assert!(build(&scene, false).warnings.is_empty());
}

#[test]
fn every_warning_reads_as_a_sentence_naming_what_is_wrong() {
    let text = Warning::LabelTooLong {
        label: "Greet the baker warmly now".into(),
        words: 5,
    }
    .to_string();
    assert!(text.contains("Greet the baker warmly now") && text.contains("at most 4"));
}
