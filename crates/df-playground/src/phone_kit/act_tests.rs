//! The Act panel's decisions: the four-card rule, the live choice list and the sample composer.

use crate::choices::build;
use crate::composer::{ComposerState, MicLook};
use crate::fixture::Fixture;
use crate::phone_kit::act::{ActChoices, ActionCard, ComposerLook, ComposerSample};

fn card(label: &str) -> ActionCard {
    ActionCard {
        label: label.to_owned(),
        explain: None,
        glyph: None,
        lit: false,
    }
}

#[test]
fn an_empty_choice_list_is_empty_and_a_card_makes_it_not() {
    assert!(ActChoices::default().is_empty());
    let choices = ActChoices {
        suggested: vec![card("a")],
        ..ActChoices::default()
    };
    assert!(!choices.is_empty());
}

#[test]
fn the_live_choice_list_becomes_cards_with_its_groups_and_reasons() {
    let fixture = Fixture::parse(
        r#"{"name":"B","tv":{"place":"P","status":{"kind":"ok"},"elements":[]},
        "phone":{"hero":"H","status":{"kind":"ok"},
        "suggested":[{"label":"Open the door","explain":"It is unlocked"}],
        "groups":[{"name":"Look","choices":[{"label":"Listen","explain":null}]}],
        "unavailable":[{"label":"Cast","reason":"No spells yet"}]}}"#,
    )
    .expect("reads");
    let view = build(&fixture.phone, true);
    let choices = ActChoices::from(&view);
    assert_eq!(choices.suggested[0].label, "Open the door");
    assert_eq!(
        choices.suggested[0].explain.as_deref(),
        Some("It is unlocked")
    );
    assert_eq!(choices.left_out, 0);
    assert_eq!(choices.groups[0].name, "Look");
    assert_eq!(choices.unavailable[0].reason, "No spells yet");
}

#[test]
fn the_sample_composer_is_the_live_composer_in_the_state_it_shows() {
    let sample = |look, text: &str| ComposerSample {
        look,
        text: text.to_owned(),
        people: vec!["Mara Voss".to_owned()],
    };
    let idle = sample(ComposerLook::Idle, "").view();
    assert_eq!(idle.state, ComposerState::Idle);
    assert_eq!(idle.mic, MicLook::Ready);
    assert_eq!(idle.targets[0].name, "Mara Voss");
    let listening = sample(ComposerLook::Listening, "").view();
    assert_eq!(listening.mic, MicLook::Recording);
    assert_eq!(listening.live.as_deref(), Some("0:04"));
    let transcript = sample(ComposerLook::Transcript, "Hello there").view();
    assert!(transcript.send_enabled);
    assert!(!sample(ComposerLook::Transcript, "  ").view().send_enabled);
}
