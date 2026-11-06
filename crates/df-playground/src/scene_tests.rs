mod media;
mod overlay;
mod slide;
mod slots;

use crate::fixture::{Caption, Choice, Fixture, PartyMember, UnavailableChoice};
use crate::limits::{MAXIMUM_CAPTION_CHARACTERS, MAXIMUM_CHOICES_PER_LIST, MAXIMUM_PARTY_MEMBERS};
use crate::overlay::Edit;
use crate::scene::{SceneChange, SceneError, SceneOverlay};

const FIXTURE: &str = r##"{
    "name": "Base",
    "tv": {
        "place": "Here", "status": {"kind": "ok"}, "elements": [],
        "caption": {"speaker": "Marta", "text": "Original line."},
        "party": [
            {"name": "Pip", "hit_points": 7, "maximum_hit_points": 9},
            {"name": "Vex", "hit_points": 4, "maximum_hit_points": 10, "conditions": ["Frightened"]}
        ]
    },
    "phone": {
        "hero": "Pip", "status": {"kind": "ok"},
        "suggested": [{"label": "Sit down"}],
        "groups": [
            {"name": "Talk", "choices": [{"label": "Greet"}]},
            {"name": "Look", "choices": [{"label": "Search"}]}
        ],
        "unavailable": [{"label": "Cast Light", "reason": "No slots"}]
    }
}"##;

fn base() -> Fixture {
    Fixture::parse(FIXTURE).expect("the fixture reads")
}

fn choice(label: &str) -> Choice {
    Choice {
        label: label.to_owned(),
        explain: None,
    }
}

fn member(name: &str, hit_points: u32, maximum: u32) -> PartyMember {
    PartyMember {
        name: name.to_owned(),
        hit_points,
        maximum_hit_points: maximum,
        conditions: Vec::new(),
        color: None,
    }
}

fn drawn(overlay: &SceneOverlay) -> Fixture {
    let mut fixture = base();
    overlay.apply_to(&mut fixture);
    fixture
}

#[test]
fn an_empty_overlay_changes_nothing() {
    let overlay = SceneOverlay::default();
    assert!(overlay.is_empty());
    assert_eq!(drawn(&overlay), base());
}

#[test]
fn a_caption_replaces_the_fixtures_and_can_be_cleared() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Caption {
                speaker: Some("Narrator".into()),
                text: "A new line.".into(),
            },
            &base(),
        )
        .expect("applies");
    assert_eq!(
        drawn(&overlay).tv.caption,
        Some(Caption {
            speaker: Some("Narrator".into()),
            text: "A new line.".into()
        })
    );
    overlay
        .apply(SceneChange::ClearCaption {}, &base())
        .expect("applies");
    assert_eq!(overlay.caption, Edit::Clear);
    assert_eq!(drawn(&overlay).tv.caption, None);
}

#[test]
fn a_caption_must_have_text_and_stay_within_the_limit() {
    let mut overlay = SceneOverlay::default();
    let empty = SceneChange::Caption {
        speaker: None,
        text: "   ".into(),
    };
    assert_eq!(
        overlay.apply(empty, &base()),
        Err(SceneError::Empty("the caption"))
    );
    let long = SceneChange::Caption {
        speaker: None,
        text: "x".repeat(MAXIMUM_CAPTION_CHARACTERS + 1),
    };
    assert!(matches!(
        overlay.apply(long, &base()),
        Err(SceneError::TooLong { .. })
    ));
    let blank_speaker = SceneChange::Caption {
        speaker: Some(String::new()),
        text: "Hi".into(),
    };
    assert_eq!(
        overlay.apply(blank_speaker, &base()),
        Err(SceneError::Empty("the speaker"))
    );
    assert!(
        overlay.is_empty(),
        "a refused change leaves the overlay alone"
    );
}

