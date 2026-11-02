//! The attachments are a shared snapshot with a version, can be cleared, and are bounded to the
//! wire frame (#1684, #1685, #1686, #1690).

use serde_json::{Value, json};

use crate::outcome::{Decline, DeclineCode};
use crate::ports::Audience;
use crate::private_data::{Narration, Secret};
use crate::projection::bounds::{
    MAXIMUM_TEXT_BYTES, NARRATION_BUDGET_BYTES, NOTE_BUDGET_BYTES, OFFERS_BUDGET_BYTES,
    PHONE_NARRATION_LINES,
};
use crate::projection::{
    AttachmentError, Attachments, OfferEntry, OfferId, ProjectionStatus, StoryLine, project_seat,
    project_shared,
};
use crate::projection_tests::{at, fact_id, json_of, public_line, table_with_two_heroes, text};
use crate::table_state::TableState;
use crate::table_state_tests::seat;
use crate::testkit::correlation;

fn seat_json(table: &TableState, attachments: &Attachments, number: u64) -> Value {
    json_of(&project_seat(&table.snapshot(), attachments, seat(number)).expect("seated"))
}

fn shared_json(table: &TableState, attachments: &Attachments) -> Value {
    json_of(&project_shared(&table.snapshot(), attachments))
}

fn lines(view: &Value) -> Vec<&str> {
    view["view"]["narration"]
        .as_array()
        .expect("an array")
        .iter()
        .map(|line| line.as_str().expect("a string"))
        .collect()
}

fn declined(reason: &str) -> Decline {
    Decline::new(DeclineCode::NotViable, reason).expect("a reason")
}

// A cheap snapshot and a version (RES30).

#[test]
fn a_snapshot_is_unaffected_by_a_later_publish_and_shares_what_was_not_changed() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_place(crate::table_state_tests::scene(10), text("The cellar"))
        .unwrap();
    attachments.push_public(public_line("one")).unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("a fact".to_owned()))
        .unwrap();
    let snapshot = attachments.snapshot();
    let before = (
        shared_json(&table, &snapshot),
        seat_json(&table, &snapshot, 1),
    );
    assert!(snapshot.shares_public_story_with(&attachments));
    attachments.push_public(public_line("two")).unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(2), Secret::new("another".to_owned()))
        .unwrap();
    attachments
        .set_notice(seat(1), declined("No."), at())
        .unwrap();
    assert_eq!(
        (
            shared_json(&table, &snapshot),
            seat_json(&table, &snapshot, 1)
        ),
        before,
        "the snapshot is what it was"
    );
    assert_ne!(
        shared_json(&table, &attachments),
        before.0,
        "the owner's own view moved on"
    );
    assert!(!snapshot.shares_public_story_with(&attachments));
    assert!(!snapshot.shares_seat_facts_with(&attachments));
    assert!(
        snapshot.shares_places_with(&attachments),
        "the places were not touched"
    );
    assert!(!snapshot.shares_notices_with(&attachments));
}

#[test]
fn taking_a_snapshot_copies_no_collection() {
    let mut attachments = Attachments::new();
    attachments.push_public(public_line("one")).unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("a".to_owned()))
        .unwrap();
    let first = attachments.snapshot();
    let second = attachments.snapshot();
    assert!(first.shares_public_story_with(&second));
    assert!(first.shares_seat_facts_with(&second));
    assert!(first.shares_places_with(&second));
    assert_eq!(first, second);
    for audience in [Audience::Public, Audience::Seat(seat(1))] {
        assert_eq!(
            first.version_for(audience),
            attachments.version_for(audience)
        );
    }
}

