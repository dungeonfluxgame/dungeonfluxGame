//! Content-free diagnostics carried by the existing authenticated PHONE timing envelope.
//! `usable` is the existing page lifecycle kind. Its numeric id has a version marker,
//! closed event code, attempt and milliseconds; scene is only an operation hash.
use df_observe::presentation::TimingEvent;

const MARKER: u64 = 0x4800_0000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum Diagnostic {
    PortraitRequestStart = 1,
    PortraitPollPending = 2,
    PortraitPollReady = 3,
    PortraitPollFailed = 4,
    PortraitPollUnknown = 5,
    PortraitApplied = 6,
    PortraitDisplayed = 7,
    PortraitImageFailed = 8,
    PortraitIgnoredRevision = 9,
    PortraitStoppedScopeChanged = 10,
    PortraitStoppedCreatorClosed = 11,
    PortraitStoppedInvalidRequest = 12,
    PortraitStoppedRpcFailed = 13,
    PortraitStoppedTerminal = 14,
    PortraitStoppedExhausted = 15,
    PortraitPageHidden = 16,
    PortraitPageVisible = 17,
    PortraitTimerThrottled = 18,
    ReconnectScheduled = 19,
    ReconnectAttempt = 20,
    ReconnectReady = 21,
    ReconnectFailed = 22,
    ReconnectCancelled = 23,
    PortraitImageLoaded = 24,
    JoinStarted = 25,
    JoinFailed = 26,
    JoinRefused = 27,
    JoinRetry = 28,
    CurrentAdventureFailed = 29,
    WatchRefused = 30,
    StreamInterrupted = 31,
    SavedSeatForgotten = 32,
    ViewRenderFailed = 33,
    SubmitSent = 34,
    SubmitAnswered = 35,
    SubmitFailed = 36,
    LocalStorageFailed = 37,
    LibraryFailed = 38,
    TelemetryUploadFailed = 39,
    TelemetryReceiptMismatch = 40,
    TelemetryCaptureFailed = 41,
    PageStarted = 42,
}

