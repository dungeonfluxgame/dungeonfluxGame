//! The mapper from the real projections to the wire's `MachineView` (plan O.16 W0, issue #1687).
//! Every test builds a real table and projects it with `df_table::projection::project`; nothing
//! here stands in for the projection.

use df_api::machine::{admit_machine_intent, check_machine_receipt, sample};
use df_protocol::common as wire;
use df_protocol::common::machine_view::Audience;
use df_table::outcome::{Decline, DeclineCode};
use df_table::ports::Audience as Viewer;
use df_table::projection::{Attachments, AudienceProjection, ProjectionStatus, StatusKind};
use df_table::table_state::TableSnapshot;
use prost::Message;

use crate::machine_wire::{WireDecline, machine_view, wire_decline};
use crate::projection_samples::{
    PIP_SEAT, VEX_SEAT, busy_table, empty_table, paused_table, projection_of, published, seat,
    shared, table_with_two_heroes,
};

type Table = (TableSnapshot, Attachments);

fn projection_for(table: &Table, audience: Viewer) -> AudienceProjection {
    projection_of(&table.0, &table.1, audience)
}

fn seat_view_of(table: &Table, number: u64) -> wire::MachineView {
    machine_view(&projection_for(table, seat(number))).expect("the seat's view maps")
}

fn shared_view_of(table: &Table) -> wire::MachineView {
    machine_view(&projection_for(table, shared())).expect("the shared view maps")
}

fn seat_part(view: &wire::MachineView) -> &wire::MachineSeatView {
    match &view.audience {
        Some(Audience::Seat(seat)) => seat,
        other => panic!("expected a seat view, found {other:?}"),
    }
}

fn carries(bytes: &[u8], text: &[u8]) -> bool {
    bytes.windows(text.len()).any(|window| window == text)
}

#[test]
fn a_real_seat_projection_maps_to_a_checked_wire_view() {
    let table = busy_table();
    let view = seat_view_of(&table, PIP_SEAT);
    // With no machine running the projection names the table itself and a plain state.
    assert_eq!(view.machine_id, "table");
    assert_eq!(view.state_label, "waiting");
    // The table's second commit: the revision is the projection's, epoch and sequence.
    assert_eq!(
        view.revision,
        Some(wire::SessionRevision {
            epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
            sequence: Some(2),
        })
    );
    let status = view.status.as_ref().expect("a status");
    assert_eq!(status.kind, wire::MachineStatusKind::Telling as i32);
    assert!(
        view.reason
            .as_deref()
            .is_some_and(|reason| !reason.trim().is_empty())
    );
    let phone = seat_part(&view);
    assert_eq!(phone.hero, "Pip");
    assert!(
        phone
            .narration
            .iter()
            .any(|line| line.contains("loose stone"))
    );
    assert_eq!(
        phone.note.as_ref().map(|note| note.text.as_str()),
        Some("The loose stone hides a key.")
    );
    // The readout is the projection's own, copied, and is never blank.
    let AudienceProjection::Seat(projection) = projection_for(&table, seat(PIP_SEAT)) else {
        panic!("a seat projection");
    };
    assert_eq!(view.readout, projection.readout().as_str());
    assert!(!view.readout.trim().is_empty());
}

#[test]
fn the_projections_offers_reach_the_wire_with_their_ids_groups_and_reasons() {
    let table = busy_table();
    let view = seat_view_of(&table, PIP_SEAT);
    let offers = &seat_part(&view).offers;
    let find = |id: &str| {
        offers
            .iter()
            .find(|offer| offer.offer_id == id)
            .unwrap_or_else(|| panic!("offer {id} is on the wire"))
    };
    let look = find("look-around");
    assert_eq!(look.label, "Look around");
    assert_eq!(
        look.explain.as_deref(),
        Some("Take a slow look at the cellar.")
    );
    assert!(look.group.is_none() && look.unavailable_reason.is_none());
    let chest = find("open-the-chest");
    assert_eq!(chest.group.as_deref(), Some("Investigate"));
    assert_eq!(
        chest.unavailable_reason.as_deref(),
        Some("The chest is locked.")
    );
    // Every offer the projection holds is there, in order, under its own id.
    let AudienceProjection::Seat(projection) = projection_for(&table, seat(PIP_SEAT)) else {
        panic!("a seat projection");
    };
    let ids: Vec<&str> = projection
        .offers()
        .iter()
        .map(|offer| offer.offer_id.as_str())
        .collect();
    let wire_ids: Vec<&str> = offers.iter().map(|offer| offer.offer_id.as_str()).collect();
    assert_eq!(wire_ids, ids);
}

