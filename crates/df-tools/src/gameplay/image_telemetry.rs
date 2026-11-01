//! One owned shared telemetry Store and typed image observer for the native gameplay process.

use df_imagery::telemetry::{ImageLogLevel, ObserverHealth, SharedImageObserver};
use df_observe::{
    DiagnosticRecord, DiagnosticSink, NativeDiagnosticSink, NativeProducer, ProducerId,
    SourceSequence, TelemetryError, TelemetryLimits,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

pub(super) struct ImageTelemetry {
    store: Arc<Mutex<df_telemetry::Store>>,
    sink: Arc<NativeDiagnosticSink>,
    observer: Arc<SharedImageObserver>,
    started: Instant,
}
pub(super) struct TelemetrySettings {
    pub(super) limits: TelemetryLimits,
    pub(super) maximum_pending: usize,
    pub(super) level: ImageLogLevel,
}
impl ImageTelemetry {
    pub(super) fn open(root: &Path, settings: TelemetrySettings) -> Result<Self, TelemetryError> {
        std::fs::create_dir_all(root).map_err(|_| TelemetryError::Io)?;
        let directory = root.join(format!("gameplay-{}", std::process::id()));
        let store = df_telemetry::Store::open_configured(&directory, settings.limits)?;
        let version = concat!("df-tools-", env!("CARGO_PKG_VERSION"));
        let build = df_types::BuildIdentity::new(
            Some(version),
            Some(crate::BUILD_ID),
            Some("not-applicable"),
            Some("gameplay-imagery-v1"),
            Some("not-applicable"),
        )
        .map_err(|_| TelemetryError::InvalidIdentity)?;
        let producer = NativeProducer::for_service(
            ProducerId::new(identity_bytes())?,
            SourceSequence::new(1)?,
            &build,
            store.ingress(),
            settings.limits,
            "df-gameplay",
        )?;
        let sink = Arc::new(NativeDiagnosticSink::new(
            producer,
            settings.maximum_pending,
            identity_bytes,
        )?);
        let started = Instant::now();
        let clock = store.clock();
        let store = Arc::new(Mutex::new(store));
        let observer = Arc::new(SharedImageObserver::new(Arc::new(TimedDiagnosticSink {
            clock,
            sink: sink.clone(),
            started,
        })));
        observer.set_level(settings.level);
        Ok(Self {
            store,
            sink,
            observer,
            started,
        })
    }
    pub(super) fn diagnostics(&self) -> TelemetryDiagnostics {
        TelemetryDiagnostics {
            store: Arc::downgrade(&self.store),
            observer: self.observer.clone(),
            sink: self.sink.clone(),
            started: self.started,
        }
    }
    pub(super) fn observer(&self) -> Arc<SharedImageObserver> {
        self.observer.clone()
    }
    pub(super) fn shutdown(&mut self, timeout: Duration) -> Result<(), TelemetryError> {
        let started = std::time::Instant::now();
        let sink = self.sink.shutdown(timeout);
        let mut store = loop {
            match self.store.try_lock() {
                Ok(store) => break store,
                Err(std::sync::TryLockError::Poisoned(_)) => {
                    return sink.and(Err(TelemetryError::Internal));
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    let remaining = timeout.saturating_sub(started.elapsed());
                    if remaining.is_zero() {
                        return sink.and(Err(TelemetryError::Deadline));
                    }
                    std::thread::park_timeout(remaining.min(Duration::from_millis(1)));
                }
            }
        };
        let clock = store.set_time(self.started.elapsed());
        let drained = store.shutdown(timeout.saturating_sub(started.elapsed()));
        clock.and(sink).and(drained)
    }
}
fn identity_bytes() -> [u8; 16] {
    let mut bytes = [0; 16];
    if getrandom::getrandom(&mut bytes).is_err() {
        return [0; 16];
    }
    bytes
}

#[derive(Clone)]
pub(super) struct TelemetryDiagnostics {
    store: Weak<Mutex<df_telemetry::Store>>,
    observer: Arc<SharedImageObserver>,
    sink: Arc<NativeDiagnosticSink>,
    started: Instant,
}
impl TelemetryDiagnostics {
    /// The newest enabled image events, oldest first; a safe in-memory view, never a store read.
    pub(super) fn recent(&self, limit: usize) -> Vec<df_imagery::telemetry::RecentImageEvent> {
        self.observer.recent(limit)
    }
    pub(super) fn health(
        &self,
    ) -> Result<
        Option<(
            ObserverHealth,
            df_observe::DiagnosticHealth,
            df_telemetry::IngestionHealth,
        )>,
        TelemetryError,
    > {
        let Some(store) = self.store.upgrade() else {
            return Ok(None);
        };
        let store = store.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => TelemetryError::Capacity,
            std::sync::TryLockError::Poisoned(_) => TelemetryError::Internal,
        })?;
        store.set_time(self.started.elapsed())?;
        self.sink.poll_receipts();
        Ok(Some((
            self.observer.health(),
            self.sink.health(),
            store.health(),
        )))
    }
}

/// Advances the existing Store clock at admission without disk work or waiting on health/shutdown.
struct TimedDiagnosticSink {
    clock: df_telemetry::StoreClock,
    sink: Arc<NativeDiagnosticSink>,
    started: Instant,
}
impl DiagnosticSink for TimedDiagnosticSink {
    fn emit(&self, record: DiagnosticRecord) -> Result<(), TelemetryError> {
        self.clock.set_time(self.started.elapsed())?;
        self.sink.emit(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use df_imagery::telemetry::{EventContext, EventKind, ImageEvent, ImageObserver};

    fn opened() -> (ImageTelemetry, std::path::PathBuf) {
        let mut nonce = [0; 16];
        getrandom::getrandom(&mut nonce).unwrap();
        let root = std::env::temp_dir().join(format!(
            "df-mounted-image-telemetry-{}",
            super::super::journey::hex(&nonce),
        ));
        let telemetry = ImageTelemetry::open(
            &root,
            TelemetrySettings {
                limits: TelemetryLimits::default(),
                maximum_pending: 256,
                level: ImageLogLevel::Debug,
            },
        )
        .unwrap();
        (telemetry, root)
    }

    #[test]
    fn concurrent_enabled_logs_do_not_wait_on_or_drop_for_the_health_store_mutex() {
        let (mut telemetry, root) = opened();
        let store = telemetry.store.clone();
        let held = store.lock().unwrap();
        std::thread::scope(|scope| {
            for _ in 0..64 {
                let observer = telemetry.observer();
                scope.spawn(move || {
                    observer.observe(ImageEvent::new(
                        EventKind::Requested,
                        EventContext::default(),
                    ))
                });
            }
        });
        assert_eq!(telemetry.observer.health().admitted, 64);
        assert_eq!(telemetry.observer.health().failures, 0);
        drop(held);
        telemetry.shutdown(Duration::from_secs(5)).unwrap();
        assert_eq!(telemetry.sink.health().confirmed, 64);
        drop(telemetry);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shutdown_store_lock_contention_obeys_its_zero_remaining_budget_and_can_be_retried() {
        let (mut telemetry, root) = opened();
        let store = telemetry.store.clone();
        let held = store.lock().unwrap();
        let mut telemetry = std::thread::scope(|scope| {
            scope
                .spawn(move || {
                    let result = telemetry.shutdown(Duration::ZERO);
                    assert_eq!(result, Err(TelemetryError::Deadline));
                    telemetry
                })
                .join()
                .unwrap()
        });
        drop(held);
        telemetry.shutdown(Duration::from_secs(5)).unwrap();
        drop(telemetry);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }
}