#[test]
fn every_publish_and_every_clear_that_changes_something_raises_the_version() {
    let mut attachments = Attachments::new();
    let phone = Audience::Seat(seat(1));
    let mut last = attachments.version_for(phone);
    let mut raised = |attachments: &Attachments, what: &str| {
        let now = attachments.version_for(phone);
        assert!(now > last, "{what} did not raise the version");
        last = now;
    };
    attachments
        .set_place(crate::table_state_tests::scene(10), text("p"))
        .unwrap();
    raised(&attachments, "a place");
    attachments.push_public(public_line("a")).unwrap();
    raised(&attachments, "a public line");
    attachments.push_private(seat(1), public_line("b")).unwrap();
    raised(&attachments, "a private line");
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("f".to_owned()))
        .unwrap();
    raised(&attachments, "a fact");
    attachments.set_seat_readout(seat(1), text("r")).unwrap();
    raised(&attachments, "a seat readout");
    attachments.set_shared_readout(text("r")).unwrap();
    raised(&attachments, "the shared readout");
    attachments.set_activity(ProjectionStatus::Reading).unwrap();
    raised(&attachments, "the activity");
    attachments
        .set_notice(seat(1), declined("No."), at())
        .unwrap();
    raised(&attachments, "a notice");
    attachments.set_seat_offers(seat(1), Vec::new()).unwrap();
    raised(&attachments, "the offers");
    attachments.clear_notice(seat(1)).unwrap();
    raised(&attachments, "clearing the notice");
}

#[test]
fn a_version_rises_only_for_the_audiences_that_can_see_the_publish() {
    let mut attachments = Attachments::new();
    let versions = |attachments: &Attachments| {
        [
            Audience::Public,
            Audience::Seat(seat(1)),
            Audience::Seat(seat(2)),
        ]
        .map(|audience| attachments.version_for(audience))
    };
    let start = versions(&attachments);
    attachments
        .push_private(seat(2), public_line("for seat 2"))
        .unwrap();
    let after_private = versions(&attachments);
    assert_eq!(after_private[0], start[0], "the shared screen cannot tell");
    assert_eq!(after_private[1], start[1], "another seat cannot tell");
    assert!(after_private[2] > start[2]);
    attachments
        .push_public(public_line("for everyone"))
        .unwrap();
    let after_public = versions(&attachments);
    assert!(
        after_public
            .iter()
            .zip(after_private)
            .all(|(now, before)| *now > before)
    );
    attachments.forget_seat(seat(2)).unwrap();
    let after_forget = versions(&attachments);
    assert_eq!(
        after_forget[0], after_public[0],
        "forgetting a seat is private to it"
    );
    assert_eq!(after_forget[1], after_public[1]);
}

#[test]
fn a_clear_that_has_nothing_to_clear_leaves_the_version_alone() {
    let mut attachments = Attachments::new();
    let version = attachments.version_for(Audience::Seat(seat(1)));
    let public = attachments.version_for(Audience::Public);
    attachments.clear_notice(seat(1)).unwrap();
    attachments.clear_seat_readout(seat(1)).unwrap();
    attachments.clear_shared_readout().unwrap();
    attachments.clear_seat_offers(seat(1)).unwrap();
    attachments.forget_seat(seat(1)).unwrap();
    attachments
        .forget_place(crate::table_state_tests::scene(1))
        .unwrap();
    attachments
        .remove_private_fact(seat(1), &fact_id(1))
        .unwrap();
    attachments.expire_notices(at()).unwrap();
    assert_eq!(attachments.version_for(Audience::Seat(seat(1))), version);
    assert_eq!(attachments.version_for(Audience::Public), public);
}

#[test]
fn the_version_is_in_the_projection_so_an_attachment_only_change_at_one_revision_shows() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let first = project_shared(&table.snapshot(), &attachments);
    attachments.push_public(public_line("something")).unwrap();
    let second = project_shared(&table.snapshot(), &attachments);
    assert_eq!(first.revision(), second.revision());
    assert!(second.attachment_version() > first.attachment_version());
    assert_eq!(
        json_of(&second)["attachment_version"],
        json!(second.attachment_version().get())
    );
    assert_eq!(
        second.record().attachment_version,
        second.attachment_version()
    );
}

// Nothing is write-only (#1685).

#[test]
fn a_decline_shows_once_for_the_input_it_answers_and_is_gone_after_the_next_commit() {
    let mut table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_notice(seat(1), declined("That is not possible."), at())
        .unwrap();
    assert_eq!(
        lines(&seat_json(&table, &attachments, 1)).last(),
        Some(&"That is not possible.")
    );
    crate::table_state_tests::apply(
        &mut table,
        3,
        &[crate::effects::TableEffect::TimeAdvanced { advance_s: 6 }],
    )
    .unwrap();
    let after = seat_json(&table, &attachments, 1);
    assert!(!after.to_string().contains("not possible"));
    assert_eq!(after["view"]["suggested"], json!([]));
}

