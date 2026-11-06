//! The first playable path (O.16 H3), end to end through the real owner: a lobby where seats join
//! through the queue, the story scene, one conversation turn with its dice and rules, the group
//! commit, and the real projection served as the wire's `MachineView` and answered as a
//! `MachineReceipt`. No browser: the TV and the phone are read as the messages they are served.

use df_api::machine::MachineIntentError;
use df_protocol::common as wire;
use df_protocol::common::machine_view::Audience;
use df_table::outcome::{DeclineCode, OutcomeKind};
use df_table::owner_queue::NotDeclinedCause;
use prost::Message;

use super::commit::{CommitError, ScriptedCommit, ScriptedCommitStore};
use super::fixtures::{
    Rig, RigOptions, count_lines, story_rig, story_rig_df_rules, story_rig_on, story_rig_options,
    tap, text, with_operation,
};
use super::fresh_id::random_u128;
use super::machine_io::{
    CallStatus, WireIntentBody, WireIntentError, WireIntentKind, forgotten, refused,
    result_of_turn, view_report,
};
use super::report::{CommitDisposition, EngineError, RefusalCause, RefusalReport};
use super::request::{AudienceSpec, QueueControl, TraceOperation};

const GUARD_MOVED: &str = "Fine. Keep your voices down";
const GUARD_UNMOVED: &str = "No. Come back in the morning";

async fn seat_offers(rig: &Rig, seat: u64) -> Vec<String> {
    let view = rig
        .handle
        .machine_view(AudienceSpec::Seat { seat })
        .await
        .expect("a view");
    let Some(Audience::Seat(seat)) = view.audience else {
        panic!("a seat view");
    };
    seat.offers
        .into_iter()
        .map(|offer| offer.offer_id)
        .collect()
}

async fn tv_readout(rig: &Rig) -> String {
    rig.handle
        .machine_view(AudienceSpec::Tv)
        .await
        .expect("a view")
        .readout
}

/// Two seats join, ready and start; the table is then in the story scene.
async fn start_story(rig: &Rig) {
    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "join")).await.expect("joined");
    }
    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "ready")).await.expect("ready");
    }
    rig.handle.submit(tap(1, "start")).await.expect("started");
}

/// Through the story scene to the conversation.
async fn reach_the_conversation(rig: &Rig) {
    start_story(rig).await;
    rig.handle
        .submit(tap(1, "pick_lock"))
        .await
        .expect("the check ran");
}

#[tokio::test]
async fn seats_join_through_the_queue_with_typed_outcomes_and_a_guest_phone_can_press_join() {
    let rig = story_rig(&[]);

    let guest = rig
        .handle
        .machine_view(AudienceSpec::Seat { seat: 3 })
        .await
        .expect("a seat in the queue that has not sat down still has a view");
    let Some(Audience::Seat(phone)) = &guest.audience else {
        panic!("a seat view");
    };
    assert_eq!(phone.hero, "Seat 3");
    assert_eq!(phone.offers.len(), 1);
    assert_eq!(phone.offers[0].offer_id, "join");
    assert_eq!(guest.machine_id, "lobby");
    assert!(
        guest.readout.contains("Join the table"),
        "{}",
        guest.readout
    );

    let joined = rig.handle.submit(tap(3, "join")).await.expect("answered");
    assert_eq!(joined.turns[0].outcome, OutcomeKind::Applied);
    assert!(matches!(
        joined.turns[0].commit,
        CommitDisposition::Committed { .. }
    ));
    let again = rig.handle.submit(tap(3, "join")).await.expect("answered");
    assert_eq!(again.turns[0].outcome, OutcomeKind::Declined);
    assert_eq!(again.turns[0].decline, Some(DeclineCode::NotViable));
    assert_eq!(again.turns[0].readout, "You already have a seat.");
    let stranger = rig.handle.submit(tap(9, "join")).await.expect("answered");
    assert!(
        stranger.submission.refusal.is_some(),
        "a seat the queue does not know is refused at the queue, typed"
    );
    assert_eq!(seat_offers(&rig, 3).await, ["ready", "start", "leave"]);
}

#[tokio::test]
async fn the_lobby_declines_a_start_until_everyone_is_ready_then_hands_the_table_to_the_scene() {
    let rig = story_rig(&[]);
    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "join")).await.expect("joined");
    }
    rig.handle.submit(tap(1, "ready")).await.expect("ready");
    assert_eq!(seat_offers(&rig, 1).await, ["unready", "start", "leave"]);

    let early = rig.handle.submit(tap(1, "start")).await.expect("answered");
    assert_eq!(early.turns[0].outcome, OutcomeKind::Declined);
    assert!(early.turns[0].readout.contains("Everyone at the table"));

    rig.handle.submit(tap(2, "ready")).await.expect("ready");
    let started = rig.handle.submit(tap(1, "start")).await.expect("answered");
    let [start, hand_off] = started.turns.as_slice() else {
        panic!("start and its hand-off: {:?}", started.turns);
    };
    assert_eq!(start.commit, hand_off.commit, "one commit covers both");
    assert_eq!(hand_off.machine_after, "explore");
    assert_eq!(seat_offers(&rig, 2).await, ["pick_lock"]);
    assert!(tv_readout(&rig).await.contains("locked_gate"));
}

