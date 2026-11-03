//! The boundary between the table's audience projections and the wire's `MachineView`.
//!
//! The wire types (`df-protocol`) and their guards (`df-api::machine`) do not depend on
//! `df-table`, and `df-api` does not depend on it either (O.8): this module is the one place that
//! knows both, so the mapping is here and nowhere else. `machine_view` turns a
//! `df_table::projection::AudienceProjection` into a `MachineView` and ends with
//! `check_machine_view`, so a projection that breaks a wire rule is a typed `MachineMapError` and
//! never a blank screen or a malformed message.
//!
//! The projection carries what the wire needs beside the audience's view (revision, machine id,
//! state label, readout, reason, offers with their ids), so most of the mapping is a copy. What
//! the wire does not carry yet, and what the mapper does with it (each is an additive wire field
//! later):
//!
//! | Projection | The wire |
//! |---|---|
//! | shared screen `elements` (id, kind, position, shape, `fields.hidden`, `portrait`) | dropped: the wire has no stage yet |
//! | hero and element `color`, hero `portrait` | not on the wire |
//! | seat `groups` and `unavailable` | carried by the offers (`MachineOffer.group`, `unavailable_reason`), which are the one list |
//! | `attachment_version`, `truncation` | not on the wire (the revision orders frames; truncation is typed data a client may later show) |
//! | `goal`, backdrop, card, image | the projection has none yet |
//!
//! It never decides what an audience may see (the projection did) and never reads the clock.

use std::fmt;

use df_api::machine::{MachineViewError, check_machine_view};
use df_protocol::common as wire;
use df_protocol::common::machine_view::Audience;
use df_table::identity::SessionRevision;
use df_table::outcome::{Decline, PublicClass};
use df_table::projection::{
    AudienceProjection, OfferView, Projection, ProjectionStatus, SeatView, SharedView,
};

/// Why a projection could not be sent as a `MachineView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MachineMapError {
    /// The mapped view broke a wire rule (`df_api::machine::check_machine_view`).
    Wire(MachineViewError),
}

impl fmt::Display for MachineMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => write!(formatter, "the view breaks a wire rule: {error:?}"),
        }
    }
}

impl std::error::Error for MachineMapError {}

pub(crate) fn wire_revision(revision: SessionRevision) -> wire::SessionRevision {
    wire::SessionRevision {
        epoch: Some(wire::RecoveryEpoch {
            value: Some(revision.epoch().get()),
        }),
        sequence: Some(revision.sequence()),
    }
}

/// The status as the wire says it. The words that explain it are the projection's `reason`.
fn wire_status(status: &ProjectionStatus) -> wire::MachineStatus {
    let (kind, who, what) = match status {
        ProjectionStatus::Ok => (wire::MachineStatusKind::Ok, None, None),
        ProjectionStatus::Reading => (wire::MachineStatusKind::Reading, None, None),
        ProjectionStatus::Rolling => (wire::MachineStatusKind::Rolling, None, None),
        ProjectionStatus::Telling => (wire::MachineStatusKind::Telling, None, None),
        ProjectionStatus::ArtDelayed => (wire::MachineStatusKind::ArtDelayed, None, None),
        ProjectionStatus::Degraded => (wire::MachineStatusKind::Degraded, None, None),
        ProjectionStatus::ReadOnly => (wire::MachineStatusKind::ReadOnly, None, None),
        ProjectionStatus::Reconnecting => (wire::MachineStatusKind::Reconnecting, None, None),
        ProjectionStatus::WaitingFor { who, what } => (
            wire::MachineStatusKind::WaitingFor,
            Some(who.as_str().to_owned()),
            Some(what.as_str().to_owned()),
        ),
    };
    wire::MachineStatus {
        kind: kind as i32,
        waiting_who: who,
        waiting_what: what,
    }
}

fn shared_screen(view: &SharedView) -> wire::MachineSharedScreen {
    wire::MachineSharedScreen {
        place: view.place.as_str().to_owned(),
        caption: Some(wire::MachineCaption {
            speaker: view
                .caption
                .speaker
                .as_ref()
                .map(|speaker| speaker.as_str().to_owned()),
            text: view.caption.text.as_str().to_owned(),
        }),
        backdrop: None,
        card: None,
        party: view
            .party
            .iter()
            .map(|member| wire::MachinePartyMember {
                name: member.name.as_str().to_owned(),
                hit_points: member.hit_points,
                maximum_hit_points: member.maximum_hit_points,
                conditions: member.conditions.clone(),
            })
            .collect(),
    }
}