#[test]
fn expiring_notices_frees_the_ones_the_table_has_moved_past_and_keeps_the_current_one() {
    let mut attachments = Attachments::new();
    attachments
        .set_notice(seat(1), declined("old"), correlation(1).revision)
        .unwrap();
    attachments
        .set_notice(seat(2), declined("current"), correlation(2).revision)
        .unwrap();
    attachments.expire_notices(correlation(2).revision).unwrap();
    assert_eq!(attachments.notice(seat(1), correlation(1).revision), None);
    assert_eq!(
        attachments
            .notice(seat(2), correlation(2).revision)
            .map(Decline::reason),
        Some("current")
    );
    let mut expired_only = Attachments::new();
    expired_only
        .set_notice(seat(1), declined("old"), correlation(1).revision)
        .unwrap();
    expired_only
        .expire_notices(correlation(2).revision)
        .unwrap();
    assert!(
        !format!("{expired_only:?}").contains("Decline"),
        "the notice is freed"
    );
}

#[test]
fn the_other_clears_withdraw_exactly_what_they_name() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    attachments
        .set_place(crate::table_state_tests::scene(10), text("The cellar"))
        .unwrap();
    attachments
        .set_shared_readout(text("Shared words."))
        .unwrap();
    attachments
        .set_seat_readout(seat(1), text("Seat words."))
        .unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("keep".to_owned()))
        .unwrap();
    attachments
        .set_private_fact(seat(1), fact_id(2), Secret::new("drop".to_owned()))
        .unwrap();
    assert_eq!(
        seat_json(&table, &attachments, 1)["view"]["note"]["text"],
        "keep\ndrop"
    );
    attachments
        .remove_private_fact(seat(1), &fact_id(2))
        .unwrap();
    assert_eq!(
        seat_json(&table, &attachments, 1)["view"]["note"]["text"],
        "keep"
    );
    attachments
        .remove_private_fact(seat(1), &fact_id(1))
        .unwrap();
    assert!(
        seat_json(&table, &attachments, 1)["view"]
            .get("note")
            .is_none()
    );
    assert_eq!(lines(&seat_json(&table, &attachments, 1)), ["Seat words."]);
    attachments.clear_seat_readout(seat(1)).unwrap();
    assert_ne!(lines(&seat_json(&table, &attachments, 1)), ["Seat words."]);
    assert_eq!(
        shared_json(&table, &attachments)["view"]["caption"]["text"],
        "Shared words."
    );
    attachments.clear_shared_readout().unwrap();
    assert_ne!(
        shared_json(&table, &attachments)["view"]["caption"]["text"],
        "Shared words."
    );
    assert_eq!(
        shared_json(&table, &attachments)["view"]["place"],
        "The cellar"
    );
    attachments
        .forget_place(crate::table_state_tests::scene(10))
        .unwrap();
    assert_eq!(
        shared_json(&table, &attachments)["view"]["place"],
        "Scene 10"
    );
}

#[test]
fn forgetting_a_seat_wipes_everything_private_to_it_and_nothing_of_the_others() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in [1, 2] {
        let who = seat(number);
        attachments
            .push_private(who, public_line(&format!("line {number}")))
            .unwrap();
        attachments
            .set_private_fact(who, fact_id(1), Secret::new(format!("fact {number}")))
            .unwrap();
        attachments
            .set_seat_readout(who, text(&format!("readout {number}")))
            .unwrap();
        attachments
            .set_notice(who, declined(&format!("notice {number}")), at())
            .unwrap();
        attachments
            .set_seat_offers(
                who,
                vec![OfferEntry::new(
                    OfferId::new(&format!("offer-{number}")).unwrap(),
                    text(&format!("label {number}")),
                )],
            )
            .unwrap();
    }
    attachments.forget_seat(seat(1)).unwrap();
    let gone = seat_json(&table, &attachments, 1).to_string();
    for word in [
        "line 1",
        "fact 1",
        "readout 1",
        "notice 1",
        "label 1",
        "offer-1",
    ] {
        assert!(!gone.contains(word), "{word} survived");
    }
    let kept = seat_json(&table, &attachments, 2).to_string();
    for word in ["line 2", "fact 2", "notice 2", "label 2", "offer-2"] {
        assert!(kept.contains(word), "{word} was lost");
    }
}

// Every published text is bounded to the wire frame (#1686).

fn too_long() -> String {
    "x".repeat(MAXIMUM_TEXT_BYTES + 1)
}