#[test]
fn a_party_change_starts_from_the_fixtures_party() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::PartySet {
                name: "Pip".into(),
                hit_points: Some(3),
                maximum_hit_points: None,
                conditions: Some(vec!["Prone".into()]),
            },
            &base(),
        )
        .expect("applies");
    let party = drawn(&overlay).tv.party;
    assert_eq!(party.len(), 2);
    assert_eq!((party[0].hit_points, party[0].maximum_hit_points), (3, 9));
    assert_eq!(party[0].conditions, vec!["Prone".to_owned()]);
    assert_eq!(party[1], base().tv.party[1]);
}

#[test]
fn members_are_added_removed_and_cleared() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::PartyAdd {
                member: member("Anselm", 12, 12),
            },
            &base(),
        )
        .expect("adds");
    assert_eq!(drawn(&overlay).tv.party.len(), 3);
    overlay
        .apply(SceneChange::PartyRemove { name: "Vex".into() }, &base())
        .expect("removes");
    let names: Vec<String> = drawn(&overlay)
        .tv
        .party
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(names, vec!["Pip", "Anselm"]);
    overlay
        .apply(SceneChange::PartyClear {}, &base())
        .expect("clears");
    assert!(drawn(&overlay).tv.party.is_empty());
}

#[test]
fn a_party_change_that_cannot_apply_is_refused_and_says_why() {
    let mut overlay = SceneOverlay::default();
    assert_eq!(
        overlay.apply(
            SceneChange::PartyAdd {
                member: member("Pip", 1, 1)
            },
            &base()
        ),
        Err(SceneError::DuplicateMember("Pip".into()))
    );
    assert_eq!(
        overlay.apply(
            SceneChange::PartyRemove {
                name: "Ghost".into()
            },
            &base()
        ),
        Err(SceneError::UnknownMember("Ghost".into()))
    );
    assert_eq!(
        overlay.apply(
            SceneChange::PartySet {
                name: "Ghost".into(),
                hit_points: Some(1),
                maximum_hit_points: None,
                conditions: None
            },
            &base()
        ),
        Err(SceneError::UnknownMember("Ghost".into()))
    );
    assert_eq!(
        overlay.apply(
            SceneChange::PartyAdd {
                member: member("Zed", 0, 0)
            },
            &base()
        ),
        Err(SceneError::InvalidMaximum)
    );
    assert_eq!(
        overlay.apply(
            SceneChange::PartySet {
                name: "Pip".into(),
                hit_points: None,
                maximum_hit_points: Some(0),
                conditions: None
            },
            &base()
        ),
        Err(SceneError::InvalidMaximum)
    );
    assert!(overlay.is_empty());
}

#[test]
fn the_party_strip_has_a_limit() {
    let mut overlay = SceneOverlay::default();
    for index in 0..MAXIMUM_PARTY_MEMBERS - 2 {
        overlay
            .apply(
                SceneChange::PartyAdd {
                    member: member(&format!("M{index}"), 1, 1),
                },
                &base(),
            )
            .expect("adds");
    }
    assert_eq!(
        overlay.apply(
            SceneChange::PartyAdd {
                member: member("Extra", 1, 1)
            },
            &base()
        ),
        Err(SceneError::TooManyMembers)
    );
}

#[test]
fn suggested_choices_are_replaced_and_may_break_the_contract_so_the_page_can_show_it() {
    let mut overlay = SceneOverlay::default();
    let choices: Vec<Choice> = (0..6)
        .map(|index| choice(&format!("Option {index} has many words")))
        .collect();
    overlay
        .apply(
            SceneChange::Suggested {
                choices: choices.clone(),
            },
            &base(),
        )
        .expect("accepted");
    assert_eq!(drawn(&overlay).phone.suggested, choices);
}

