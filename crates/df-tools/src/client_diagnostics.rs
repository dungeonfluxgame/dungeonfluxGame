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
}

impl Diagnostic {
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

pub(crate) fn operation_hash(bytes: &[u8]) -> u64 {
    let label = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    df_observe::presentation::local_identity(&label)
}

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
        _ => return None,
    };
    Some((
        diagnostic,
        ((event.id >> 8) & 0xffff) as u32,
        ((event.id >> 24) & 0xffff_ffff) as u32,
    ))
}

pub(crate) fn log_suffix(event: &TimingEvent) -> String {
    decoded(event).map_or_else(String::new, |(diagnostic, attempt, delay_ms)| {
        format!(
            " diagnostic={} operation_hash={} attempt={attempt} delay_ms={delay_ms}{}",
            diagnostic.label(),
            event.scene,
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
        ] {
            let event = diagnostic.event(12345, 240, 30_000, 123456);
            let bytes = df_observe::presentation::encode(&[event.clone()]).unwrap();
            let decoded = df_observe::presentation::decode(&bytes).unwrap();
            assert_eq!(decoded, [event]);
            assert!(log_suffix(&decoded[0]).contains(diagnostic.label()));
            assert!(log_suffix(&decoded[0]).contains("attempt=240 delay_ms=30000"));
        }
    }
}
