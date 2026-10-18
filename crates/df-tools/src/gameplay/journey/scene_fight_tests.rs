//! Fight-start narration wording: the lines that tell how a fight began, in the order the
//! table must hear them (the aggressor's blow first, then who joins, then who flees).

use super::{Person, Plan, announce};

fn person(id: &str, name: &str) -> Person {
    Person {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

#[test]
fn the_fight_start_is_told_before_anyone_acts_in_join_flee_order() {
    let plan = Plan {
        target: person("shop-adventurer", "Corin"),
        joiners: vec![person("shop-soldier", "Jessa")],
        fleers: vec![person("merchant", "Marek")],
        out: vec![person("shop-keeper", "Tovin")],
    };
    let (lines, audit) = announce("Brakka", &plan);
    assert_eq!(
        lines,
        [
            "Brakka attacks Corin.",
            "Jessa joins the fight against Brakka.",
            "Marek flees, shouting for the watch.",
            "Tovin steps back and stays out of it.",
        ]
    );
    // The offer's button caption is never a narration line.
    assert!(lines.iter().all(|line| !line.contains("Start a fight")));
    assert!(audit.contains("aggressor=\"Brakka\""));
}
