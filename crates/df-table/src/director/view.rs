//! What the Director sees of the shared screen when it decides (O.14, G1).
//!
//! The Director resolves a focus target from the audience's own view, so the camera can only
//! zoom to something the whole table may see (camera rule 1). `StageView` is that view as plain
//! data a caller builds each host tick: whether a plate or only a card is showing, which targets
//! are on the stage and who may see them, whether a result card or a reveal beat is holding the
//! camera still, and the two settings that change a move (a collapsed beat, reduced effects).
//! The Director keeps none of it between calls, so a stale view cannot outlive the tick it
//! described.
//!
//! The ceiling of a zoom is the plate's: the least of the model's largest zoom, the plate's width
//! in pixels over the stage's width in device pixels, and the zoom asked for, never below the whole
//! stage (`zoom_ceiling_percent`, the same rule `df-playground` draws with).

use std::collections::BTreeMap;

use crate::director::reason::{MAXIMUM_ZOOM_PERCENT, MINIMUM_ZOOM_PERCENT};
use crate::element::ElementId;
use crate::plate::timing::PLATE_DISSOLVE_MINIMUM_MS;

/// What stands behind the world layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlateView {
    /// The establishing card or a place card: there is no plate and no anchor, so every focus is
    /// declined (K.2).
    Card,
    /// A generated plate. `generation` counts the plates of a scene, so a swap is seen as a change
    /// of it; `width_px` is the picture's own width when the library knows it.
    Plate {
        generation: u64,
        width_px: Option<u32>,
    },
}

/// What one element on the stage is to the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetView {
    /// Whether the whole table may see it (R06). A hidden creature is not targetable.
    pub visible: bool,
    /// Every hero is always visible (K.13), so a hero may always be focused.
    pub hero: bool,
    /// Whether it has an anchor on the plate now showing (a narrator has none).
    pub anchored: bool,
}

impl TargetView {
    /// A visible creature or object with an anchor.
    pub const SEEN: Self = Self {
        visible: true,
        hero: false,
        anchored: true,
    };

    /// A hero: always visible.
    pub const HERO: Self = Self {
        visible: true,
        hero: true,
        anchored: true,
    };

    /// Hidden from the party.
    pub const HIDDEN: Self = Self {
        visible: false,
        hero: false,
        anchored: true,
    };

    /// Whether the camera may look at it.
    pub fn focusable(&self) -> bool {
        self.hero || self.visible
    }
}

/// What holds the camera still.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeatKind {
    /// A dice or result card, shown at least 3000 ms.
    ResultCard,
    /// A reveal beat (K.7): a move that belongs to the reveal may start, no other.
    RevealBeat,
}

/// A beat that keeps the camera where it is until `until_ms` (O.13 rule 11, O.14 rule 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeatHold {
    pub kind: BeatKind,
    pub until_ms: u64,
    /// What the beat is about: a move onto it is allowed during the beat.
    pub subject: Option<ElementId>,
}

/// The shared screen as the Director sees it for one decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageView {
    pub plate: PlateView,
    /// The stage's width in device pixels, for the zoom ceiling; 0 when unknown.
    pub stage_width_px: u32,
    pub targets: BTreeMap<ElementId, TargetView>,
    /// A K.12 collapsed beat: `active_creature` does not fire.
    pub collapsed_beat: bool,
    /// `reduced_effects`: every move is a cut.
    pub reduced_effects: bool,
    pub beat: Option<BeatHold>,
    /// How long a re-aim after a plate swap takes: the same dissolve as the swap, never less than
    /// the 1.5 s minimum.
    pub swap_dissolve_ms: u64,
}

impl StageView {
    /// An empty stage with a plate of unknown width.
    pub fn new(plate: PlateView) -> Self {
        Self {
            plate,
            stage_width_px: 0,
            targets: BTreeMap::new(),
            collapsed_beat: false,
            reduced_effects: false,
            beat: None,
            swap_dissolve_ms: PLATE_DISSOLVE_MINIMUM_MS,
        }
    }

    /// The view with one more target on the stage.
    pub fn with_target(mut self, id: ElementId, target: TargetView) -> Self {
        self.targets.insert(id, target);
        self
    }

    pub fn target(&self, id: &ElementId) -> Option<&TargetView> {
        self.targets.get(id)
    }

    /// The plate's generation, or `None` while a card shows.
    pub fn plate_generation(&self) -> Option<u64> {
        match self.plate {
            PlateView::Card => None,
            PlateView::Plate { generation, .. } => Some(generation),
        }
    }

    /// The zoom this plate can bear on this stage.
    pub fn zoom_ceiling_percent(&self) -> u16 {
        let width = match self.plate {
            PlateView::Card => None,
            PlateView::Plate { width_px, .. } => width_px,
        };
        zoom_ceiling_percent(width, self.stage_width_px)
    }
}

/// The zoom a picture can bear, in percent: the least of the largest zoom the model allows and the
/// plate's width in pixels over the stage's width in device pixels (a plate drawn larger than its
/// own pixels is a blur), and never less than the whole stage. With no plate known, or a stage of
/// no width, only the model's largest applies.
pub fn zoom_ceiling_percent(plate_width_px: Option<u32>, stage_width_px: u32) -> u16 {
    let (Some(plate), true) = (plate_width_px, stage_width_px > 0) else {
        return MAXIMUM_ZOOM_PERCENT;
    };
    let bearable = u64::from(plate) * 100 / u64::from(stage_width_px);
    let floor = u64::from(MINIMUM_ZOOM_PERCENT);
    let ceiling = u64::from(MAXIMUM_ZOOM_PERCENT);
    u16::try_from(bearable.clamp(floor, ceiling)).unwrap_or(MAXIMUM_ZOOM_PERCENT)
}

/// The zoom that is drawn when `requested` is asked for under `ceiling`, and the ceiling itself when
/// the request was cut, so it is logged and never cut silently (`CAMERA clamped requested=400
/// allowed=210`).
pub fn clamped_zoom(requested: u16, ceiling: u16) -> (u16, Option<u16>) {
    if requested > ceiling {
        (ceiling, Some(ceiling))
    } else {
        (requested, None)
    }
}
