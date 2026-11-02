//! The window, the pending book and the ruling book on their own: bounds, order, copies, nudges
//! and the saved form.

use crate::envelope::PendingId;
use crate::event::TableEvent;
use crate::identity::{OperationId, SceneId, TableId, TurnSeq};
use crate::limits::MAXIMUM_OPEN_PENDINGS;
use crate::outcome::PendingKind;
use crate::pending::window::{AnswerRefusal, Completion, ResolutionWindow, WindowError};
use crate::pending::{
    Continuation, MalformedPending, PendingBook, PendingError, PendingLoadError, PendingMark,
    PendingOpening, PendingRecord, PendingResolution, RulingBook, RulingSource, ScopedRuling,
    StoredDraw,
};
use crate::pipeline_fixtures_tests::{action, seat};
use crate::ports::{
    ConfirmationReason, Draw, Offer, RulingAnswer, RulingKey, RulingScope, RulingTier,
};
use std::num::NonZeroU16;

fn offer(seat_number: u64, choices: &[&str]) -> Offer {
    Offer {
        seat: seat(seat_number),
        choices: choices.iter().map(|name| action(name)).collect(),
    }
}

fn two_seat_window(completion: Completion) -> ResolutionWindow {
    ResolutionWindow::new(
        vec![offer(2, &["shield", "pass"]), offer(3, &["pass"])],
        completion,
    )
    .expect("a window of two offers")
}

fn operation(number: u128) -> OperationId {
    OperationId::from_nonzero_u128(std::num::NonZeroU128::new(number).expect("nonzero"))
}

fn opening(book: &PendingBook, number: u128, kind: PendingKind) -> PendingOpening {
    PendingOpening {
        id: book.next_id(),
        kind,
        opened_by: seat(1),
        operation: operation(number),
        turn: TurnSeq::new(number as u64),
        now_ms: 1_000,
        scene: Some(SceneId::new(1)),
        continuation: Continuation::Confirm {
            seat: seat(1),
            action: action("stab"),
            reason: ConfirmationReason::HarmsANeutral,
        },
        draws: Vec::new(),
        window: two_seat_window(Completion::EveryOffer),
    }
}

fn open_one(book: &mut PendingBook, number: u128, kind: PendingKind) -> PendingId {
    let pending = PendingResolution::open(opening(book, number, kind));
    let id = pending.id();
    book.insert(pending).expect("the pending is inserted");
    id
}

#[test]
fn a_window_needs_offers_each_with_a_seat_once_and_something_to_choose() {
    assert_eq!(
        ResolutionWindow::new(Vec::new(), Completion::EveryOffer),
        Err(WindowError::NoOffers)
    );
    assert_eq!(
        ResolutionWindow::new(
            vec![offer(2, &["pass"]), offer(2, &["pass"])],
            Completion::EveryOffer
        ),
        Err(WindowError::SeatOfferedTwice(seat(2)))
    );
    assert_eq!(
        ResolutionWindow::new(vec![offer(2, &[])], Completion::EveryOffer),
        Err(WindowError::NothingToChoose(seat(2)))
    );
}

#[test]
fn a_window_that_needs_every_offer_completes_only_when_each_seat_has_answered() {
    let window = two_seat_window(Completion::EveryOffer);

    let one = window
        .answered(seat(2), &action("shield"))
        .expect("the first answer");
    let both = one
        .answered(seat(3), &action("pass"))
        .expect("the second answer");

    assert!(!window.is_complete() && !one.is_complete() && both.is_complete());
    assert_eq!(one.awaiting().collect::<Vec<_>>(), [seat(3)]);
}

#[test]
fn a_window_that_needs_any_offer_completes_at_the_first_answer() {
    let window = two_seat_window(Completion::AnyOffer);

    let answered = window
        .answered(seat(3), &action("pass"))
        .expect("an answer");

    assert!(answered.is_complete());
}

#[test]
fn answering_returns_a_copy_and_leaves_the_window_as_it_was() {
    let window = two_seat_window(Completion::EveryOffer);

    let answered = window
        .answered(seat(2), &action("pass"))
        .expect("an answer");

    assert_ne!(window, answered);
    assert!(
        window
            .offer_for(seat(2))
            .expect("an offer")
            .answer()
            .is_none()
    );
}

#[test]
fn an_answer_from_a_seat_without_an_offer_or_off_the_offer_or_twice_is_refused() {
    let window = two_seat_window(Completion::EveryOffer);
    let answered = window
        .answered(seat(2), &action("pass"))
        .expect("an answer");

    assert_eq!(
        window.answered(seat(9), &action("pass")),
        Err(AnswerRefusal::NotEligible)
    );
    assert_eq!(
        window.answered(seat(3), &action("shield")),
        Err(AnswerRefusal::NotOffered)
    );
    assert_eq!(
        answered.answered(seat(2), &action("shield")),
        Err(AnswerRefusal::AlreadyAnswered)
    );
}