impl Diagnostic {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::PortraitRequestStart => "portrait-request-start",
            Self::PortraitPollPending => "portrait-poll-pending",
            Self::PortraitPollReady => "portrait-poll-ready",
            Self::PortraitPollFailed => "portrait-poll-failed",
            Self::PortraitPollUnknown => "portrait-poll-unknown",
            Self::PortraitApplied => "portrait-applied",
            Self::PortraitDisplayed => "portrait-displayed",
            Self::PortraitImageFailed => "portrait-image-failed",
            Self::PortraitIgnoredRevision => "portrait-ignored-revision",
            Self::PortraitStoppedScopeChanged => "portrait-stopped-scope-changed",
            Self::PortraitStoppedCreatorClosed => "portrait-stopped-creator-closed",
            Self::PortraitStoppedInvalidRequest => "portrait-stopped-invalid-request",
            Self::PortraitStoppedRpcFailed => "portrait-stopped-rpc-failed",
            Self::PortraitStoppedTerminal => "portrait-stopped-terminal",
            Self::PortraitStoppedExhausted => "portrait-stopped-exhausted",
            Self::PortraitPageHidden => "portrait-page-hidden",
            Self::PortraitPageVisible => "portrait-page-visible",
            Self::PortraitTimerThrottled => "portrait-timer-throttled",
            Self::ReconnectScheduled => "reconnect-scheduled",
            Self::ReconnectAttempt => "reconnect-attempt",
            Self::ReconnectReady => "reconnect-ready",
            Self::ReconnectFailed => "reconnect-failed",
            Self::ReconnectCancelled => "reconnect-cancelled",
            Self::PortraitImageLoaded => "portrait-image-loaded",
            Self::JoinStarted => "join-started",
            Self::JoinFailed => "join-failed",
            Self::JoinRefused => "join-refused",
            Self::JoinRetry => "join-retry",
            Self::CurrentAdventureFailed => "current-adventure-failed",
            Self::WatchRefused => "watch-refused",
            Self::StreamInterrupted => "stream-interrupted",
            Self::SavedSeatForgotten => "saved-seat-forgotten",
            Self::ViewRenderFailed => "view-render-failed",
            Self::SubmitSent => "submit-sent",
            Self::SubmitAnswered => "submit-answered",
            Self::SubmitFailed => "submit-failed",
            Self::LocalStorageFailed => "local-storage-failed",
            Self::LibraryFailed => "library-failed",
            Self::TelemetryUploadFailed => "telemetry-upload-failed",
            Self::TelemetryReceiptMismatch => "telemetry-receipt-mismatch",
            Self::TelemetryCaptureFailed => "telemetry-capture-failed",
            Self::PageStarted => "page-started",
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    pub(crate) fn event(
        self,
        operation_hash: u64,
        attempt: u32,
        delay_ms: u32,
        at_ms: u64,
    ) -> TimingEvent {
        let reason = match self {
            Self::PortraitRequestStart => "changed",
            Self::PortraitPollPending => "changed",
            Self::PortraitPollReady => "ready",
            Self::PortraitPollFailed => "failed",
            Self::PortraitPollUnknown => "blocked",
            Self::PortraitApplied => "ready",
            Self::PortraitDisplayed => "ready",
            Self::PortraitImageFailed => "failed",
            Self::PortraitIgnoredRevision => "replaced",
            Self::PortraitStoppedScopeChanged => "scene-change",
            Self::PortraitStoppedCreatorClosed => "left",
            Self::PortraitStoppedInvalidRequest => "failed",
            Self::PortraitStoppedRpcFailed => "failed",
            Self::PortraitStoppedTerminal => "ended",
            Self::PortraitStoppedExhausted => "timeout",
            Self::PortraitPageHidden => "visibility-lost",
            Self::PortraitPageVisible => "appeared",
            Self::PortraitTimerThrottled => "timeout",
            Self::ReconnectScheduled => "changed",
            Self::ReconnectAttempt => "changed",
            Self::ReconnectReady => "ready",
            Self::ReconnectFailed => "failed",
            Self::ReconnectCancelled => "replaced",
            Self::PortraitImageLoaded => "ready",
            Self::JoinStarted => "changed",
            Self::JoinFailed => "failed",
            Self::JoinRefused => "failed",
            Self::JoinRetry => "changed",
            Self::CurrentAdventureFailed => "failed",
            Self::WatchRefused => "failed",
            Self::StreamInterrupted => "failed",
            Self::SavedSeatForgotten => "failed",
            Self::ViewRenderFailed => "failed",
            Self::SubmitSent => "changed",
            Self::SubmitAnswered => "ready",
            Self::SubmitFailed => "failed",
            Self::LocalStorageFailed => "failed",
            Self::LibraryFailed => "failed",
            Self::TelemetryUploadFailed => "failed",
            Self::TelemetryReceiptMismatch => "failed",
            Self::TelemetryCaptureFailed => "failed",
            Self::PageStarted => "changed",
        };
        TimingEvent::lifecycle(
            "usable",
            reason,
            MARKER
                | (u64::from(delay_ms) << 24)
                | (u64::from(attempt.min(u16::MAX.into())) << 8)
                | self as u64,
            operation_hash,
            at_ms,
        )
    }
}

