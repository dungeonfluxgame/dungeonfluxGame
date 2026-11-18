//! The quest record's pure rules: what a quest must say, a board, a quest taken, and the coin the
//! advance is paid in.

use super::quest::{
    Quest, QuestRecord, QuestStanding, accept, check_quest, complete, offer, share_of,
};
use super::request::WorldError;

fn quest(id: &str, reward_gp: u64, advance_gp: u64) -> Quest {
    Quest {
        id: id.to_owned(),
        title: format!("The {id} job"),
        giver: "the guild master".to_owned(),
        reward_gp,
        advance_gp,
        site: "beyond the causeway".to_owned(),
        clues: vec!["shrine_verse".to_owned(), "elder_token".to_owned()],
    }
}

#[test]
fn a_quest_says_what_it_is_and_pays_no_more_in_advance_than_its_reward() {
    assert!(check_quest(&quest("sealed_tomb", 600, 50)).is_ok());
    assert!(matches!(
        check_quest(&quest("sealed_tomb", 50, 60)),
        Err(WorldError::InvalidQuest { .. })
    ));
    assert!(check_quest(&quest("Sealed Tomb", 600, 50)).is_err());
    let mut twice = quest("sealed_tomb", 600, 50);
    twice.clues.push("shrine_verse".to_owned());
    assert!(check_quest(&twice).is_err());
    let mut blank = quest("sealed_tomb", 600, 50);
    blank.giver = "  ".to_owned();
    assert!(check_quest(&blank).is_err());
}

#[test]
fn a_board_replaces_the_offered_quests_and_keeps_the_taken_ones() {
    let mut records = Vec::new();
    offer(&mut records, vec![quest("a", 100, 10), quest("b", 200, 20)]).unwrap();
    accept(&mut records, "a", &["hero-1".to_owned()]).unwrap();
    offer(&mut records, vec![quest("c", 300, 30)]).unwrap();
    let standing: Vec<(&str, QuestStanding)> = records
        .iter()
        .map(|record: &QuestRecord| (record.quest.id.as_str(), record.standing))
        .collect();
    assert_eq!(
        standing,
        vec![
            ("a", QuestStanding::Accepted),
            ("c", QuestStanding::Offered)
        ]
    );
    // A later day's board offers the taken quest again: it stays theirs, never offered anew.
    offer(&mut records, vec![quest("a", 100, 10), quest("b", 200, 20)]).unwrap();
    let standing: Vec<(&str, QuestStanding)> = records
        .iter()
        .map(|record: &QuestRecord| (record.quest.id.as_str(), record.standing))
        .collect();
    assert_eq!(
        standing,
        vec![
            ("a", QuestStanding::Accepted),
            ("b", QuestStanding::Offered)
        ]
    );
    assert!(offer(&mut records, vec![quest("b", 200, 20), quest("b", 200, 20)]).is_err());
    assert!(offer(&mut records, Vec::new()).is_err());
}

#[test]
fn a_quest_is_taken_once_by_named_heroes() {
    let mut records = Vec::new();
    offer(&mut records, vec![quest("a", 100, 10)]).unwrap();
    assert!(matches!(
        accept(&mut records, "z", &["hero-1".to_owned()]),
        Err(WorldError::UnknownQuest { .. })
    ));
    assert!(accept(&mut records, "a", &[]).is_err());
    assert!(
        accept(
            &mut records,
            "a",
            &["hero-1".to_owned(), "hero-1".to_owned()]
        )
        .is_err()
    );
    let heroes = ["hero-1".to_owned(), "hero-2".to_owned()];
    let (taken, newly) = accept(&mut records, "a", &heroes).unwrap();
    assert_eq!(taken.id, "a");
    assert_eq!(newly, heroes);
    assert_eq!(records[0].accepted_by, heroes);
}

#[test]
fn taking_a_quest_again_is_theirs_already_and_pays_no_one_twice() {
    let mut records = Vec::new();
    offer(&mut records, vec![quest("sealed_tomb", 600, 50)]).unwrap();
    let heroes = ["hero-10".to_owned(), "hero-9".to_owned()];
    accept(&mut records, "sealed_tomb", &heroes).unwrap();
    // A later day: the same board, the same pick.
    offer(&mut records, vec![quest("sealed_tomb", 600, 50)]).unwrap();
    let (_, newly) = accept(&mut records, "sealed_tomb", &heroes).unwrap();
    assert!(newly.is_empty(), "nobody takes it anew: {newly:?}");
    // A hero who joins later takes it once, and only they are new.
    let joined = ["hero-10".to_owned(), "hero-11".to_owned()];
    let (_, newly) = accept(&mut records, "sealed_tomb", &joined).unwrap();
    assert_eq!(newly, vec!["hero-11".to_owned()]);
    assert_eq!(
        records[0].accepted_by,
        vec![
            "hero-10".to_owned(),
            "hero-9".to_owned(),
            "hero-11".to_owned()
        ]
    );
}

// TODO(DEV-12): the advance through gain_coins on a store fixture: into held gold, into a new gold
// item when the purse is empty, and a refusal that pays nothing (the QUEST pay_refused line).

#[test]
fn a_quest_completes_once_and_its_reward_splits_evenly() {
    let mut records = Vec::new();
    offer(&mut records, vec![quest("sealed_tomb", 600, 50)]).unwrap();
    let heroes = ["hero-9".to_owned(), "hero-10".to_owned()];
    assert!(complete(&mut records, "sealed_tomb").is_err());
    accept(&mut records, "sealed_tomb", &heroes).unwrap();
    let (_, by) = complete(&mut records, "sealed_tomb").unwrap();
    assert_eq!(by, heroes.to_vec());
    assert_eq!(records[0].standing, QuestStanding::Completed);
    assert!(complete(&mut records, "sealed_tomb").is_err());
    assert_eq!((share_of(600, 2, 0), share_of(600, 2, 1)), (300, 300));
    assert_eq!((share_of(601, 2, 0), share_of(601, 2, 1)), (301, 300));
}