#[test]
fn the_answers_come_in_the_order_of_the_offers_whatever_order_they_arrived() {
    let window = two_seat_window(Completion::EveryOffer);

    let late_first = window
        .answered(seat(3), &action("pass"))
        .and_then(|next| next.answered(seat(2), &action("shield")))
        .expect("both answer");

    let seats: Vec<_> = late_first.answers().map(|(seat, _)| seat).collect();
    assert_eq!(seats, [seat(2), seat(3)]);
}

#[test]
fn a_seat_that_is_asked_again_can_answer_again() {
    let window = two_seat_window(Completion::EveryOffer);
    let answered = window
        .answered(seat(2), &action("shield"))
        .expect("an answer");

    let asked_again = answered.reasked(seat(2));

    assert!(asked_again.answered(seat(2), &action("pass")).is_ok());
}

#[test]
fn the_book_hands_out_ids_in_order_and_refuses_one_that_is_not_next() {
    let mut book = PendingBook::default();
    let first = open_one(&mut book, 1, PendingKind::Reaction);
    let second = open_one(&mut book, 2, PendingKind::Choice);
    let mut wrong = opening(&book, 3, PendingKind::Choice);
    wrong.id = PendingId::new(99);

    assert_eq!((first.get(), second.get()), (1, 2));
    assert_eq!(
        book.insert(PendingResolution::open(wrong)),
        Err(PendingError::IdNotNext)
    );
}

#[test]
fn a_book_that_is_full_refuses_a_new_pending_and_a_closed_one_makes_room() {
    let mut book = PendingBook::holding_at_most(1);
    let first = open_one(&mut book, 1, PendingKind::Reaction);
    let second = PendingResolution::open(opening(&book, 2, PendingKind::Reaction));

    assert_eq!(book.insert(second.clone()), Err(PendingError::TooManyOpen));
    book.remove(first);
    assert_eq!(book.insert(second), Ok(()));
}

#[test]
fn an_operation_opens_one_pending_and_the_book_finds_it_by_the_operation() {
    let mut book = PendingBook::default();
    let id = open_one(&mut book, 5, PendingKind::Reaction);
    let again = PendingResolution::open(opening(&book, 5, PendingKind::Reaction));

    assert_eq!(
        book.opened_by(operation(5)).map(|pending| pending.id()),
        Some(id)
    );
    assert_eq!(
        book.insert(again),
        Err(PendingError::OperationAlreadyOpened)
    );
    book.remove(id);
    assert!(book.opened_by(operation(5)).is_none());
}

#[test]
fn a_pending_of_another_scene_does_not_count_for_this_one() {
    let mut book = PendingBook::default();
    open_one(&mut book, 1, PendingKind::Reaction);

    assert_eq!(book.open_in_scene(Some(SceneId::new(1))), 1);
    assert_eq!(book.open_in_scene(Some(SceneId::new(2))), 0);
}

#[test]
fn holding_a_seat_marks_the_pendings_it_waits_on_once_and_releasing_gives_them_back() {
    let mut book = PendingBook::default();
    let id = open_one(&mut book, 1, PendingKind::Reaction);

    let held = book.would_hold(seat(2));
    book.mark(PendingMark::Hold(seat(2)));
    let held_again = book.would_hold(seat(2));
    let unrelated = book.would_hold(seat(9));
    let is_held = book.get(id).map(PendingResolution::is_held);
    let released = book.would_release(seat(2));
    book.mark(PendingMark::Release(seat(2)));

    assert_eq!(
        (held, held_again, unrelated),
        (vec![id], Vec::new(), Vec::new())
    );
    assert_eq!(is_held, Some(true));
    assert_eq!(released, [id]);
    assert_eq!(book.get(id).map(PendingResolution::is_held), Some(false));
}

#[test]
fn a_reconnecting_seat_finds_its_offers_by_pending_id() {
    let mut book = PendingBook::default();
    let id = open_one(&mut book, 1, PendingKind::Reaction);

    let offers = book.offers_awaiting(seat(2));

    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].0, id);
    assert!(book.offers_awaiting(seat(9)).is_empty());
}