#[tokio::test]
async fn the_story_scene_hands_the_table_to_the_conversation_and_the_phone_offers_the_urging() {
    let rig = story_rig(&[]);
    start_story(&rig).await;

    let checked = rig
        .handle
        .submit(tap(1, "pick_lock"))
        .await
        .expect("answered");

    let [check, hand_off] = checked.turns.as_slice() else {
        panic!("the check and its hand-off: {:?}", checked.turns);
    };
    assert_eq!(check.outcome, OutcomeKind::Applied);
    assert_eq!(check.commit, hand_off.commit);
    assert_eq!(
        (
            hand_off.machine_after.as_str(),
            hand_off.state_after.as_str()
        ),
        ("dialogue", "listening")
    );
    assert_eq!(seat_offers(&rig, 1).await, ["persuade_guard"]);
    assert_eq!(seat_offers(&rig, 2).await, ["persuade_guard"]);
    let tv = tv_readout(&rig).await;
    assert!(tv.contains("The gate guard") && tv.contains("Halt"), "{tv}");
}

#[tokio::test]
async fn a_conversation_turn_draws_through_the_dice_adapter_and_the_rules_owner_settles_it() {
    for (face, line) in [(20, GUARD_MOVED), (1, GUARD_UNMOVED)] {
        // The scene's own check draws the first die; the urging draws the second.
        let rig = story_rig(&[10, face]);
        reach_the_conversation(&rig).await;
        let drawn_before = count_lines(&rig.log, "DICE");
        let reply = rig
            .handle
            .submit(tap(1, "persuade_guard"))
            .await
            .expect("answered");
        let turn = &reply.turns[0];
        assert_eq!(turn.outcome, OutcomeKind::Applied, "{turn:?}");
        assert!(
            turn.readout
                .contains(&format!("succeeded {}", u8::from(face == 20))),
            "{}",
            turn.readout
        );
        let seat_view = turn.view.as_ref().expect("the sender's view is released");
        assert_eq!(seat_view["viewer"]["seat"], 1);
        assert!(tv_readout(&rig).await.contains(line));
        assert!(
            count_lines(&rig.log, "DICE") > drawn_before,
            "the draw is in the log"
        );
        assert!(
            seat_offers(&rig, 1).await.is_empty(),
            "nothing is left to take"
        );
    }
}

#[tokio::test]
async fn a_typed_line_is_read_by_the_interpreter_and_a_line_it_cannot_read_gives_buttons() {
    let rig = story_rig(&[10, 20]);
    reach_the_conversation(&rig).await;

    let unclear = rig
        .handle
        .submit(text(1, "Please, friend, we only need a minute"))
        .await
        .expect("answered");
    assert_eq!(unclear.turns[0].outcome, OutcomeKind::NeedsClarification);
    assert!(
        seat_offers(&rig, 1)
            .await
            .contains(&"persuade_guard".to_owned())
    );

    let clear = rig
        .handle
        .submit(text(1, "persuade_guard"))
        .await
        .expect("answered");
    assert_eq!(clear.turns[0].outcome, OutcomeKind::Applied);
    assert_eq!(clear.turns[0].action.as_deref(), Some("persuade_guard"));
    assert!(tv_readout(&rig).await.contains(GUARD_MOVED));
}

#[tokio::test]
async fn the_production_rules_adapter_runs_the_influence_flow_with_two_dice_for_a_friendly_guard() {
    let rig = story_rig_df_rules(11);
    reach_the_conversation(&rig).await;
    let reply = rig
        .handle
        .submit(tap(1, "persuade_guard"))
        .await
        .expect("answered");
    let turn = &reply.turns[0];
    assert_eq!(turn.outcome, OutcomeKind::Applied, "{turn:?}");
    assert!(turn.readout.contains("dice rolled 2"), "{}", turn.readout);
    assert!(
        turn.readout.contains("difficulty class 15"),
        "{}",
        turn.readout
    );
    let tv = tv_readout(&rig).await;
    assert!(
        tv.contains(GUARD_MOVED) || tv.contains(GUARD_UNMOVED),
        "{tv}"
    );
}

#[tokio::test]
async fn the_wire_view_is_the_real_projection_and_its_bytes_decode_to_the_same_message() {
    let rig = story_rig(&[]);
    start_story(&rig).await;

    let tv = rig
        .handle
        .machine_view(AudienceSpec::Tv)
        .await
        .expect("view");
    let Some(Audience::SharedScreen(screen)) = &tv.audience else {
        panic!("the shared screen");
    };
    assert!(!screen.place.is_empty());
    assert_eq!(tv.machine_id, "explore");
    let decoded = wire::MachineView::decode(tv.encode_to_vec().as_slice()).expect("decodes");
    assert_eq!(decoded, tv);
    df_api::machine::check_machine_view(&tv).expect("the guard passes it");
    assert_eq!(view_report(&tv).protobuf_bytes, tv.encoded_len());

    let phone = rig
        .handle
        .machine_view(AudienceSpec::Seat { seat: 2 })
        .await
        .expect("view");
    let Some(Audience::Seat(seat)) = &phone.audience else {
        panic!("a seat view");
    };
    assert_eq!(seat.offers.len(), 1);
    assert_eq!(seat.offers[0].label, "Pick lock");
    assert_eq!(phone.revision, tv.revision);
}

