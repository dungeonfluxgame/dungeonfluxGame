//! Audience projections: one view for the shared screen and one per seat, as a pure function.
//!
//! `project(snapshot, attachments, audience)` builds a view from an immutable `TableSnapshot` and
//! the owners' published `Attachments` (RES30), both cheap to snapshot, so a view call never blocks
//! the owner. It does no I/O, reads no clock, commits nothing and draws no die, so it is cheap and
//! bounded: the work is the cast, the last lines of story and the newest facts of the audience,
//! and `ProjectionWork` counts it so a test can prove the growth without timing anything (H.5, O.9
//! C5). Every projection carries the revision of the snapshot (the genesis revision before the
//! first commit, which the wire accepts), the `attachment_version` and a status (H.15), and never
//! an empty slot: the place, the caption, the narration and the readout are `NonEmptyText`, a blank
//! model output is replaced by the structured readout, and a table with nothing to say still says
//! where it is. The text of a view is bounded to the wire frame (`bounds`): a published text is
//! refused when it is too long, and the story and the note show their newest entries within a
//! budget with the count left off as typed data (`Truncation`).
//!
//! Hidden information is filtered by audience in one place each. A private fact or story line
//! reaches only its seat, because attachments are filed by audience when they are published and a
//! seat's build reads only the public queue and its own. On the shared screen every hero is always
//! drawn and counted, with a `hidden` chip when the party cannot see it, while a creature that is
//! not a hero and is not seen by the party is left out and leaves no mark (K.13, M2-D17, R06). A
//! decline's text goes to the declined seat only, while it answers the table's current revision,
//! and the diagnostic record carries evidence references and length classes and never text (RES36).
//!
//! The JSON shape is what the playground's fixture reads (`df_playground::fixture`), so a real
//! projection can be drawn by the playground. A `Projection<V>` serializes as `{viewer, revision,
//! attachment_version, machine_id, state_label, readout, reason?, offers?, overlay?, truncation,
//! view}`. The fields before `view` are what the wire's `MachineView` needs beside the audience's
//! view (O.8) and, for a hidden hero's own seat, where it stands; `view` is the playground's scene.
//! For the shared screen `view` is a `TvScene`: `place`, `status`, `caption` (`speaker` only when
//! the words are the speaker's, `text`), `party` (`name`, `hit_points`, `maximum_hit_points`,
//! `conditions`, `color?`) and `elements` (`id`, `kind`, `label`, `x`, `y`, `color?`, `shape`,
//! `width_units`, `height_units`, `fields` with `hidden` on a hidden hero and `portrait`). For a
//! seat `view` is a `PhoneScene`: `hero`, `status`, `narration` (strings, never empty),
//! `suggested`, `groups` and `unavailable` when there are some, and `note` when the seat knows
//! something private; each tappable choice has its id in `offers` beside the view, since the
//! fixture's choice has none. A composer adds the fixture's `name` and `description` and puts the
//! two views under `tv` and `phone`; the contract test that parses the result as a `Fixture` lives
//! outside this crate, which must not depend on the playground. `status` serializes as the
//! playground's `Status`, and `reason` is the plain sentence for any status but `ok`. It never
//! builds a view for a seat that is not at the table.

use serde::Serialize;

use crate::identity::{SeatId, SessionRevision};
use crate::listed::listed_enum;
use crate::ports::Audience;
use crate::private_data::{LengthClass, Measured};
use crate::table_state::{TableSnapshot, TableStatus};

mod attachments;
pub mod bounds;
mod events;
mod readout;
mod record;
mod seat;
mod shared;
mod status;
mod text;

pub use attachments::{
    AttachmentError, AttachmentLimits, AttachmentVersion, Attachments, OfferEntry, OfferId,
    PUBLISHED_RECORD_VERSION, PublishedLoadError, PublishedRecord, StoryLine,
};
pub use events::ProjectionEvent;
pub use readout::StructuredReadout;
pub use record::{ProjectionRecord, ProjectionWork};
pub use seat::{
    ChoiceView, GroupView, NarrationLines, NoteView, OfferView, PositionOverlay, SeatView,
    UnavailableView,
};
pub use shared::{CaptionView, PartyMemberView, SharedView, StageElementView};
pub use status::{ProjectionStatus, StatusKind};
pub use text::{BlankText, NonEmptyText};

const PAUSED_WHO: &str = "the host";
const PAUSED_WHAT: &str = "to resume the game";
/// The machine id a view names while no machine is running: the table itself.
const NO_MACHINE_ID: &str = "table";
/// The state label a view carries while its machine has published none.
const NO_STATE_LABEL: &str = "waiting";