fn wire_offer(offer: &OfferView) -> wire::MachineOffer {
    wire::MachineOffer {
        offer_id: offer.offer_id.as_str().to_owned(),
        label: offer.label.as_str().to_owned(),
        explain: offer.explain.as_ref().map(|text| text.as_str().to_owned()),
        unavailable_reason: offer
            .unavailable_reason
            .as_ref()
            .map(|text| text.as_str().to_owned()),
        group: offer.group.as_ref().map(|text| text.as_str().to_owned()),
    }
}

fn seat_view(view: &SeatView, offers: &[OfferView]) -> wire::MachineSeatView {
    wire::MachineSeatView {
        hero: view.hero.as_str().to_owned(),
        goal: None,
        narration: view
            .narration
            .iter()
            .map(|line| line.as_str().to_owned())
            .collect(),
        note: view.note.as_ref().map(|note| wire::MachineNote {
            text: note.text.as_str().to_owned(),
        }),
        image: None,
        offers: offers.iter().map(wire_offer).collect(),
    }
}

fn assemble<V>(
    projection: &Projection<V>,
    status: &ProjectionStatus,
    audience: Audience,
) -> Result<wire::MachineView, MachineMapError> {
    let view = wire::MachineView {
        revision: Some(wire_revision(projection.revision())),
        machine_id: projection.machine_id().as_str().to_owned(),
        state_label: projection.state_label().as_str().to_owned(),
        status: Some(wire_status(status)),
        reason: projection.reason().map(|text| text.as_str().to_owned()),
        readout: projection.readout().as_str().to_owned(),
        audience: Some(audience),
    };
    check_machine_view(&view).map_err(MachineMapError::Wire)?;
    Ok(view)
}

/// The wire view of one audience's projection: the shared screen's, or one seat's.
pub fn machine_view(projection: &AudienceProjection) -> Result<wire::MachineView, MachineMapError> {
    match projection {
        AudienceProjection::Shared(shared) => assemble(
            shared,
            &shared.view().status,
            Audience::SharedScreen(shared_screen(shared.view())),
        ),
        AudienceProjection::Seat(seat) => assemble(
            seat,
            &seat.view().status,
            Audience::Seat(seat_view(seat.view(), seat.offers())),
        ),
    }
}

/// How a decline reaches a client (RES40): as a `MachineDecline` in a receipt, or, for the two
/// codes that mean "too much, too fast", as a `RESOURCE_EXHAUSTED` call failure and no message.
#[derive(Debug, Clone, PartialEq)]
pub enum WireDecline {
    Receipt(wire::MachineDecline),
    ResourceExhausted,
}

/// The rejection class a decline's public class maps to. A code the plan leaves internal has no
/// class: the decline carries `REJECTION_CODE_UNSPECIFIED` and its reason text, which is all a
/// person reads (K.5).
fn rejection_code(class: PublicClass) -> Option<wire::RejectionCode> {
    match class {
        PublicClass::WrongTurn => Some(wire::RejectionCode::WrongTurn),
        PublicClass::StaleOffer => Some(wire::RejectionCode::StaleOffer),
        PublicClass::InvalidSelection => Some(wire::RejectionCode::InvalidSelection),
        PublicClass::ResourceMissing => Some(wire::RejectionCode::ResourceMissing),
        PublicClass::Paused => Some(wire::RejectionCode::Paused),
        PublicClass::CapabilityUnavailable => Some(wire::RejectionCode::CapabilityUnavailable),
        PublicClass::InternalReasonKey => Some(wire::RejectionCode::Unspecified),
        PublicClass::ResourceExhausted => None,
    }
}

/// The wire form of a decline. Its alternatives become offers numbered the way the seat view
/// numbers the alternatives of its notice (`alternative-1`, `alternative-2`, ...), so a tap on
/// one means the same thing in both.
pub fn wire_decline(decline: &Decline) -> WireDecline {
    let Some(code) = rejection_code(decline.code().public_class()) else {
        return WireDecline::ResourceExhausted;
    };
    WireDecline::Receipt(wire::MachineDecline {
        code: code as i32,
        reason: decline.reason().to_owned(),
        missing: decline.missing_piece().map(str::to_owned),
        alternatives: decline
            .alternatives()
            .iter()
            .enumerate()
            .map(|(index, label)| wire::MachineOffer {
                offer_id: format!("alternative-{}", index + 1),
                label: label.clone(),
                explain: None,
                unavailable_reason: None,
                group: None,
            })
            .collect(),
    })
}