fn intent_for(
    machine: &str,
    seat: u64,
    what: WireIntentKind,
    operation: u128,
    observed: u64,
) -> wire::SubmitMachineIntentRequest {
    let body = WireIntentBody {
        seat,
        intent: what,
        operation: None,
        machine: None,
        observed: None,
    };
    body.request(machine, observed, operation)
}

fn intent(
    seat: u64,
    what: WireIntentKind,
    operation: u64,
    observed: u64,
) -> wire::SubmitMachineIntentRequest {
    intent_for("lobby", seat, what, u128::from(operation), observed)
}

fn tap_kind(offer: &str) -> WireIntentKind {
    WireIntentKind::Tap {
        offer: offer.to_owned(),
    }
}

fn receipt(response: &wire::SubmitMachineIntentResponse) -> &wire::MachineReceipt {
    match &response.outcome {
        Some(wire::submit_machine_intent_response::Outcome::Receipt(receipt)) => receipt,
        other => panic!("a receipt: {other:?}"),
    }
}

async fn revision_of(rig: &Rig, seat: u64) -> u64 {
    rig.handle
        .machine_view(AudienceSpec::Seat { seat })
        .await
        .expect("view")
        .revision
        .and_then(|revision| revision.sequence)
        .unwrap_or_default()
}

#[tokio::test]
async fn a_wire_intent_runs_through_the_queue_once_per_operation_id_and_answers_one_receipt() {
    let rig = story_rig(&[]);
    let observed = revision_of(&rig, 1).await;
    let request = intent(
        1,
        WireIntentKind::Tap {
            offer: "join".to_owned(),
        },
        77,
        observed,
    );

    let first = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect("a receipt");
    assert!(matches!(
        receipt(&first).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert!(!receipt(&first).replayed);

    let again = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect("a receipt");
    assert!(
        receipt(&again).replayed,
        "the same operation id is a retry, not a second join"
    );
    let dump = rig.handle.dump().await.expect("dump");
    assert_eq!(dump.table.seats, [1]);

    let observed = revision_of(&rig, 1).await;
    let nothing = intent(
        1,
        WireIntentKind::Tap {
            offer: "nothing".to_owned(),
        },
        78,
        observed,
    );
    let response = rig
        .handle
        .machine_intent(1, &nothing)
        .await
        .expect("a receipt");
    let Some(wire::machine_receipt::Result::Declined(decline)) = &receipt(&response).result else {
        panic!("a decline: {response:?}");
    };
    assert!(!decline.reason.is_empty());
}

#[tokio::test]
async fn wire_intents_of_every_kind_are_read_at_the_edge_and_answered_by_kind() {
    let rig = story_rig(&[]);
    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "join")).await.expect("joined");
    }
    let observed = revision_of(&rig, 1).await;
    let line = intent(
        1,
        WireIntentKind::Text {
            text: "hello there".to_owned(),
        },
        90,
        observed,
    );
    let response = rig
        .handle
        .machine_intent(1, &line)
        .await
        .expect("a receipt");
    assert_eq!(
        receipt(&response).utterance_id,
        Some(1),
        "a line is named by its utterance id"
    );
    assert!(matches!(
        receipt(&response).result,
        Some(wire::machine_receipt::Result::NeedsClarification(_))
    ));

    let observed = revision_of(&rig, 2).await;
    let pause = intent(2, WireIntentKind::Pause, 91, observed);
    let response = rig
        .handle
        .machine_intent(2, &pause)
        .await
        .expect("a receipt");
    assert!(matches!(
        receipt(&response).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));

    let mut malformed = intent(
        1,
        WireIntentKind::Tap {
            offer: "ready".to_owned(),
        },
        92,
        observed,
    );
    malformed.intent.as_mut().expect("an intent").offer_id = None;
    assert!(matches!(
        rig.handle.machine_intent(1, &malformed).await,
        Err(WireIntentError::Refused(MachineIntentError::Missing(_)))
    ));

    let view = rig
        .handle
        .machine_view(AudienceSpec::Seat { seat: 9 })
        .await;
    assert!(matches!(view, Err(EngineError::UnknownSeat { seat: 9 })));
}

#[tokio::test]
async fn trace_shows_the_events_of_one_conversation_turn_and_queue_show_the_commits() {
    let rig = story_rig(&[10, 20]);
    reach_the_conversation(&rig).await;
    let request = with_operation(tap(1, "persuade_guard"), 4242);
    let reply = rig.handle.submit(request).await.expect("answered");
    assert_eq!(reply.turns[0].outcome, OutcomeKind::Applied);

    let trace = rig
        .handle
        .trace(TraceOperation::new("4242").expect("an operation"))
        .await
        .expect("a trace");
    assert!(!trace.events.is_empty(), "the operation left events");
    assert_eq!(trace.turns.len(), 1);
    let queue = rig.handle.queue_show().await.expect("queue");
    assert!(queue.commit.commits >= 6, "{:?}", queue.commit);
}

