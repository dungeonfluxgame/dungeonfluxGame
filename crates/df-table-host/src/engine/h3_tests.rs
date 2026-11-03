//! The first playable path (O.16 H3), end to end through the real owner: a lobby where seats join
//! through the queue, the story scene, one conversation turn with its dice and rules, the group
//! commit, and the real projection served as the wire's `MachineView` and answered as a
//! `MachineReceipt`. No browser: the TV and the phone are read as the messages they are served.

use df_api::machine::MachineIntentError;
use df_protocol::common as wire;
use df_protocol::common::machine_view::Audience;
use df_table::identity::{RecoveryEpoch, SessionRevision};
use df_table::outcome::{DeclineCode, OutcomeKind};
use prost::Message;

use crate::machine_wire::wire_revision;

use super::machine_io::{WireIntentBody, WireIntentError, WireIntentKind, view_report};
use super::report::{CommitDisposition, EngineError};
use super::request::AudienceSpec;
use super::tests_support::{
    Rig, count_lines, story_rig, story_rig_df_rules, tap, text, with_operation,
};

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

fn intent(
    seat: u64,
    what: WireIntentKind,
    operation: u64,
    observed: u64,
) -> wire::SubmitMachineIntentRequest {
    let body = WireIntentBody {
        seat,
        what,
        operation: Some(operation),
        machine: None,
        observed: None,
    };
    body.request("lobby", observed, u128::from(operation))
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
    let stale = wire_revision(SessionRevision::new(RecoveryEpoch::FIRST, 0));
    assert_eq!(stale.sequence, Some(0));
}

#[tokio::test]
async fn trace_shows_the_events_of_one_conversation_turn_and_queue_show_the_commits() {
    let rig = story_rig(&[10, 20]);
    reach_the_conversation(&rig).await;
    let request = with_operation(tap(1, "persuade_guard"), 4242);
    let reply = rig.handle.submit(request).await.expect("answered");
    assert_eq!(reply.turns[0].outcome, OutcomeKind::Applied);

    let trace = rig.handle.trace("4242".to_owned()).await.expect("a trace");
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