pub(crate) use df_observe::operation_hash;

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn decoded(event: &TimingEvent) -> Option<(Diagnostic, u32, u32)> {
    if event.kind != "usable" || event.id & 0xff00_0000_0000_0000 != MARKER {
        return None;
    }
    let diagnostic = match event.id & 255 {
        1 => Diagnostic::PortraitRequestStart,
        2 => Diagnostic::PortraitPollPending,
        3 => Diagnostic::PortraitPollReady,
        4 => Diagnostic::PortraitPollFailed,
        5 => Diagnostic::PortraitPollUnknown,
        6 => Diagnostic::PortraitApplied,
        7 => Diagnostic::PortraitDisplayed,
        8 => Diagnostic::PortraitImageFailed,
        9 => Diagnostic::PortraitIgnoredRevision,
        10 => Diagnostic::PortraitStoppedScopeChanged,
        11 => Diagnostic::PortraitStoppedCreatorClosed,
        12 => Diagnostic::PortraitStoppedInvalidRequest,
        13 => Diagnostic::PortraitStoppedRpcFailed,
        14 => Diagnostic::PortraitStoppedTerminal,
        15 => Diagnostic::PortraitStoppedExhausted,
        16 => Diagnostic::PortraitPageHidden,
        17 => Diagnostic::PortraitPageVisible,
        18 => Diagnostic::PortraitTimerThrottled,
        19 => Diagnostic::ReconnectScheduled,
        20 => Diagnostic::ReconnectAttempt,
        21 => Diagnostic::ReconnectReady,
        22 => Diagnostic::ReconnectFailed,
        23 => Diagnostic::ReconnectCancelled,
        24 => Diagnostic::PortraitImageLoaded,
        25 => Diagnostic::JoinStarted,
        26 => Diagnostic::JoinFailed,
        27 => Diagnostic::JoinRefused,
        28 => Diagnostic::JoinRetry,
        29 => Diagnostic::CurrentAdventureFailed,
        30 => Diagnostic::WatchRefused,
        31 => Diagnostic::StreamInterrupted,
        32 => Diagnostic::SavedSeatForgotten,
        33 => Diagnostic::ViewRenderFailed,
        34 => Diagnostic::SubmitSent,
        35 => Diagnostic::SubmitAnswered,
        36 => Diagnostic::SubmitFailed,
        37 => Diagnostic::LocalStorageFailed,
        38 => Diagnostic::LibraryFailed,
        39 => Diagnostic::TelemetryUploadFailed,
        40 => Diagnostic::TelemetryReceiptMismatch,
        41 => Diagnostic::TelemetryCaptureFailed,
        42 => Diagnostic::PageStarted,

        _ => return None,
    };
    Some((
        diagnostic,
        ((event.id >> 8) & 0xffff) as u32,
        ((event.id >> 24) & 0xffff_ffff) as u32,
    ))
}