#[test]
fn the_shared_screen_maps_with_its_party_and_caption_and_no_choices() {
    let table = busy_table();
    let view = shared_view_of(&table);
    let Some(Audience::SharedScreen(screen)) = &view.audience else {
        panic!("expected a shared screen, found {:?}", view.audience);
    };
    assert_eq!(screen.place, "The cellar stairs");
    let caption = screen.caption.as_ref().expect("a caption");
    assert_eq!(caption.speaker.as_deref(), Some("Pip"));
    assert_eq!(caption.text, "Pip swings at the goblin and it yelps.");
    let party: Vec<(&str, &[String])> = screen
        .party
        .iter()
        .map(|member| (member.name.as_str(), member.conditions.as_slice()))
        .collect();
    assert_eq!(party[0], ("Pip", ["prone".to_owned()].as_slice()));
    assert_eq!(party[1], ("Vex", ["hidden".to_owned()].as_slice()));
    // Nothing the shared screen shows is a seat's own: not the secret, the private line, the
    // decline or an offer.
    let bytes = view.encode_to_vec();
    for secret in [
        &b"loose stone"[..],
        b"hides a key",
        b"your turn",
        b"Look around",
        b"open-the-chest",
    ] {
        assert!(
            !carries(&bytes, secret),
            "the shared view carries {}",
            String::from_utf8_lossy(secret)
        );
    }
}

#[test]
fn another_seat_gets_no_note_no_offers_and_none_of_pips_words() {
    let table = busy_table();
    let view = seat_view_of(&table, VEX_SEAT);
    let phone = seat_part(&view);
    assert_eq!(phone.hero, "Vex");
    assert!(phone.note.is_none() && phone.offers.is_empty());
    let bytes = view.encode_to_vec();
    for secret in [
        &b"loose stone"[..],
        b"hides a key",
        b"Wait for Vex",
        b"open-the-chest",
    ] {
        assert!(!carries(&bytes, secret));
    }
}

#[test]
fn a_paused_table_waits_on_the_host_with_its_words() {
    let table = paused_table();
    let view = seat_view_of(&table, PIP_SEAT);
    let status = view.status.as_ref().expect("a status");
    assert_eq!(status.kind, wire::MachineStatusKind::WaitingFor as i32);
    assert_eq!(status.waiting_who.as_deref(), Some("the host"));
    assert_eq!(status.waiting_what.as_deref(), Some("to resume the game"));
    assert!(
        view.reason
            .as_deref()
            .is_some_and(|reason| reason.contains("the host"))
    );
}

#[test]
fn every_projection_status_maps_and_only_ok_has_no_reason() {
    let snapshot = table_with_two_heroes().snapshot();
    let statuses = [
        ProjectionStatus::Ok,
        ProjectionStatus::Reading,
        ProjectionStatus::Rolling,
        ProjectionStatus::Telling,
        ProjectionStatus::ArtDelayed,
        ProjectionStatus::Degraded,
        ProjectionStatus::ReadOnly,
        ProjectionStatus::Reconnecting,
        ProjectionStatus::waiting_for("Sam", "a choice").expect("words"),
    ];
    assert_eq!(statuses.len(), StatusKind::ALL.len());
    for status in statuses {
        let key = status.kind().key();
        let mut attachments = published(snapshot.revision());
        attachments
            .set_activity(status.clone())
            .expect("room for an activity");
        let table = (snapshot.clone(), attachments);
        for view in [seat_view_of(&table, PIP_SEAT), shared_view_of(&table)] {
            let is_ok = status == ProjectionStatus::Ok;
            assert_eq!(view.reason.is_none(), is_ok, "{key}");
            let kind = view.status.as_ref().map(|status| status.kind);
            assert_ne!(
                kind,
                Some(wire::MachineStatusKind::Unspecified as i32),
                "{key}"
            );
        }
    }
}

