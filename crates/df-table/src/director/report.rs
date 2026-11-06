//! `director show`: the Director's state and log as one audience may read them.

use super::Director;
use crate::director::log::{ActiveReport, DirectorReport, EndsReport, WaitingReport};
use crate::director::reason::Ends;
use crate::director::view::StageView;
use crate::element::ElementId;
use crate::inspect::InspectAudience;

/// The element's name for this audience: hidden targets are never named to the shared audience.
fn shown(audience: InspectAudience, view: &StageView, element: &ElementId) -> Option<ElementId> {
    let visible = view
        .target(element)
        .is_some_and(|target| target.focusable());
    (audience.is_operator() || visible).then(|| element.clone())
}

impl Director {
    /// The report for `audience`, with the newest `log_limit` entries (oldest first). `view` is the
    /// shared screen now: it says what the plate can bear, and which names the shared audience may
    /// be told.
    pub fn report(
        &self,
        audience: InspectAudience,
        view: &StageView,
        log_limit: usize,
    ) -> DirectorReport {
        let skip = self.log.len().saturating_sub(log_limit);
        DirectorReport {
            audience,
            now_ms: self.now_ms,
            paused: self.paused,
            auto_camera: self.auto_camera,
            serial: self.serial,
            ceiling_percent: view.zoom_ceiling_percent(),
            active: self.active.as_ref().map(|active| ActiveReport {
                element: shown(audience, view, &active.element),
                reason: active.reason,
                serial: active.serial,
                zoom_percent: active.zoom_percent,
                since_ms: active.since_ms,
                ends: match active.ends {
                    Ends::AfterHold => EndsReport::At(active.deadline_ms),
                    Ends::WhenReasonEnds if active.ended => EndsReport::At(active.deadline_ms),
                    Ends::WhenReasonEnds => EndsReport::WhenReasonEnds(active.deadline_ms),
                },
            }),
            waiting: self.pending.as_ref().map(|waiting| WaitingReport {
                element: waiting
                    .request
                    .element()
                    .and_then(|element| shown(audience, view, &element)),
                reason: waiting.request.reason,
                since_ms: waiting.at_ms,
            }),
            queued: self.queued,
            applied: self.applied,
            dropped: self.dropped,
            log: self
                .log
                .iter()
                .skip(skip)
                .map(|entry| entry.for_audience(audience))
                .collect(),
        }
    }
}