#[test]
fn a_group_is_replaced_added_or_removed_by_name() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Group {
                name: "Talk".into(),
                choices: vec![choice("Haggle")],
            },
            &base(),
        )
        .expect("replaces");
    overlay
        .apply(
            SceneChange::Group {
                name: "Move".into(),
                choices: vec![choice("Dash")],
            },
            &base(),
        )
        .expect("adds");
    overlay
        .apply(
            SceneChange::Group {
                name: "Look".into(),
                choices: Vec::new(),
            },
            &base(),
        )
        .expect("removes");
    let groups = drawn(&overlay).phone.groups;
    let described: Vec<(String, Vec<String>)> = groups
        .into_iter()
        .map(|group| {
            (
                group.name,
                group.choices.into_iter().map(|c| c.label).collect(),
            )
        })
        .collect();
    assert_eq!(
        described,
        vec![
            ("Talk".to_owned(), vec!["Haggle".to_owned()]),
            ("Move".to_owned(), vec!["Dash".to_owned()]),
        ]
    );
}

#[test]
fn unavailable_choices_need_a_label_and_a_reason() {
    let mut overlay = SceneOverlay::default();
    let missing_reason = UnavailableChoice {
        label: "Cast".into(),
        reason: String::new(),
    };
    assert_eq!(
        overlay.apply(
            SceneChange::Unavailable {
                choices: vec![missing_reason]
            },
            &base()
        ),
        Err(SceneError::Empty("a reason"))
    );
    let fine = UnavailableChoice {
        label: "Cast".into(),
        reason: "No slots".into(),
    };
    overlay
        .apply(
            SceneChange::Unavailable {
                choices: vec![fine.clone()],
            },
            &base(),
        )
        .expect("applies");
    assert_eq!(drawn(&overlay).phone.unavailable, vec![fine]);
}

#[test]
fn a_list_of_choices_has_a_limit_and_a_label_must_not_be_empty() {
    let mut overlay = SceneOverlay::default();
    let too_many: Vec<Choice> = (0..=MAXIMUM_CHOICES_PER_LIST)
        .map(|i| choice(&i.to_string()))
        .collect();
    assert!(matches!(
        overlay.apply(SceneChange::Suggested { choices: too_many }, &base()),
        Err(SceneError::TooManyChoices { .. })
    ));
    assert_eq!(
        overlay.apply(
            SceneChange::Suggested {
                choices: vec![choice("")]
            },
            &base()
        ),
        Err(SceneError::Empty("a choice label"))
    );
}

#[test]
fn clearing_the_choices_empties_all_three_lists_and_reset_returns_to_the_fixture() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(SceneChange::ClearChoices {}, &base())
        .expect("clears");
    let phone = drawn(&overlay).phone;
    assert!(phone.suggested.is_empty() && phone.groups.is_empty() && phone.unavailable.is_empty());
    overlay
        .apply(SceneChange::Reset {}, &base())
        .expect("resets");
    assert!(overlay.is_empty());
    assert_eq!(drawn(&overlay), base());
}

#[test]
fn a_change_is_written_and_read_by_its_name_and_refuses_unknown_keys() {
    let change = SceneChange::PartyRemove { name: "Vex".into() };
    let text = serde_json::to_string(&change).expect("writes");
    assert_eq!(text, r#"{"change":"party_remove","name":"Vex"}"#);
    assert_eq!(
        serde_json::from_str::<SceneChange>(&text).expect("reads"),
        change
    );
    assert!(
        serde_json::from_str::<SceneChange>(r#"{"change":"clear_caption","extra":1}"#).is_err()
    );
    assert!(serde_json::from_str::<SceneChange>(r#"{"change":"dance"}"#).is_err());
}

#[test]
fn an_overlay_survives_a_round_trip_through_json() {
    let mut overlay = SceneOverlay::default();
    overlay
        .apply(
            SceneChange::Caption {
                speaker: None,
                text: "Hi".into(),
            },
            &base(),
        )
        .expect("applies");
    overlay
        .apply(SceneChange::PartyClear {}, &base())
        .expect("applies");
    let text = serde_json::to_string(&overlay).expect("writes");
    assert_eq!(
        serde_json::from_str::<SceneOverlay>(&text).expect("reads"),
        overlay
    );
}