#[test]
fn a_table_before_its_first_commit_maps_at_the_genesis_revision() {
    // The projection of an empty table carries the genesis revision, which the wire accepts
    // (epoch one, sequence zero), so a table that has said nothing still fills every slot.
    let table = empty_table();
    for view in [seat_view_of(&table, PIP_SEAT), shared_view_of(&table)] {
        assert_eq!(
            view.revision,
            Some(wire::SessionRevision {
                epoch: Some(wire::RecoveryEpoch { value: Some(1) }),
                sequence: Some(0),
            })
        );
        assert!(!view.readout.trim().is_empty());
    }
}

/// The public class each decline code reaches the wire as (RES40). Every code is named: a code
/// added to `DeclineCode` is a compile error here until it is given a class.
fn expected_class(code: DeclineCode) -> Option<wire::RejectionCode> {
    use wire::RejectionCode as Class;
    match code {
        DeclineCode::NotYourTurn => Some(Class::WrongTurn),
        DeclineCode::WindowClosed | DeclineCode::StockChanged => Some(Class::StaleOffer),
        DeclineCode::IntentInvalid => Some(Class::InvalidSelection),
        DeclineCode::ResourceSpent | DeclineCode::EconomySpent => Some(Class::ResourceMissing),
        DeclineCode::TimeLocked | DeclineCode::PendingOpen => Some(Class::Paused),
        DeclineCode::PvpOff | DeclineCode::ConsentDeclined => Some(Class::CapabilityUnavailable),
        DeclineCode::TableBusy | DeclineCode::RateLimited => None,
        DeclineCode::NotViable
        | DeclineCode::NotSupportedYet
        | DeclineCode::NotInCombat
        | DeclineCode::NoCombatProfile
        | DeclineCode::MissingRequirement
        | DeclineCode::NotKnowable
        | DeclineCode::SameUrging
        | DeclineCode::CastingRestricted => Some(Class::Unspecified),
    }
}

#[test]
fn every_decline_code_reaches_the_wire_as_its_public_class_and_passes_the_receipt_guard() {
    let tap = admit_machine_intent(&sample::tap_request()).expect("the sample tap is admitted");
    for &code in DeclineCode::ALL {
        let decline = Decline::with_default_reason(code)
            .with_missing_piece("a key")
            .expect("a piece")
            .with_alternative("Try the window")
            .expect("an alternative");
        let key = code.key();
        match (wire_decline(&decline), expected_class(code)) {
            (WireDecline::ResourceExhausted, None) => {}
            (WireDecline::Receipt(declined), Some(class)) => {
                assert_eq!(declined.code, class as i32, "{key}");
                assert_eq!(declined.reason, decline.reason(), "{key}");
                assert_eq!(declined.missing.as_deref(), Some("a key"), "{key}");
                assert_eq!(declined.alternatives[0].offer_id, "alternative-1", "{key}");
                let receipt = wire::MachineReceipt {
                    operation_id: Some(wire::OperationId {
                        value: Some(tap.operation.as_bytes().to_vec()),
                    }),
                    revision: Some(sample::revision()),
                    replayed: false,
                    utterance_id: None,
                    result: Some(wire::machine_receipt::Result::Declined(declined)),
                };
                assert_eq!(check_machine_receipt(&receipt, &tap), Ok(()), "{key}");
            }
            (other, expected) => panic!("{key}: got {other:?}, expected {expected:?}"),
        }
    }
}