/// Who a projection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Viewer {
    /// The shared screen: what everyone in the room may see.
    Shared,
    Seat {
        seat: SeatId,
    },
}

impl Viewer {
    pub fn of(audience: Audience) -> Self {
        match audience {
            Audience::Public => Self::Shared,
            Audience::Seat(seat) => Self::Seat { seat },
        }
    }
}

listed_enum! {
    /// The kind of audience, without the seat, for an event line.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum AudienceKind {
        Shared,
        Seat,
    }
}

impl AudienceKind {
    pub fn of(viewer: Viewer) -> Self {
        match viewer {
            Viewer::Shared => Self::Shared,
            Viewer::Seat { .. } => Self::Seat,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Seat => "seat",
        }
    }
}

/// Why a projection could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionError {
    /// The seat is not at the table, so it has no view; it is never given an empty one.
    SeatNotAtTable { seat: SeatId },
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SeatNotAtTable { seat } => write!(formatter, "seat {seat} is not at the table"),
        }
    }
}

impl std::error::Error for ProjectionError {}

/// What a view left off to fit the wire frame and the phone's window, as typed data so a client can
/// say "older lines are not shown" without the server inventing words.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Truncation {
    /// Story entries older than the phone's window that were not shown.
    pub older_story_entries: usize,
    /// Private facts older than the note's window that were not shown.
    pub older_facts: usize,
}

/// A view, the revision and attachment version it was built at, the fields the wire's view carries
/// beside it and what it says about itself. The record alone does not serialize: it is for logs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Projection<V> {
    viewer: Viewer,
    revision: SessionRevision,
    attachment_version: AttachmentVersion,
    machine_id: NonEmptyText,
    state_label: NonEmptyText,
    readout: NonEmptyText,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<NonEmptyText>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    offers: Vec<OfferView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overlay: Option<PositionOverlay>,
    truncation: Truncation,
    view: V,
    #[serde(skip)]
    record: ProjectionRecord,
}

impl<V> Projection<V> {
    pub fn viewer(&self) -> Viewer {
        self.viewer
    }

    /// The revision of the snapshot it was built from: the genesis revision before the first
    /// commit.
    pub fn revision(&self) -> SessionRevision {
        self.revision
    }

    /// The version of the attachments it was built from; with the revision it says whether
    /// anything changed since the last frame (RES30).
    pub fn attachment_version(&self) -> AttachmentVersion {
        self.attachment_version
    }

    /// The machine the view is of: the focus scene's active machine, else the table machine, else
    /// `table`.
    pub fn machine_id(&self) -> &NonEmptyText {
        &self.machine_id
    }

    pub fn state_label(&self) -> &NonEmptyText {
        &self.state_label
    }

    /// The plain-text summary that stands where narration is blank and is what a text-only seat
    /// reads. On the shared screen it is built from public facts only.
    pub fn readout(&self) -> &NonEmptyText {
        &self.readout
    }

    /// Why the status is not `ok`, in one plain sentence; `None` exactly when it is `ok`.
    pub fn reason(&self) -> Option<&NonEmptyText> {
        self.reason.as_ref()
    }

    /// What a seat may do now, with the ids a tap sends back. Empty on the shared screen.
    pub fn offers(&self) -> &[OfferView] {
        &self.offers
    }

    /// Where a hero the party cannot see stands, for its own seat's phone only.
    pub fn overlay(&self) -> Option<PositionOverlay> {
        self.overlay
    }

    pub fn truncation(&self) -> Truncation {
        self.truncation
    }

    pub fn view(&self) -> &V {
        &self.view
    }

    pub fn record(&self) -> &ProjectionRecord {
        &self.record
    }
}

/// A projection for either kind of audience.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AudienceProjection {
    Shared(Projection<SharedView>),
    Seat(Projection<SeatView>),
}

impl AudienceProjection {
    pub fn viewer(&self) -> Viewer {
        match self {
            Self::Shared(projection) => projection.viewer(),
            Self::Seat(projection) => projection.viewer(),
        }
    }

    pub fn record(&self) -> &ProjectionRecord {
        match self {
            Self::Shared(projection) => projection.record(),
            Self::Seat(projection) => projection.record(),
        }
    }
}

