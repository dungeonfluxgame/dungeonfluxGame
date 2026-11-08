//! What a character calls the others: a stranger's id (which may carry a name) is never shown,
//! only a handle; a known name's id is; the answer's handles read back to ids.

use df_content::persona::{Appraisal, Attention, Attitude, Noticed, PersonaState, Relationship};

use super::handles::Handles;

fn state(known: &[&str]) -> PersonaState {
    PersonaState {
        id: "weapon_seller".to_owned(),
        turn: 0,
        emotions: Vec::new(),
        mood: Default::default(),
        attention: Attention::default(),
        relationships: Vec::new(),
        known_names: known.iter().map(|id| (*id).to_owned()).collect(),
    }
}

#[test]
fn a_stranger_is_a_handle_and_a_known_name_its_id() {
    let handles = Handles::new(
        &state(&["warrior"]),
        ["mara_adventurer", "warrior", "jory_adventurer"],
    );
    assert_eq!(handles.shown("mara_adventurer"), "person-1");
    assert_eq!(handles.shown("warrior"), "warrior");
    assert_eq!(handles.shown("jory_adventurer"), "person-2");
    assert_eq!(
        handles.shown("bram_adventurer"),
        "someone",
        "an id not told of has no name"
    );
    assert_eq!(
        handles.id_of("person-2").as_deref(),
        Some("jory_adventurer")
    );
    assert_eq!(handles.id_of("person-9"), None);
}

#[test]
fn recent_labels_briefs_and_the_state_carry_no_strangers_id() {
    let mut met = state(&[]);
    met.relationships = vec![Relationship {
        toward: "mara_adventurer".to_owned(),
        attitude: Attitude::Indifferent,
        rapport: 4,
    }];
    met.attention = Attention {
        focus: Some("mara_adventurer".to_owned()),
        topic: None,
        noticed: vec![Noticed {
            speaker: "mara_adventurer".to_owned(),
            gist: "asked about the fuller".to_owned(),
        }],
    };
    let handles = Handles::new(&met, ["mara_adventurer"]);
    assert_eq!(
        handles.recent_line("mara_adventurer: How was the fuller cut?"),
        "person-1: How was the fuller cut?"
    );
    assert_eq!(handles.recent_line("no label here"), "no label here");
    let shown = serde_json::to_string(&handles.state(&met)).unwrap();
    assert!(!shown.contains("mara"), "{shown}");
}

#[test]
fn an_answers_handles_read_back_to_ids() {
    let handles = Handles::new(&state(&[]), ["mara_adventurer"]);
    let appraisal = Appraisal {
        emotions: Vec::new(),
        toward: Some("person-1".to_owned()),
        rapport_delta: Some(3),
        attention: Attention {
            focus: Some("person-1".to_owned()),
            topic: None,
            noticed: vec![Noticed {
                speaker: "person-1".to_owned(),
                gist: "doubted the runes".to_owned(),
            }],
        },
        tone: None,
    };
    let read = handles.appraisal_ids(appraisal);
    assert_eq!(read.toward.as_deref(), Some("mara_adventurer"));
    assert_eq!(read.attention.focus.as_deref(), Some("mara_adventurer"));
    assert_eq!(read.attention.noticed[0].speaker, "mara_adventurer");
}

#[test]
fn the_character_itself_known_names_and_unknown_labels_are_shown_as_the_rule_says() {
    let handles = Handles::new(&state(&["warrior"]), ["mara_adventurer"]);
    assert_eq!(handles.shown("weapon_seller"), "weapon_seller", "itself");
    assert_eq!(
        handles.shown("warrior"),
        "warrior",
        "a known name, even unlisted"
    );
    assert_eq!(
        handles.recent_line("weapon_seller: Fair trade."),
        "weapon_seller: Fair trade."
    );
    assert_eq!(
        handles.recent_line("Mara: Hello."),
        "someone: Hello.",
        "a display name"
    );
    assert_eq!(
        handles.recent_line("bram_adventurer: Hm."),
        "someone: Hm.",
        "an id gone"
    );
}

#[test]
fn a_stall_keeper_in_the_world_facts_is_shown_as_the_character_knows_them() {
    let facts = serde_json::json!({"stalls": [{"keeper": "weapon_seller", "items": []}]});
    let mut stranger = state(&[]);
    stranger.id = "mara_adventurer".to_owned();
    let seen = Handles::new(&stranger, ["weapon_seller"]).world_facts(facts.clone());
    assert_eq!(seen["stalls"][0]["keeper"], "person-1");
    let own = Handles::new(&state(&[]), ["mara_adventurer"]).world_facts(facts);
    assert_eq!(
        own["stalls"][0]["keeper"], "weapon_seller",
        "the keeper's own stall"
    );
}

#[test]
fn a_text_carrying_a_storyteller_handle_is_seen() {
    let handles = Handles::author(["warrior", "harbour_merchant"]);
    assert!(handles.carried_by("Ask person-2 where he was."));
    assert!(!handles.carried_by("Ask the merchant where he was."));
}
