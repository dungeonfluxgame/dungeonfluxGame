//! The shared screen's view: the place, the status, the caption, the party and the stage.
//!
//! It owns `SharedView`, whose JSON is the playground's `TvScene` (`place`, `status`, `caption`,
//! `party`, `elements`), and the build of it from a snapshot and the attachments. The view is the
//! public one, the same for everybody in the room, so it reads only what the whole table may know:
//! the public story, the focus scene and the cast. The one filter on the cast is the hiding rule:
//! every hero is always drawn and counted in the party strip, with a `hidden` chip when the party
//! cannot see it, because hiding filters only the enemy view; a creature that is not a hero and is
//! not seen by the party is left out of the stage altogether and leaves no mark, unless the party
//! knows of it and cannot see it (`Sensed`), when a marker with no name stands at its place and the
//! figure never does (K.13, M2-D17, R06, question 66).
//! A private story line or fact never reaches it, and it holds no field a seat's secret could set.
//! The caption names a speaker only when the words shown are what that speaker said: the structured
//! readout that stands in for blank narration is the table's own line and has no speaker. It never
//! fails: a blank slot is not representable, and a table with nothing to say still says where it
//! is.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use crate::element::ElementId;
use crate::identity::EvidenceRef;
use crate::projection::bounds::FIGURE_SIZE_UNITS;
use crate::projection::record::ProjectionWork;
use crate::projection::status::ProjectionStatus;
use crate::projection::text::NonEmptyText;
use crate::projection::{Attachments, overview, status_of};
use crate::table_state::{CastEntry, CastKind, DisplayName, TableSnapshot, Visibility};

const LOBBY_PLACE: &str = "The table";
const STAGE_SHAPE: &str = "circle";
const HIDDEN_FIELD: &str = "hidden";
const PORTRAIT_FIELD: &str = "portrait";
const MARKER_KIND: &str = "marker";
const MARKER_LABEL: &str = "Something unseen";

/// The line under the picture on the shared screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptionView {
    /// Who said the words, only when the text is what they said.
    pub speaker: Option<DisplayName>,
    pub text: NonEmptyText,
}

/// One hero in the party strip, which is always shown (K.13).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartyMemberView {
    pub name: DisplayName,
    pub hit_points: u32,
    pub maximum_hit_points: u32,
    /// The creature's conditions, and `hidden` when the party cannot see it.
    pub conditions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// One creature drawn on the stage.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StageElementView {
    pub id: ElementId,
    /// The kind of creature, or `marker` for a threat the party knows of and cannot see.
    pub kind: &'static str,
    pub label: DisplayName,
    pub x: f32,
    pub y: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub shape: &'static str,
    pub width_units: f32,
    pub height_units: f32,
    /// `hidden` is true on a hero the party cannot see, and `portrait` names a hero's picture.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<&'static str, Value>,
}

/// What the shared screen shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SharedView {
    pub place: NonEmptyText,
    pub status: ProjectionStatus,
    pub caption: CaptionView,
    pub party: Vec<PartyMemberView>,
    pub elements: Vec<StageElementView>,
}

/// A shared view and what was learned building it.
pub(super) struct SharedBuild {
    pub(super) view: SharedView,
    pub(super) readout: NonEmptyText,
    pub(super) lines_shown: usize,
    pub(super) newest_evidence: Option<EvidenceRef>,
}

fn place_of(snapshot: &TableSnapshot, attachments: &Attachments) -> NonEmptyText {
    match snapshot.focus_scene() {
        Some(scene) => attachments
            .place(scene)
            .cloned()
            .unwrap_or_else(|| NonEmptyText::or_fallback(&format!("Scene {scene}"))),
        None => NonEmptyText::or_fallback(LOBBY_PLACE),
    }
}

/// What the shared screen says under the picture, and the readout of the whole view.
struct Said {
    caption: CaptionView,
    readout: NonEmptyText,
    lines_shown: usize,
    evidence: Option<EvidenceRef>,
}

