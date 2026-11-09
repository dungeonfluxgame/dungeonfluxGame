//! The journal's title, entries and page.

use crate::kit::journal::{JOURNAL_ROWS_AT_MOST, Journal, QuestDetail, QuestEntry, QuestState};
use crate::kit::kit_tests::assert_css_words_are_serde_names;

#[test]
fn a_journal_names_its_page_and_keeps_titled_entries_in_order() {
    let journal: Journal = serde_json::from_str(
        r#"{"entries":[
            {"id":"a","title":"The Missing Caravan","state":"current"},
            {"id":"b","title":"  "},
            {"id":"c","title":"Rats","state":"done"}
        ],"more":" +2 more "}"#,
    )
    .expect("a journal reads");
    assert_eq!(journal.title(), "Quests");
    let ids: Vec<&str> = journal
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids, vec!["a", "c"]);
    assert_eq!(journal.entries()[0].state, QuestState::Current);
    assert_eq!(journal.more().as_deref(), Some("+2 more"));
    assert!(journal.detail().is_none());
}

#[test]
fn a_page_with_neither_title_nor_body_is_not_drawn() {
    let journal: Journal =
        serde_json::from_str(r#"{"title":"Lore","entries":[],"detail":{"title":" ","body":""}}"#)
            .expect("a journal reads");
    assert_eq!(journal.title(), "Lore");
    assert!(journal.detail().is_none());
    assert_css_words_are_serde_names(
        &[
            QuestState::Active,
            QuestState::Current,
            QuestState::Done,
            QuestState::Failed,
        ],
        QuestState::key,
    );
}

#[test]
fn the_page_draws_at_most_six_whole_rows() {
    let entry = |id: usize| QuestEntry {
        id: id.to_string(),
        title: format!("Quest {id}"),
        state: QuestState::Active,
        marker: None,
    };
    let journal = Journal {
        title: None,
        entries: (0..9).map(entry).collect(),
        more: Some("+3 more".to_owned()),
        detail: Some(QuestDetail {
            title: "The Missing Caravan".to_owned(),
            body: "The Greyhaven caravan vanished.".to_owned(),
            sketch: None,
        }),
    };
    let ids: Vec<&str> = journal
        .entries()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect();
    assert_eq!(ids.len(), JOURNAL_ROWS_AT_MOST);
    assert_eq!(ids, vec!["0", "1", "2", "3", "4", "5"]);
    assert_eq!(journal.more().as_deref(), Some("+3 more"));
}

#[test]
fn rows_past_the_page_without_the_hosts_line_are_counted() {
    let entry = |id: usize| QuestEntry {
        id: id.to_string(),
        title: format!("Quest {id}"),
        state: QuestState::Active,
        marker: None,
    };
    let journal = Journal {
        title: None,
        entries: (0..10).map(entry).collect(),
        more: None,
        detail: None,
    };
    assert_eq!(journal.entries().len(), JOURNAL_ROWS_AT_MOST);
    assert_eq!(journal.more().as_deref(), Some("+4"));
    let fits = Journal {
        entries: (0..JOURNAL_ROWS_AT_MOST).map(entry).collect(),
        ..journal
    };
    assert_eq!(fits.more(), None);
}