fn longest() -> String {
    "x".repeat(MAXIMUM_TEXT_BYTES)
}

#[test]
fn a_text_over_the_limit_is_refused_when_it_is_published_and_changes_nothing() {
    let mut attachments = Attachments::new();
    let version = attachments.version_for(Audience::Seat(seat(1)));
    let long = too_long();
    let refusals = [
        attachments.push_public(StoryLine::new(text(&long))),
        attachments.push_public(public_line("ok").with_narration(Narration::new(&long))),
        attachments.push_private(seat(1), StoryLine::new(text(&long))),
        attachments.set_place(crate::table_state_tests::scene(1), text(&long)),
        attachments.set_shared_readout(text(&long)),
        attachments.set_seat_readout(seat(1), text(&long)),
        attachments.set_private_fact(seat(1), fact_id(1), Secret::new(long.clone())),
        attachments.set_notice(seat(1), declined(&long), at()),
        attachments.set_state_label(crate::table_state_tests::machine("explore"), text(&long)),
        attachments.set_activity(ProjectionStatus::waiting_for(&long, "to act").unwrap()),
    ];
    for refusal in refusals {
        assert_eq!(refusal, Err(AttachmentError::TextTooLong));
    }
    assert_eq!(attachments.version_for(Audience::Seat(seat(1))), version);
    assert_eq!(attachments, Attachments::new());
}

#[test]
fn a_text_exactly_at_the_limit_is_accepted() {
    let mut attachments = Attachments::new();
    let long = longest();
    assert_eq!(attachments.push_public(StoryLine::new(text(&long))), Ok(()));
    assert_eq!(
        attachments.set_private_fact(seat(1), fact_id(1), Secret::new(long.clone())),
        Ok(())
    );
    assert_eq!(attachments.set_shared_readout(text(&long)), Ok(()));
}

#[test]
fn offers_must_fit_their_budget_together_and_each_alternative_of_a_decline_must_fit_too() {
    let mut attachments = Attachments::new();
    let one = |number: usize, bytes: usize| {
        OfferEntry::new(
            OfferId::new(&format!("o{number}")).unwrap(),
            text(&"y".repeat(bytes)),
        )
    };
    let fits = OFFERS_BUDGET_BYTES / 4;
    assert_eq!(
        attachments.set_seat_offers(seat(1), (0..3).map(|n| one(n, fits)).collect()),
        Ok(())
    );
    assert_eq!(
        attachments.set_seat_offers(seat(1), (0..6).map(|n| one(n, fits)).collect()),
        Err(AttachmentError::TextTooLong)
    );
    let mut decline = declined("No.");
    for _ in 0..6 {
        decline = decline.with_alternative(&"z".repeat(fits)).unwrap();
    }
    assert_eq!(
        attachments.set_notice(seat(1), decline, at()),
        Err(AttachmentError::TextTooLong)
    );
}

#[test]
fn the_note_shows_the_newest_facts_within_its_budget_and_counts_the_older_ones() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let big = NOTE_BUDGET_BYTES / 2 - 1;
    for number in 1..=5 {
        let body = format!("{number}{}", "f".repeat(big));
        attachments
            .set_private_fact(seat(1), fact_id(number), Secret::new(body))
            .unwrap();
    }
    let projection = project_seat(&table.snapshot(), &attachments, seat(1)).unwrap();
    let note = json_of(&projection)["view"]["note"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let shown: Vec<&str> = note.lines().collect();
    assert_eq!(shown.len(), 2, "two of the five fit");
    assert!(
        shown[0].starts_with('4') && shown[1].starts_with('5'),
        "oldest first: {note:.5}"
    );
    assert_eq!(projection.truncation().older_facts, 3);
    assert_eq!(projection.record().facts_shown, 2);
    assert!(
        projection.record().work.facts_visited <= 3,
        "{} facts examined",
        projection.record().work.facts_visited
    );
}