/// The documented script, run through the command line against a host on an ephemeral port: it
/// is the evaluator's route through the path, so it has to keep working.
#[tokio::test]
async fn the_documented_script_drives_the_whole_path_through_the_command_line() {
    use crate::command::split_words;
    use crate::dev::run_dev;
    use crate::test_support::{STARTER_FILES, start};

    let host = start(&STARTER_FILES).await;
    let script = format!(
        "{}/../../development/playground-scripts/h3-path.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let words = split_words(&format!("script {script}")).expect("splits");
    let output = run_dev(host.port, &words)
        .await
        .unwrap_or_else(|error| panic!("the script failed: {error}"));

    for needle in [
        "Join the table to take a seat",
        "You already have a seat.",
        "Everyone at the table needs to be ready first.",
        "the table is in explore (presenting)",
        "The gate guard: \"Halt.",
        "applied at revision",
        "(replayed)",
        "wrote the fixture h3-live",
    ] {
        assert!(
            output.contains(needle),
            "the output has no '{needle}':\n{output}"
        );
    }
    let dump = run_dev(host.port, &split_words("dump").expect("splits"))
        .await
        .expect("a dump");
    assert!(dump.contains("\"machine\": \"dialogue\""), "{dump}");
    assert!(dump.contains("concluded"), "{dump}");
}

// ---- Review defects of the H3 host work (#1747 to #1755) ----

/// A refusal whose cause is read from its key, as the wire carries it.
fn refusal(cause: &str, retry_after_ms: Option<u64>) -> RefusalReport {
    RefusalReport {
        cause: serde_json::from_value(serde_json::Value::from(cause)).expect("a cause key"),
        detail: "the queue says so".to_owned(),
        retry_after_ms,
        first_status: None,
    }
}

fn declined_code(response: &wire::SubmitMachineIntentResponse) -> wire::RejectionCode {
    let Some(wire::machine_receipt::Result::Declined(decline)) = &receipt(response).result else {
        panic!("a decline: {response:?}");
    };
    wire::RejectionCode::try_from(decline.code).expect("a code this build knows")
}

/// #1747: every cause the queue can refuse with has its own typed answer, and none of the queue's
/// own causes (which are not declines) is ever told to a phone as a decline.
#[test]
fn every_refusal_of_the_queue_has_its_own_typed_answer_and_none_reads_as_not_viable() {
    let stalled = refused(&refusal("lane_stalled", None), 1).expect_err("a stall is no receipt");
    assert_eq!(stalled, WireIntentError::Stalled);
    assert_eq!(stalled.status(), CallStatus::Unavailable);
    assert_eq!(EngineError::from(stalled), EngineError::Stalled);

    let unknown = refused(&refusal("unknown_seat", None), 4).expect_err("no such seat");
    assert_eq!(unknown, WireIntentError::UnknownSeat { seat: 4 });
    assert_eq!(unknown.status(), CallStatus::InvalidArgument);
    assert_eq!(
        EngineError::from(unknown),
        EngineError::UnknownSeat { seat: 4 }
    );

    let limited = refused(&refusal("rate_limited", Some(250)), 1).expect_err("too fast");
    assert_eq!(
        limited,
        WireIntentError::TooFast {
            retry_after_ms: Some(250)
        }
    );
    assert_eq!(limited.status(), CallStatus::ResourceExhausted);
    assert_eq!(
        EngineError::from(limited),
        EngineError::TooFast {
            retry_after_ms: Some(250)
        },
        "the dev route says the same cause the wire does, not 'mailbox full'"
    );
    assert_eq!(
        refused(&refusal("table_busy", None), 1).expect_err("busy"),
        WireIntentError::TooFast {
            retry_after_ms: None
        }
    );

    // A repeat is answered before `refused` is asked, so here it is an error and never a receipt.
    let repeat = refused(&refusal("duplicate", None), 1).expect_err("a repeat");
    assert_eq!(
        repeat,
        WireIntentError::UnknownRefusal {
            cause: RefusalCause::NotDeclined(NotDeclinedCause::Duplicate)
        }
    );
    assert_eq!(repeat.status(), CallStatus::Internal);
    // A cause is a closed key: one that names neither list does not read at all.
    assert!(
        serde_json::from_value::<RefusalCause>(serde_json::Value::from("no_such_cause")).is_err()
    );

    for cause in NotDeclinedCause::ALL {
        assert!(
            refused(&refusal(cause.key(), None), 1).is_err(),
            "{} is not a decline and is never a receipt",
            cause.key()
        );
    }
    let stale = refused(&refusal("window_closed", None), 1).expect("a decline is a receipt");
    assert!(matches!(
        stale,
        wire::machine_receipt::Result::Declined(ref decline)
            if decline.code == wire::RejectionCode::StaleOffer as i32
    ));
}

/// #1747: a declined turn with no code, or one whose line cannot make a decline, is an error and
/// never `not_viable` with the default text.
#[tokio::test]
async fn a_declined_turn_without_a_code_or_a_line_is_an_error_and_not_a_not_viable_receipt() {
    let rig = story_rig(&[]);
    rig.handle.submit(tap(1, "join")).await.expect("joined");
    let again = rig.handle.submit(tap(1, "join")).await.expect("answered");
    let mut turn = again.turns[0].clone();
    assert!(result_of_turn(&turn, 1).is_ok());

    turn.decline = None;
    assert_eq!(
        result_of_turn(&turn, 1).expect_err("no code"),
        WireIntentError::MissingDeclineCode
    );
    turn.decline = Some(DeclineCode::NotViable);
    turn.readout = String::new();
    assert!(matches!(
        result_of_turn(&turn, 1),
        Err(WireIntentError::Decline(_))
    ));
}

/// #1747: through the engine, an unknown seat, a rate limit and a seat with its inputs waiting are
/// each their own failure.
#[tokio::test]
async fn a_wire_intent_that_the_queue_refuses_fails_with_its_own_class() {
    let rig = story_rig(&[]);
    let observed = revision_of(&rig, 1).await;
    let stranger = intent(9, tap_kind("join"), 31, observed);
    assert_eq!(
        rig.handle.machine_intent(9, &stranger).await,
        Err(WireIntentError::UnknownSeat { seat: 9 }),
        "a seat the queue does not know is not told 'not viable'"
    );

    let rig = story_rig_options(RigOptions {
        default_seat_limits: true,
        ..RigOptions::default()
    });
    let observed = revision_of(&rig, 1).await;
    for operation in 1..=3 {
        let nothing = intent(1, tap_kind("nothing"), operation, observed);
        let answered = rig.handle.machine_intent(1, &nothing).await;
        assert_eq!(
            declined_code(&answered.expect("the burst is allowed")),
            wire::RejectionCode::Unspecified
        );
    }
    let fourth = intent(1, tap_kind("nothing"), 4, observed);
    let limited = rig
        .handle
        .machine_intent(1, &fourth)
        .await
        .expect_err("the bucket is empty");
    assert!(
        matches!(
            limited,
            WireIntentError::TooFast {
                retry_after_ms: Some(_)
            }
        ),
        "{limited:?}"
    );
    assert_eq!(limited.status(), CallStatus::ResourceExhausted);
    assert!(matches!(
        EngineError::from(limited),
        EngineError::TooFast {
            retry_after_ms: Some(_)
        }
    ));

    let rig = story_rig_options(RigOptions {
        held: true,
        default_seat_limits: true,
        ..RigOptions::default()
    });
    let observed = revision_of(&rig, 1).await;
    for operation in 1..=3 {
        let waiting = intent(1, tap_kind("nothing"), operation, observed);
        assert_eq!(
            rig.handle.machine_intent(1, &waiting).await,
            Err(WireIntentError::NotServedYet)
        );
    }
    let busy = rig
        .handle
        .machine_intent(1, &intent(1, tap_kind("nothing"), 4, observed))
        .await
        .expect_err("the seat has its limit waiting");
    assert_eq!(
        busy,
        WireIntentError::TooFast {
            retry_after_ms: None
        }
    );
}

/// #1748: an input held in the queue has no receipt (nothing ran), runs once on release, and its
/// retry under the same id is the replayed receipt and not an expiry.
#[tokio::test]
async fn a_held_input_gets_no_receipt_runs_once_when_released_and_its_retry_is_replayed() {
    let rig = story_rig_options(RigOptions {
        held: true,
        ..RigOptions::default()
    });
    let observed = revision_of(&rig, 1).await;
    let request = intent(1, tap_kind("join"), 5_000, observed);

    for _ in 0..2 {
        let held = rig.handle.machine_intent(1, &request).await;
        assert_eq!(
            held,
            Err(WireIntentError::NotServedYet),
            "no declined receipt, and the retry is not an expiry either"
        );
    }
    assert_eq!(
        WireIntentError::NotServedYet.status(),
        CallStatus::Unavailable
    );
    assert!(
        rig.handle
            .dump()
            .await
            .expect("dump")
            .table
            .seats
            .is_empty()
    );

    rig.handle
        .queue_control(QueueControl::Release)
        .await
        .expect("released");
    assert_eq!(rig.handle.dump().await.expect("dump").table.seats, [1]);

    let replayed = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect("the retry is answered");
    assert!(receipt(&replayed).replayed);
    assert!(matches!(
        receipt(&replayed).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert_eq!(
        rig.handle.dump().await.expect("dump").table.seats,
        [1],
        "one join took effect"
    );
}

fn strings_of_seat_view(view: &wire::MachineView) -> Vec<String> {
    let Some(Audience::Seat(seat)) = &view.audience else {
        panic!("a seat view");
    };
    let mut strings = vec![seat.hero.clone()];
    strings.extend(seat.narration.iter().cloned());
    strings.extend(seat.note.iter().map(|note| note.text.clone()));
    for offer in &seat.offers {
        strings.push(offer.label.clone());
        strings.extend(offer.explain.clone());
        strings.extend(offer.unavailable_reason.clone());
    }
    strings
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
}

/// Seats 1 and 2 sit; seat 3 is only in the queue. Its guest view must carry nothing of the
/// seated players that the shared screen does not, and must be the shared screen's view plus only
/// the machine's readout for an unseated seat and the unseated actions.
async fn check_guest_view_is_public(rig: &Rig, unseated_actions: &[&str]) {
    let guest = rig
        .handle
        .machine_view(AudienceSpec::Seat { seat: 3 })
        .await
        .expect("a guest view");
    let tv = rig.handle.machine_view(AudienceSpec::Tv).await.expect("tv");
    let guest_bytes = guest.encode_to_vec();
    let tv_bytes = tv.encode_to_vec();

    let mut private = 0;
    for seated in [1, 2] {
        let view = rig
            .handle
            .machine_view(AudienceSpec::Seat { seat: seated })
            .await
            .expect("a seated view");
        assert_eq!(
            view.state_label, guest.state_label,
            "a seat keeps its vocabulary when it joins"
        );
        for line in strings_of_seat_view(&view) {
            if contains_bytes(&tv_bytes, &line) {
                continue;
            }
            private += 1;
            assert!(
                !contains_bytes(&guest_bytes, &line),
                "seat {seated}'s private '{line}' reached the guest: {guest:?}"
            );
        }
    }
    assert!(
        private > 0,
        "the seated views had something private to leak"
    );

    assert_eq!(guest.machine_id, tv.machine_id);
    assert_eq!(guest.state_label, tv.state_label);
    assert_eq!(guest.status, tv.status);
    assert_eq!(guest.reason, tv.reason);
    assert_eq!(guest.revision, tv.revision);
    assert!(
        guest.readout == tv.readout || guest.readout.ends_with(&tv.readout),
        "the readout is the public one, with only a join prompt before it: '{}' vs '{}'",
        guest.readout,
        tv.readout
    );
    let Some(Audience::Seat(phone)) = &guest.audience else {
        panic!("a seat view");
    };
    assert_eq!(phone.hero, "Seat 3");
    assert_eq!(phone.narration, std::slice::from_ref(&guest.readout));
    assert!(phone.note.is_none() && phone.goal.is_none());
    let offers: Vec<&str> = phone
        .offers
        .iter()
        .map(|offer| offer.offer_id.as_str())
        .collect();
    assert_eq!(offers, unseated_actions);
    df_api::machine::check_machine_view(&guest).expect("the guard passes the guest view");
}

/// #1749: in the lobby, the story scene and the conversation, with a populated table and private
/// notices, an unseated seat's view is the public one plus only what an unseated seat is told.
#[tokio::test]
async fn the_guest_view_of_an_unseated_seat_carries_none_of_the_seated_players_data() {
    let rig = story_rig(&[10, 20]);
    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "join")).await.expect("joined");
    }
    // Each seat is told something only it reads: a private notice from a decline.
    let again = rig.handle.submit(tap(1, "join")).await.expect("answered");
    assert_eq!(again.turns[0].outcome, OutcomeKind::Declined);
    let early = rig.handle.submit(tap(2, "start")).await.expect("answered");
    assert_eq!(early.turns[0].outcome, OutcomeKind::Declined);
    check_guest_view_is_public(&rig, &["join"]).await;

    for seat in [1, 2] {
        rig.handle.submit(tap(seat, "ready")).await.expect("ready");
    }
    rig.handle.submit(tap(1, "start")).await.expect("started");
    check_guest_view_is_public(&rig, &[]).await;

    rig.handle
        .submit(tap(1, "pick_lock"))
        .await
        .expect("the check ran");
    check_guest_view_is_public(&rig, &[]).await;
}