#[test]
fn the_nudge_sweep_marks_a_pending_once_after_its_own_nudge_time() {
    let mut book = PendingBook::default();
    let window = open_one(&mut book, 1, PendingKind::Reaction);
    let ruling = open_one(&mut book, 2, PendingKind::Ruling);

    let early = book.would_nudge(1_000 + 19_999, 20_000, 30_000);
    let ruling_due = book.would_nudge(1_000 + 20_000, 20_000, 30_000);
    for nudge in &ruling_due {
        book.mark(PendingMark::Nudged(nudge.pending));
    }
    let window_due = book.would_nudge(1_000 + 30_000, 20_000, 30_000);
    for nudge in &window_due {
        book.mark(PendingMark::Nudged(nudge.pending));
    }
    let again = book.would_nudge(1_000 + 90_000, 20_000, 30_000);

    assert!(early.is_empty() && again.is_empty());
    assert_eq!(
        ruling_due
            .iter()
            .map(|nudge| nudge.pending)
            .collect::<Vec<_>>(),
        [ruling]
    );
    assert_eq!(
        window_due
            .iter()
            .map(|nudge| nudge.pending)
            .collect::<Vec<_>>(),
        [window]
    );
    assert_eq!(book.len(), 2, "a nudge never closes a pending");
}

#[test]
fn a_ruling_pending_waits_for_an_adjudicator_from_the_moment_it_opens() {
    let mut book = PendingBook::default();
    let id = open_one(&mut book, 1, PendingKind::Ruling);

    let status = book.get(id).map(|pending| pending.status().key());

    assert_eq!(status, Some("awaiting_adjudicator"));
}

#[test]
fn the_saved_book_round_trips_through_json_with_its_dice_and_ids() {
    let mut book = PendingBook::default();
    let mut opened = opening(&book, 1, PendingKind::Reaction);
    let die = Draw::new(NonZeroU16::new(20).expect("20"), 14).expect("14 is on a d20");
    opened.draws = vec![StoredDraw::of(die)];
    book.insert(PendingResolution::open(opened))
        .expect("inserted");
    open_one(&mut book, 2, PendingKind::Choice);
    book.remove(PendingId::new(2));

    let text = serde_json::to_string(&book.to_record(1_000)).expect("the record writes");
    let record: PendingRecord = serde_json::from_str(&text).expect("the record reads");
    let restored =
        PendingBook::from_record(record, 1_000, MAXIMUM_OPEN_PENDINGS).expect("the record loads");

    assert_eq!(restored, book);
    assert_eq!(restored.digest(), book.digest());
    assert_eq!(
        restored.next_id().get(),
        3,
        "an id that was used is never handed out again"
    );
    assert_eq!(
        restored
            .get(PendingId::new(1))
            .expect("kept")
            .restored_draws()
            .expect("valid"),
        [die]
    );
}

fn record_with(edit: impl FnOnce(&mut serde_json::Value)) -> Result<PendingBook, PendingLoadError> {
    let mut book = PendingBook::default();
    open_one(&mut book, 1, PendingKind::Reaction);
    let mut value = serde_json::to_value(book.to_record(1_000)).expect("the record writes");
    edit(&mut value);
    let record: PendingRecord =
        serde_json::from_value(value).expect("the edited record still parses");
    PendingBook::from_record(record, 1_000, MAXIMUM_OPEN_PENDINGS)
}

#[test]
fn a_saved_book_from_a_newer_engine_or_with_an_impossible_shape_is_refused_with_a_typed_cause() {
    let newer = record_with(|value| value["schema_version"] = serde_json::json!(2));
    let next_too_low = record_with(|value| value["next_id"] = serde_json::json!(1));
    let impossible_die = record_with(|value| {
        value["pendings"][0]["draws"] = serde_json::json!([{"sides": 6, "value": 9}]);
    });
    let doubled = record_with(|value| {
        let again = value["pendings"][0].clone();
        value["pendings"]
            .as_array_mut()
            .expect("an array")
            .push(again);
    });

    assert_eq!(
        newer.err(),
        Some(PendingLoadError::UnsupportedVersion {
            found: 2,
            expected: 1
        })
    );
    assert_eq!(
        next_too_low.err(),
        Some(PendingLoadError::Malformed(
            MalformedPending::IdNotBelowNext
        ))
    );
    assert_eq!(
        impossible_die.err(),
        Some(PendingLoadError::Malformed(MalformedPending::ImpossibleDie))
    );
    assert_eq!(
        doubled.err(),
        Some(PendingLoadError::Malformed(
            MalformedPending::DuplicateOperation
        ))
    );
}

#[test]
fn a_refused_pending_record_is_recorded_as_a_load_refusal_naming_its_kind() {
    let error = PendingLoadError::UnsupportedVersion {
        found: 2,
        expected: 1,
    };

    let line = error.event(TableId::new(1)).render_audit_line();

    assert_eq!(
        line,
        "LOAD refused table=1 record=pending_book version=2 expected=1 cause=unsupported_version"
    );
}