#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn log_suffix(event: &TimingEvent) -> String {
    decoded(event).map_or_else(String::new, |(diagnostic, attempt, delay_ms)| {
        format!(
            " diagnostic={} operation_hash={} attempt={attempt} delay_ms={delay_ms} op={}{}{}",
            diagnostic.label(),
            event.scene,
            event.scene,
            match diagnostic {
                Diagnostic::JoinFailed
                | Diagnostic::JoinRefused
                | Diagnostic::CurrentAdventureFailed
                | Diagnostic::WatchRefused
                | Diagnostic::StreamInterrupted
                | Diagnostic::SubmitFailed
                | Diagnostic::SubmitAnswered
                | Diagnostic::TelemetryUploadFailed =>
                    format!(" code={attempt} elapsed_ms={delay_ms}"),
                Diagnostic::LibraryFailed => format!(
                    " code={attempt} library_operation={}",
                    match delay_ms {
                        1 => "list",
                        2 => "load",
                        3 => "delete",
                        4 => "save",
                        _ => "unknown",
                    }
                ),
                _ => String::new(),
            },
            if diagnostic == Diagnostic::PortraitDisplayed {
                " evidence=visible-paint-opportunity"
            } else {
                ""
            }
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_and_phone_operation_bridge_use_the_same_raw_id_contract() {
        assert_eq!(
            operation_hash(&[1; 16]),
            df_observe::presentation::local_identity("01010101010101010101010101010101")
        );
        assert_ne!(operation_hash(&[1; 16]), operation_hash(&[2; 16]));
    }

    #[test]
    fn every_client_diagnostic_survives_shared_record_encoding_without_text() {
        for diagnostic in [
            Diagnostic::PortraitRequestStart,
            Diagnostic::PortraitPollPending,
            Diagnostic::PortraitPollReady,
            Diagnostic::PortraitPollFailed,
            Diagnostic::PortraitPollUnknown,
            Diagnostic::PortraitApplied,
            Diagnostic::PortraitDisplayed,
            Diagnostic::PortraitImageFailed,
            Diagnostic::PortraitIgnoredRevision,
            Diagnostic::PortraitStoppedScopeChanged,
            Diagnostic::PortraitStoppedCreatorClosed,
            Diagnostic::PortraitStoppedInvalidRequest,
            Diagnostic::PortraitStoppedRpcFailed,
            Diagnostic::PortraitStoppedTerminal,
            Diagnostic::PortraitStoppedExhausted,
            Diagnostic::PortraitPageHidden,
            Diagnostic::PortraitPageVisible,
            Diagnostic::PortraitTimerThrottled,
            Diagnostic::ReconnectScheduled,
            Diagnostic::ReconnectAttempt,
            Diagnostic::ReconnectReady,
            Diagnostic::ReconnectFailed,
            Diagnostic::ReconnectCancelled,
            Diagnostic::PortraitImageLoaded,
            Diagnostic::JoinStarted,
            Diagnostic::JoinFailed,
            Diagnostic::JoinRefused,
            Diagnostic::JoinRetry,
            Diagnostic::CurrentAdventureFailed,
            Diagnostic::WatchRefused,
            Diagnostic::StreamInterrupted,
            Diagnostic::SavedSeatForgotten,
            Diagnostic::ViewRenderFailed,
            Diagnostic::SubmitSent,
            Diagnostic::SubmitAnswered,
            Diagnostic::SubmitFailed,
            Diagnostic::LocalStorageFailed,
            Diagnostic::LibraryFailed,
            Diagnostic::TelemetryUploadFailed,
            Diagnostic::TelemetryReceiptMismatch,
            Diagnostic::TelemetryCaptureFailed,
            Diagnostic::PageStarted,
        ] {
            let event = diagnostic.event(12345, 240, 30_000, 123456);
            let bytes = df_observe::presentation::encode(std::slice::from_ref(&event)).unwrap();
            let decoded = df_observe::presentation::decode(&bytes).unwrap();
            assert_eq!(decoded, [event]);
            assert!(log_suffix(&decoded[0]).contains(diagnostic.label()));
            assert!(log_suffix(&decoded[0]).contains("attempt=240 delay_ms=30000"));
        }
    }
}

/// Page-start diagnostics retain the earliest failures until authenticated upload exists.
/// New records beyond the bound are counted explicitly in the first connected batch.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Default)]
pub(crate) struct PreconnectBuffer {
    events: std::collections::VecDeque<TimingEvent>,
    dropped: u64,
}
#[cfg(any(target_arch = "wasm32", test))]
impl PreconnectBuffer {
    pub(crate) fn push(&mut self, event: TimingEvent) {
        if self.events.len() == 128 {
            self.dropped = self.dropped.saturating_add(1);
        } else {
            self.events.push_back(event);
        }
    }
    pub(crate) fn drain(&mut self) -> (Vec<TimingEvent>, u64) {
        (
            self.events.drain(..).collect(),
            std::mem::take(&mut self.dropped),
        )
    }
}
#[cfg(test)]
mod preconnect_tests {
    use super::*;
    #[test]
    fn page_start_failures_flush_once_with_explicit_overflow() {
        let mut buffer = PreconnectBuffer::default();
        for attempt in 0..130 {
            buffer.push(Diagnostic::JoinFailed.event(71, attempt, 0, attempt.into()));
        }
        let (events, dropped) = buffer.drain();
        assert_eq!(events.len(), 128);
        assert_eq!(dropped, 2);
        assert_eq!(decoded(&events[0]), Some((Diagnostic::JoinFailed, 0, 0)));
        assert_eq!(
            decoded(&events[127]),
            Some((Diagnostic::JoinFailed, 127, 0))
        );
        assert_eq!(buffer.drain(), (Vec::new(), 0));
    }
}