/// The newest public line as a caption: the storyteller's words, attributed to their speaker, when
/// there are some, otherwise the structured readout with no speaker (H.15, C.7). With no story yet,
/// the machine's readout, and failing that a plain sentence from the table's own facts.
fn said_by(snapshot: &TableSnapshot, attachments: &Attachments, work: &mut ProjectionWork) -> Said {
    if let Some(entry) = attachments.public_story().back() {
        work.story_entries_visited += 1;
        let line = &entry.line;
        let words = line
            .narration
            .as_ref()
            .and_then(|narration| NonEmptyText::new(narration.expose()).ok());
        let (text, speaker) = match words {
            Some(words) => (words, line.speaker.clone()),
            None => (line.readout.clone(), None),
        };
        return Said {
            caption: CaptionView { speaker, text },
            readout: line.readout.clone(),
            lines_shown: 1,
            evidence: line.evidence,
        };
    }
    let text = attachments
        .shared_readout()
        .cloned()
        .unwrap_or_else(|| overview(snapshot, attachments));
    Said {
        caption: CaptionView {
            speaker: None,
            text: text.clone(),
        },
        readout: text,
        lines_shown: 0,
        evidence: None,
    }
}

fn party_member(entry: &CastEntry) -> PartyMemberView {
    let conditions = entry
        .conditions()
        .iter()
        .map(|condition| condition.as_str().to_owned())
        .chain(entry.visibility().chip().map(str::to_owned))
        .collect();
    PartyMemberView {
        name: entry.name().clone(),
        hit_points: entry.vitals().hit_points(),
        maximum_hit_points: entry.vitals().maximum_hit_points(),
        conditions,
        color: entry.color().map(|color| color.to_hex()),
    }
}

fn stage_element(entry: &CastEntry, x: f32, y: f32) -> StageElementView {
    let mut fields = BTreeMap::new();
    if entry.kind() == CastKind::Party && entry.visibility().chip().is_some() {
        fields.insert(HIDDEN_FIELD, Value::Bool(true));
    }
    if let Some(portrait) = entry.portrait() {
        fields.insert(PORTRAIT_FIELD, Value::String(portrait.as_str().to_owned()));
    }
    StageElementView {
        id: entry.id().clone(),
        kind: entry.kind().key(),
        label: entry.name().clone(),
        x,
        y,
        color: entry.color().map(|color| color.to_hex()),
        shape: STAGE_SHAPE,
        width_units: FIGURE_SIZE_UNITS,
        height_units: FIGURE_SIZE_UNITS,
        fields,
    }
}

/// A marker for a threat the party knows of and cannot see: where it is, and nothing of what it is.
/// Its id and label are the table's own words, so neither names the creature (question 66). The
/// words are valid names by construction, so a refusal could only mean there is nothing to draw.
fn marker_element(number: usize, x: f32, y: f32) -> Option<StageElementView> {
    Some(StageElementView {
        id: ElementId::new(&format!("marker-{number}")).ok()?,
        kind: MARKER_KIND,
        label: DisplayName::new(MARKER_LABEL).ok()?,
        x,
        y,
        color: None,
        shape: STAGE_SHAPE,
        width_units: FIGURE_SIZE_UNITS,
        height_units: FIGURE_SIZE_UNITS,
        fields: BTreeMap::new(),
    })
}

/// Builds the shared view. One pass over the cast serves both the party strip and the stage.
pub(super) fn build(
    snapshot: &TableSnapshot,
    attachments: &Attachments,
    work: &mut ProjectionWork,
) -> SharedBuild {
    let focus = snapshot.focus_scene();
    let mut party = Vec::new();
    let mut elements = Vec::new();
    let mut markers = 0;
    for entry in snapshot.cast() {
        work.cast_visited += 1;
        let is_hero = entry.kind() == CastKind::Party;
        if is_hero {
            party.push(party_member(entry));
        }
        let on_stage = Some(entry.scene()) == focus;
        let Some(placement) = entry.placement() else {
            continue;
        };
        if !on_stage {
            continue;
        }
        let (x, y) = (placement.x_units(), placement.y_units());
        match (entry.visibility(), is_hero) {
            (Visibility::Seen, _) | (_, true) => elements.push(stage_element(entry, x, y)),
            (Visibility::Sensed, false) => {
                elements.extend(marker_element(markers, x, y));
                markers += 1;
            }
            (Visibility::Hidden | Visibility::Invisible, false) => {}
        }
    }
    let said = said_by(snapshot, attachments, work);
    SharedBuild {
        view: SharedView {
            place: place_of(snapshot, attachments),
            status: status_of(snapshot, attachments),
            caption: said.caption,
            party,
            elements,
        },
        readout: said.readout,
        lines_shown: said.lines_shown,
        newest_evidence: said.evidence,
    }
}