/// What the table says about itself when nobody has said anything yet: plain words from the
/// table's own facts.
pub(crate) fn overview(snapshot: &TableSnapshot, attachments: &Attachments) -> NonEmptyText {
    let text = match snapshot.focus_scene() {
        Some(scene) => {
            let place = attachments.place(scene).map_or_else(
                || format!("scene {scene}"),
                |place| place.as_str().to_owned(),
            );
            format!("The party is at {place}.")
        }
        None => format!(
            "The game has not started. Seats at the table: {}.",
            snapshot.seats().len()
        ),
    };
    NonEmptyText::or_fallback(&text)
}

/// The status of every view: the pipeline's activity, unless the table is paused, which outranks
/// it with a plain wait on the host (K.5).
pub(crate) fn status_of(snapshot: &TableSnapshot, attachments: &Attachments) -> ProjectionStatus {
    match snapshot.status() {
        TableStatus::Paused => ProjectionStatus::waiting_for(PAUSED_WHO, PAUSED_WHAT)
            .unwrap_or(ProjectionStatus::Degraded),
        TableStatus::Active | TableStatus::Idle => attachments.activity().clone(),
    }
}

/// The machine a view names and the label of its state.
fn machine_of(snapshot: &TableSnapshot, attachments: &Attachments) -> (NonEmptyText, NonEmptyText) {
    let Some(active) = snapshot.focus_machine() else {
        return (
            NonEmptyText::or_fallback(NO_MACHINE_ID),
            NonEmptyText::or_fallback(NO_STATE_LABEL),
        );
    };
    let label = attachments
        .state_label(active.machine())
        .cloned()
        .unwrap_or_else(|| NonEmptyText::or_fallback(NO_STATE_LABEL));
    (NonEmptyText::or_fallback(active.machine().as_str()), label)
}

/// The shared screen's projection of a snapshot.
pub fn project_shared(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
) -> Projection<SharedView> {
    let mut work = ProjectionWork::default();
    let built = shared::build(snapshot, attachments, &mut work);
    let revision = snapshot.revision();
    let (machine_id, state_label) = machine_of(snapshot, attachments);
    let record = ProjectionRecord {
        viewer: Viewer::Shared,
        revision,
        attachment_version: attachments.version_for(Audience::Public),
        status: built.view.status.kind(),
        elements_shown: built.view.elements.len(),
        party_shown: built.view.party.len(),
        lines_shown: built.lines_shown,
        facts_shown: 0,
        newest_line_length: LengthClass::of_length(built.view.caption.text.length_in_chars()),
        evidence: built.newest_evidence,
        work,
    };
    Projection {
        viewer: Viewer::Shared,
        revision,
        attachment_version: attachments.version_for(Audience::Public),
        machine_id,
        state_label,
        readout: built.readout,
        reason: built.view.status.reason(),
        offers: Vec::new(),
        overlay: None,
        truncation: Truncation::default(),
        view: built.view,
        record,
    }
}

/// One seat's projection of a snapshot, or an error if the seat is not at the table.
pub fn project_seat(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
    seat: SeatId,
) -> Result<Projection<SeatView>, ProjectionError> {
    let mut work = ProjectionWork::default();
    let built = seat::build(snapshot, attachments, seat, &mut work)?;
    let viewer = Viewer::Seat { seat };
    let revision = snapshot.revision();
    let (machine_id, state_label) = machine_of(snapshot, attachments);
    let record = ProjectionRecord {
        viewer,
        revision,
        attachment_version: attachments.version_for(Audience::Seat(seat)),
        status: built.view.status.kind(),
        elements_shown: 0,
        party_shown: 0,
        lines_shown: built.view.narration.len(),
        facts_shown: built.facts_shown,
        newest_line_length: built.newest_line_length,
        evidence: built.newest_evidence,
        work,
    };
    Ok(Projection {
        viewer,
        revision,
        attachment_version: attachments.version_for(Audience::Seat(seat)),
        machine_id,
        state_label,
        readout: built.readout,
        reason: built.view.status.reason(),
        offers: built.offers,
        overlay: built.overlay,
        truncation: Truncation {
            older_story_entries: built.older_story_entries,
            older_facts: built.older_facts,
        },
        view: built.view,
        record,
    })
}

/// The projection of a snapshot for an audience.
pub fn project(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
    audience: Audience,
) -> Result<AudienceProjection, ProjectionError> {
    match audience {
        Audience::Public => Ok(AudienceProjection::Shared(project_shared(
            snapshot,
            attachments,
        ))),
        Audience::Seat(seat) => {
            project_seat(snapshot, attachments, seat).map(AudienceProjection::Seat)
        }
    }
}
