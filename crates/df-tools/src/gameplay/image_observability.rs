//! Image observations after host authority checks and actual authenticated browser reports.
//! This adapter owns no aliases, delivery registry, configuration or gameplay outcomes.

use df_imagery::{
    AssetId, ImageJobId,
    telemetry::{DeliveryId, EventContext, EventKind, ImageEvent, ImageObserver},
};
use df_observe::{ProducerId, presentation::TimingEvent};
use df_types::OperationId;

/// Actual authorized publication; a missing frozen configuration revision remains unknown.
#[derive(Clone, Debug)]
pub(super) struct PublishedObservation {
    pub(super) operation: OperationId,
    pub(super) job: ImageJobId,
    pub(super) asset: AssetId,
    pub(super) config_revision: Option<u64>,
}

impl PublishedObservation {
    pub(super) fn context(&self) -> EventContext {
        EventContext {
            operation: Some(self.operation),
            job: Some(self.job),
            asset: Some(self.asset),
            config_revision: self.config_revision,
            ..EventContext::default()
        }
    }
}

/// The caller emits only after its execution gate acknowledged this effective revision.
pub(super) fn configuration_changed(observer: &dyn ImageObserver, revision: u64) {
    if revision == 0 {
        return;
    }
    observer.observe(ImageEvent::new(
        EventKind::ConfigurationChanged,
        EventContext {
            config_revision: Some(revision),
            ..EventContext::default()
        },
    ));
}

/// Bytes and canonical source/grant checks have succeeded; this is not a display claim.
pub(super) fn delivery_authorized(
    observer: &dyn ImageObserver,
    mut context: EventContext,
    delivery: Option<DeliveryId>,
) {
    context.delivery = delivery;
    let mut event = ImageEvent::new(EventKind::DeliveryAuthorized, context);
    event.measurements.delivery_identified = Some(delivery.is_some());
    observer.observe(event);
}

/// This existing browser event names the actual plate asset, unlike figure actor identities.
pub(super) fn displayed_asset_identity(report: &TimingEvent) -> Option<u64> {
    (report.kind == "plate-shown" && report.reason == "ready" && report.id != 0)
        .then_some(report.id)
}

/// Only an authenticated, deduplicated report and freshly authorized publication reach here.
/// Browser clock and producer remain explicit; no matching server delivery is inferred.
pub(super) fn displayed(
    observer: &dyn ImageObserver,
    publication: &PublishedObservation,
    report: &TimingEvent,
    producer: ProducerId,
) {
    let Some(identity) = displayed_asset_identity(report) else {
        return;
    };
    let mut context = publication.context();
    context.browser_producer = Some(producer);
    let mut event = ImageEvent::new(EventKind::Displayed, context);
    event.measurements.browser_asset_identity = Some(identity);
    event.measurements.browser_scene_identity = Some(report.scene);
    event.measurements.browser_at_ms = Some(report.at_ms);
    observer.observe(event);
}

#[cfg(test)]
#[path = "image_observability_tests.rs"]
mod tests;