fn ruling(
    key: &str,
    scope: RulingScope,
    scene: u64,
    answer: &str,
    tier: RulingTier,
) -> ScopedRuling {
    ScopedRuling::new(
        RulingKey::new(key).expect("valid"),
        scope,
        Some(SceneId::new(scene)),
        RulingAnswer::new(answer).expect("valid"),
        tier,
        RulingSource::Host,
        action("swing"),
    )
}

fn key(text: &str) -> RulingKey {
    RulingKey::new(text).expect("valid")
}

#[test]
fn a_ruling_is_found_by_the_equal_key_and_scope_and_by_nothing_else() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "chandelier_swing",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));

    assert!(
        book.find(
            &key("chandelier_swing"),
            RulingScope::Table,
            Some(SceneId::new(7))
        )
        .is_some()
    );
    assert!(
        book.find(&key("chandelier_swing_2"), RulingScope::Table, None)
            .is_none()
    );
    assert!(
        book.find(
            &key("chandelier_swing"),
            RulingScope::Scene,
            Some(SceneId::new(1))
        )
        .is_none()
    );
}

#[test]
fn a_scene_ruling_binds_only_its_own_scene() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "rope_pull",
        RulingScope::Scene,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));

    assert!(
        book.find(&key("rope_pull"), RulingScope::Scene, Some(SceneId::new(1)))
            .is_some()
    );
    assert!(
        book.find(&key("rope_pull"), RulingScope::Scene, Some(SceneId::new(2)))
            .is_none()
    );
}

#[test]
fn a_newer_ruling_for_the_same_question_replaces_the_older() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "rope_pull",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));
    book.store(ruling(
        "rope_pull",
        RulingScope::Table,
        1,
        "advantage",
        RulingTier::Advantage,
    ));

    assert_eq!(book.len(), 1);
    let found = book
        .find(&key("rope_pull"), RulingScope::Table, None)
        .expect("stored");
    assert_eq!(found.answer().as_str(), "advantage");
}

#[test]
fn the_book_keeps_the_newest_rulings_and_forgets_the_oldest_first() {
    let mut book = RulingBook::remembering(2);
    book.store(ruling(
        "one",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));
    book.store(ruling(
        "two",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));
    book.store(ruling(
        "three",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));

    assert!(book.find(&key("one"), RulingScope::Table, None).is_none());
    assert!(book.find(&key("two"), RulingScope::Table, None).is_some());
    assert!(book.find(&key("three"), RulingScope::Table, None).is_some());
}

#[test]
fn a_reuse_that_grants_advantage_is_confirmed_once_per_scene() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "rope_pull",
        RulingScope::Table,
        1,
        "advantage",
        RulingTier::Advantage,
    ));
    let in_same_scene = book
        .find(&key("rope_pull"), RulingScope::Table, None)
        .expect("stored");

    assert!(!in_same_scene.needs_reconfirmation(Some(SceneId::new(1))));
    assert!(in_same_scene.needs_reconfirmation(Some(SceneId::new(2))));
    book.reconfirm(&key("rope_pull"), RulingScope::Table, Some(SceneId::new(2)));
    let after = book
        .find(&key("rope_pull"), RulingScope::Table, None)
        .expect("stored");
    assert!(!after.needs_reconfirmation(Some(SceneId::new(2))));
    assert!(after.needs_reconfirmation(Some(SceneId::new(3))));
}

#[test]
fn an_ordinary_ruling_is_never_asked_again() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "rope_pull",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));
    let found = book
        .find(&key("rope_pull"), RulingScope::Table, None)
        .expect("stored");

    assert!(!found.needs_reconfirmation(Some(SceneId::new(9))));
}

#[test]
fn a_revoked_ruling_is_gone_and_revoking_it_again_says_so() {
    let mut book = RulingBook::default();
    book.store(ruling(
        "rope_pull",
        RulingScope::Table,
        1,
        "ordinary",
        RulingTier::Ordinary,
    ));

    assert!(book.revoke(&key("rope_pull"), RulingScope::Table, None));
    assert!(!book.revoke(&key("rope_pull"), RulingScope::Table, None));
    assert!(book.is_empty());
}

#[test]
fn the_pending_event_types_render_the_pinned_context_then_their_own_keys() {
    let event = TableEvent::Pipeline(crate::pipeline::events::PipelineEvent::PendingResolved {
        scope: crate::identity::TableScope::of_table(TableId::new(1)),
        correlation: crate::testkit::correlation(3),
        pending: PendingId::new(4),
        kind: PendingKind::Reaction,
        ms: 250,
    });

    let line = event.render_audit_line();

    assert!(line.starts_with("PENDING resolved table=1 operation_id="));
    assert!(line.ends_with(" pending=4 kind=reaction ms=250"));
    let parsed: TableEvent =
        serde_json::from_str(&serde_json::to_string(&event).expect("writes")).expect("reads back");
    assert_eq!(parsed, event);
}