/// #1752: an intent aimed at a machine that a hand-off replaced is declined as a stale offer and
/// runs nothing, while a retry of an operation that did run still gets its replay.
#[tokio::test]
async fn an_intent_for_a_replaced_machine_is_declined_stale_and_runs_nothing() {
    let rig = story_rig(&[10, 20]);
    start_story(&rig).await;
    let observed = revision_of(&rig, 1).await;
    let hand_off = intent_for("explore", 1, tap_kind("pick_lock"), 600, observed);
    let first = rig
        .handle
        .machine_intent(1, &hand_off)
        .await
        .expect("a receipt");
    assert!(matches!(
        receipt(&first).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    let observed = revision_of(&rig, 1).await;
    let dump = rig.handle.dump().await.expect("dump");
    assert_eq!(dump.machine, "dialogue");

    let stale = intent_for("explore", 1, tap_kind("persuade_guard"), 601, observed);
    let response = rig
        .handle
        .machine_intent(1, &stale)
        .await
        .expect("a receipt");
    assert_eq!(declined_code(&response), wire::RejectionCode::StaleOffer);
    let trace = rig
        .handle
        .trace(TraceOperation::new(&format!("{:032x}", 601)).expect("an operation"))
        .await
        .expect("a trace");
    assert!(trace.turns.is_empty(), "nothing ran: {:?}", trace.turns);
    assert_eq!(rig.handle.dump().await.expect("dump").state, dump.state);

    let again = rig
        .handle
        .machine_intent(1, &hand_off)
        .await
        .expect("the retry is answered");
    assert!(
        receipt(&again).replayed,
        "the first send did the hand-off; its retry is a replay and not stale"
    );

    let current = intent_for("dialogue", 1, tap_kind("persuade_guard"), 602, observed);
    let moved = rig
        .handle
        .machine_intent(1, &current)
        .await
        .expect("a receipt");
    assert!(matches!(
        receipt(&moved).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
}

/// #1753: an intent drawn from an older revision is not trusted as drawn: it is checked against
/// what the seat may do now, so a stale offer that is still legal applies and one that no longer
/// is legal is declined and changes nothing.
#[tokio::test]
async fn an_intent_from_a_stale_revision_is_checked_against_the_offers_of_now() {
    let rig = story_rig(&[]);
    rig.handle.submit(tap(1, "join")).await.expect("joined");
    rig.handle.submit(tap(2, "join")).await.expect("joined");
    assert!(revision_of(&rig, 1).await > 0);

    let still_legal = intent(1, tap_kind("ready"), 610, 0);
    let applied = rig
        .handle
        .machine_intent(1, &still_legal)
        .await
        .expect("a receipt");
    assert!(
        matches!(
            receipt(&applied).result,
            Some(wire::machine_receipt::Result::Applied(_))
        ),
        "{applied:?}"
    );
    assert_eq!(seat_offers(&rig, 1).await, ["unready", "start", "leave"]);

    let no_longer_legal = intent(1, tap_kind("join"), 611, 0);
    let declined = rig
        .handle
        .machine_intent(1, &no_longer_legal)
        .await
        .expect("a receipt");
    assert_eq!(declined_code(&declined), wire::RejectionCode::Unspecified);
    assert_eq!(
        seat_offers(&rig, 1).await,
        ["unready", "start", "leave"],
        "the declined intent changed nothing"
    );
}

/// #1753: the answer for an operation the owner no longer holds is the observation, not a receipt.
#[test]
fn an_operation_the_owner_forgot_is_an_expiry_observation() {
    let response = forgotten();
    let report = super::machine_io::receipt_report(&response);
    assert_eq!(report.result, "observation");
    assert_eq!(report.code.as_deref(), Some("operation_expired"));
    let request = intent(1, tap_kind("join"), 620, 1);
    let admitted = df_api::machine::admit_machine_intent(&request).expect("admitted");
    df_api::machine::check_machine_response(&response, &admitted)
        .expect("the guard lets an observation through");
}

/// #1753: the same operation id from another seat is another operation, with its own receipt.
#[tokio::test]
async fn the_same_operation_id_from_another_seat_is_not_seat_a_s_receipt() {
    let rig = story_rig(&[]);
    let observed = revision_of(&rig, 1).await;
    let first = rig
        .handle
        .machine_intent(1, &intent(1, tap_kind("join"), 720, observed))
        .await
        .expect("a receipt");
    let observed = revision_of(&rig, 2).await;
    let second_request = intent(2, tap_kind("join"), 720, observed);
    let second = rig
        .handle
        .machine_intent(2, &second_request)
        .await
        .expect("a receipt");
    assert!(
        !receipt(&second).replayed,
        "seat 2's join is its own operation"
    );
    assert!(matches!(
        receipt(&second).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert_eq!(rig.handle.dump().await.expect("dump").table.seats, [1, 2]);

    let trace = rig
        .handle
        .trace(TraceOperation::new(&format!("{:032x}", 720)).expect("an operation"))
        .await
        .expect("a trace");
    let senders: Vec<&str> = trace
        .turns
        .iter()
        .map(|turn| turn.sender.as_str())
        .collect();
    assert_eq!(senders, ["seat:1", "seat:2"]);

    let retry = rig
        .handle
        .machine_intent(2, &second_request)
        .await
        .expect("a receipt");
    assert!(receipt(&retry).replayed);
    assert_eq!(
        receipt(&retry).revision,
        receipt(&second).revision,
        "seat 2's retry returns seat 2's turn"
    );
    assert_ne!(receipt(&retry).revision, receipt(&first).revision);
}

/// #1753: an answer to a pending is declined typed, because no machine of this path opens one.
#[tokio::test]
async fn a_pending_answer_is_declined_typed() {
    let rig = story_rig(&[]);
    let observed = revision_of(&rig, 1).await;
    let mut request = intent(1, tap_kind("join"), 730, observed);
    let inner = request.intent.as_mut().expect("an intent");
    inner.payload = Some(wire::machine_intent::Payload::PendingAnswer(
        wire::MachinePendingAnswer {
            pending_id: Some(1),
        },
    ));
    let response = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect("a receipt");
    assert_eq!(declined_code(&response), wire::RejectionCode::Unspecified);
    let Some(wire::machine_receipt::Result::Declined(decline)) = &receipt(&response).result else {
        panic!("a decline");
    };
    assert!(
        decline.reason.contains("not supported"),
        "{}",
        decline.reason
    );
    assert!(
        rig.handle
            .dump()
            .await
            .expect("dump")
            .table
            .seats
            .is_empty()
    );
}

/// #1753: the headline phone flow, a guest seat joining through the wire.
#[tokio::test]
async fn a_guest_seat_joins_the_table_through_the_wire() {
    let rig = story_rig(&[]);
    let observed = revision_of(&rig, 3).await;
    let request = intent(3, tap_kind("join"), 740, observed);
    let response = rig
        .handle
        .machine_intent(3, &request)
        .await
        .expect("a receipt");
    assert!(matches!(
        receipt(&response).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert_eq!(rig.handle.dump().await.expect("dump").table.seats, [3]);
    assert_eq!(seat_offers(&rig, 3).await, ["ready", "start", "leave"]);
}

/// #1753: a commit that fails during an intent answers `table_busy` (too fast, RES40) and frees
/// the operation id, so the same id applies once the owner has backed off.
#[tokio::test]
async fn a_commit_failure_during_an_intent_answers_busy_and_frees_the_operation_id() {
    let rig = story_rig_options(RigOptions {
        script: vec![ScriptedCommit::Fails(CommitError::Unavailable)],
        ..RigOptions::default()
    });
    let observed = revision_of(&rig, 1).await;
    let request = intent(1, tap_kind("join"), 800, observed);
    let failed = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect_err("the commit failed");
    assert_eq!(
        failed,
        WireIntentError::TooFast {
            retry_after_ms: None
        }
    );
    assert!(
        rig.handle
            .dump()
            .await
            .expect("dump")
            .table
            .seats
            .is_empty()
    );

    rig.clock.advance_ms(5_000);
    let retried = rig
        .handle
        .machine_intent(1, &request)
        .await
        .expect("a receipt");
    assert!(!receipt(&retried).replayed, "the id was released");
    assert!(matches!(
        receipt(&retried).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert_eq!(rig.handle.dump().await.expect("dump").table.seats, [1]);
}

/// #1751: a read between two submits in the mailbox does not split their batch.
#[tokio::test(flavor = "current_thread")]
async fn a_view_read_between_two_submits_does_not_split_their_commit() {
    let rig = super::fixtures::blocking_rig(&RigOptions::default());
    let first = tokio::spawn({
        let handle = std::sync::Arc::clone(&rig.handle);
        async move { handle.submit(tap(1, "join")).await }
    });
    tokio::task::yield_now().await;
    rig.entered
        .recv()
        .expect("the owner began its first commit");

    let submit_a = tokio::spawn({
        let handle = std::sync::Arc::clone(&rig.handle);
        async move { handle.submit(tap(2, "join")).await }
    });
    let read = tokio::spawn({
        let handle = std::sync::Arc::clone(&rig.handle);
        async move { handle.machine_view(AudienceSpec::Tv).await }
    });
    let submit_b = tokio::spawn({
        let handle = std::sync::Arc::clone(&rig.handle);
        async move { handle.submit(tap(3, "join")).await }
    });
    tokio::task::yield_now().await;

    rig.release.send(()).expect("the store is waiting");
    rig.entered
        .recv()
        .expect("the owner began its second commit");
    rig.release.send(()).expect("the store is waiting");

    assert!(first.await.expect("joined").is_ok());
    let a = submit_a.await.expect("joined").expect("answered");
    let b = submit_b.await.expect("joined").expect("answered");
    assert!(read.await.expect("read").is_ok(), "the read is answered");
    assert_eq!(
        a.turns[0].commit, b.turns[0].commit,
        "the read in between did not end the batch"
    );
    assert_eq!(rig.store.attempts(), [(1, 1), (2, 2)]);
}

/// #1754: ids minted after a restart over the same store do not repeat the ones a checkpoint
/// remembers, so a new input is never answered as the replay of an old one.
#[tokio::test]
async fn operation_ids_minted_after_a_restart_do_not_collide_with_the_saved_ones() {
    let store = std::sync::Arc::new(ScriptedCommitStore::scripted(Vec::new()));
    let first = story_rig_on(std::sync::Arc::clone(&store));
    let one = first.handle.submit(tap(1, "join")).await.expect("joined");
    let two = first.handle.submit(tap(2, "join")).await.expect("joined");
    let wire_before = random_u128();
    let observed = revision_of(&first, 3).await;
    let before = intent_for("lobby", 3, tap_kind("join"), wire_before, observed);
    first
        .handle
        .machine_intent(3, &before)
        .await
        .expect("a receipt");
    drop(first);

    let restarted = story_rig_on(store);
    let three = restarted
        .handle
        .submit(tap(1, "join"))
        .await
        .expect("answered");
    for earlier in [&one, &two] {
        assert_ne!(
            three.submission.operation, earlier.submission.operation,
            "the owner's own ids do not restart where they began"
        );
    }
    let wire_after = random_u128();
    assert_ne!(wire_after, wire_before);
    let observed = revision_of(&restarted, 4).await;
    let after = intent_for("lobby", 4, tap_kind("join"), wire_after, observed);
    let response = restarted
        .handle
        .machine_intent(4, &after)
        .await
        .expect("a receipt");
    assert!(!receipt(&response).replayed);
    assert!(matches!(
        receipt(&response).result,
        Some(wire::machine_receipt::Result::Applied(_))
    ));
    assert!(
        restarted
            .handle
            .dump()
            .await
            .expect("dump")
            .table
            .seats
            .contains(&4),
        "the new input took effect"
    );
}

/// #1754: the ids the route mints are 128 random bits, never zero and never a counter.
#[test]
fn minted_wire_operation_ids_are_wide_distinct_and_never_zero() {
    let ids: std::collections::BTreeSet<u128> = (0..1_000).map(|_| random_u128()).collect();
    assert_eq!(ids.len(), 1_000, "no two minted ids are alike");
    assert!(ids.iter().all(|id| *id != 0));
    assert!(
        ids.iter().any(|id| *id > u128::from(u64::MAX)),
        "the ids use the high 64 bits too"
    );
}