#[test]
fn a_fact_set_again_becomes_the_newest_and_the_note_always_shows_the_newest_fact() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in 1..=3 {
        attachments
            .set_private_fact(
                seat(1),
                fact_id(number),
                Secret::new(format!("fact {number}")),
            )
            .unwrap();
    }
    attachments
        .set_private_fact(seat(1), fact_id(1), Secret::new("fact 1 again".to_owned()))
        .unwrap();
    let note = &seat_json(&table, &attachments, 1)["view"]["note"]["text"];
    assert_eq!(note, "fact 2\nfact 3\nfact 1 again");
    let mut huge = Attachments::new();
    huge.set_private_fact(seat(1), fact_id(1), Secret::new(longest()))
        .unwrap();
    let alone = project_seat(&table.snapshot(), &huge, seat(1)).unwrap();
    assert_eq!(
        alone.record().facts_shown,
        1,
        "the newest fact shows even beyond the budget"
    );
}

fn two_line_entry(number: usize, bytes: usize) -> StoryLine {
    public_line(&format!("r{number}"))
        .with_narration(Narration::new(format!("n{number}{}", "w".repeat(bytes))))
}

#[test]
fn the_story_window_never_cuts_an_entry_in_half_so_a_readout_always_comes_before_its_words() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in 0..30 {
        attachments.push_public(two_line_entry(number, 3)).unwrap();
    }
    let view = seat_json(&table, &attachments, 1);
    let shown = lines(&view);
    assert_eq!(
        shown.len(),
        PHONE_NARRATION_LINES,
        "ten whole entries fill twenty lines"
    );
    assert!(
        shown[0].starts_with('r'),
        "the oldest shown line is a readout: {}",
        shown[0]
    );
    for pair in shown.chunks(2) {
        assert!(
            pair[0].starts_with('r') && pair[1].starts_with('n'),
            "{pair:?}"
        );
    }
    assert!(shown[shown.len() - 1].starts_with("n29"));
}

#[test]
fn a_decline_counts_as_one_of_the_twenty_lines_and_no_phone_shows_twenty_one() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in 0..30 {
        attachments
            .push_public(public_line(&format!("line {number}")))
            .unwrap();
    }
    attachments
        .set_notice(seat(1), declined("Not now."), at())
        .unwrap();
    let shown_with_notice = seat_json(&table, &attachments, 1);
    let with_notice = lines(&shown_with_notice);
    assert_eq!(with_notice.len(), PHONE_NARRATION_LINES);
    assert_eq!(with_notice.last(), Some(&"Not now."));
    assert_eq!(with_notice.first(), Some(&"line 11"));
    let without = seat_json(&table, &attachments, 2);
    assert_eq!(lines(&without).len(), PHONE_NARRATION_LINES);
    assert_eq!(lines(&without).first(), Some(&"line 10"));
}

#[test]
fn the_story_window_stops_at_its_byte_budget_keeps_the_newest_entry_and_counts_the_rest() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    let body = NARRATION_BUDGET_BYTES / 3;
    for number in 0..6 {
        attachments
            .push_public(two_line_entry(number, body))
            .unwrap();
    }
    let projection = project_seat(&table.snapshot(), &attachments, seat(1)).unwrap();
    let json = json_of(&projection);
    let shown = lines(&json);
    assert_eq!(shown.len(), 4, "two entries of two lines fit the budget");
    assert!(shown.last().unwrap().starts_with("n5"));
    assert_eq!(projection.truncation().older_story_entries, 4);
    let mut huge = Attachments::new();
    huge.push_public(two_line_entry(0, MAXIMUM_TEXT_BYTES - 3))
        .unwrap();
    let lone = project_seat(&table.snapshot(), &huge, seat(1)).unwrap();
    assert_eq!(
        lone.view().narration.len(),
        2,
        "the newest entry shows however long it is"
    );
}

#[test]
fn the_work_of_a_phone_view_is_bounded_by_what_it_shows_whatever_the_story_and_the_facts() {
    let table = table_with_two_heroes();
    let mut attachments = Attachments::new();
    for number in 0..500 {
        attachments
            .push_public(public_line(&format!("line {number}")))
            .unwrap();
        attachments
            .set_private_fact(
                seat(1),
                fact_id(u8::try_from(number % 200).unwrap() + 1),
                Secret::new(format!("fact {number}")),
            )
            .unwrap();
    }
    let work = project_seat(&table.snapshot(), &attachments, seat(1))
        .unwrap()
        .record()
        .work;
    assert!(
        work.story_entries_visited <= PHONE_NARRATION_LINES + 1,
        "{work:?}"
    );
    assert_eq!(
        work.cast_visited, 0,
        "the phone finds its hero without scanning the cast"
    );
}
