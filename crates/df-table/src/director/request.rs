//! What goes into the Director and what comes out: a `Request`, the `Receipt` it gets at once and
//! the `StageDirection` a tick answers with (O.14, G1).

use crate::director_error::DirectorError;
use crate::director_log::DropReason;
use crate::director_reason::{MAXIMUM_CAMERA_MS, Reason, WideReason, check_focus_move, check_zoom};
use crate::element::ElementId;

/// One thing that happened that the camera may answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub reason: Reason,
    /// The element to look at; several mean one result named several heroes (an area).
    pub targets: Vec<ElementId>,
    /// A zoom other than the reason's default.
    pub zoom_percent: Option<u16>,
    /// A move time other than the reason's default.
    pub over_ms: Option<u32>,
    /// A hold other than the default: for a moment reason `focus_hold_ms`, for one that ends with
    /// its reason the safety limit.
    pub hold_ms: Option<u64>,
    /// The result this request is about: requests of one result that name different heroes are one
    /// area, in one host tick or across several.
    pub result: Option<u64>,
    /// A person at the dev command line drove it, so `director pause` and `auto` do not hold it.
    pub driven: bool,
}

impl Request {
    pub fn focus(reason: Reason, element: ElementId) -> Self {
        Self::area(reason, vec![element])
    }

    /// One result that names several elements.
    pub fn area(reason: Reason, elements: Vec<ElementId>) -> Self {
        Self {
            reason,
            targets: elements,
            zoom_percent: None,
            over_ms: None,
            hold_ms: None,
            result: None,
            driven: false,
        }
    }

    pub fn zoom(mut self, zoom_percent: u16) -> Self {
        self.zoom_percent = Some(zoom_percent);
        self
    }

    pub fn over(mut self, over_ms: u32) -> Self {
        self.over_ms = Some(over_ms);
        self
    }

    pub fn hold(mut self, hold_ms: u64) -> Self {
        self.hold_ms = Some(hold_ms);
        self
    }

    pub fn result(mut self, result: u64) -> Self {
        self.result = Some(result);
        self
    }

    pub fn driven(mut self) -> Self {
        self.driven = true;
        self
    }

    pub(super) fn check(&self, maximum_targets: usize) -> Result<(), DirectorError> {
        if self.targets.is_empty() {
            return Err(DirectorError::NoTargets);
        }
        if self.targets.len() > maximum_targets {
            return Err(DirectorError::TooManyTargets {
                limit: maximum_targets,
            });
        }
        if let Some(zoom) = self.zoom_percent {
            check_zoom(zoom)?;
        }
        if let Some(over) = self.over_ms {
            check_focus_move(over)?;
        }
        if let Some(hold) = self.hold_ms
            && !(1..=u64::from(MAXIMUM_CAMERA_MS)).contains(&hold)
        {
            return Err(DirectorError::HoldOutOfRange { hold_ms: hold });
        }
        Ok(())
    }

    /// The element the entry names: one target, or none for an area.
    pub(super) fn element(&self) -> Option<ElementId> {
        match self.targets.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        }
    }
}

/// What the Director said at once about a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Receipt {
    /// It waits for the next host tick (`tick` decides).
    Queued,
    Dropped(DropReason),
}

/// One move of the shared camera, for the host to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StageDirection {
    Focus {
        element: ElementId,
        reason: Reason,
        /// The zoom to draw: the zoom asked, held to the plate's ceiling.
        zoom_percent: u16,
        /// The zoom that was asked when it was cut.
        clamped_from: Option<u16>,
        over_ms: u32,
        serial: u64,
        driven: bool,
    },
    Wide {
        reason: WideReason,
        over_ms: u32,
        serial: u64,
        driven: bool,
    },
}
