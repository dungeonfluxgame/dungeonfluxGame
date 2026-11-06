//! The engine's typed refusals: every closed key is its serde name, and a refusal's status says
//! whose fault it is.

use axum::http::StatusCode;
use df_table::config::SettingCause;
use df_table::event::FailureCause;
use df_table::outcome::DeclineCode;
use df_table::owner_queue::NotDeclinedCause;
use df_table::ports::PortName;

use crate::dev_http::{BodyCause, BodyError, RouteError};

use super::commit::CommitError;
use super::report::{
    CommitDisposition, DevControl, EngineError, ForceCause, QueryError, RefusalCause,
    UnreadableBodyCause, ViewCause, WireIntentCause,
};
use super::setup::StartCause;

/// The text serde writes for a unit variant of a `snake_case` enum.
fn serde_name(value: &impl serde::Serialize) -> String {
    match serde_json::to_value(value).expect("a closed key serializes") {
        serde_json::Value::String(name) => name,
        other => panic!("a closed key is a string, got {other}"),
    }
}

#[test]
fn every_closed_key_of_the_engine_routes_is_its_serde_name_and_reads_back() {
    for cause in ForceCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(ForceCause::from_key(cause.key()), Some(cause));
    }
    for cause in ViewCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(ViewCause::from_key(cause.key()), Some(cause));
    }
    for cause in WireIntentCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(WireIntentCause::from_key(cause.key()), Some(cause));
    }
    for control in DevControl::ALL {
        assert_eq!(control.key(), serde_name(&control));
        assert_eq!(DevControl::from_key(control.key()), Some(control));
    }
    for cause in StartCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(StartCause::from_key(cause.key()), Some(cause));
    }
    for cause in UnreadableBodyCause::ALL {
        assert_eq!(cause.key(), serde_name(&cause));
        assert_eq!(UnreadableBodyCause::from_key(cause.key()), Some(cause));
    }
}

/// One of every variant, so a variant added without a key or a status is caught.
fn every_error() -> Vec<EngineError> {
    vec![
        EngineError::MailboxFull,
        EngineError::Stopped,
        EngineError::NoAnswer,
        EngineError::OwnerPanicked,
        EngineError::NotStarted {
            cause: StartCause::Thread,
            detail: "no thread".to_owned(),
        },
        EngineError::InvalidBody(BodyError {
            cause: BodyCause::Syntax,
            line: 1,
            column: 1,
            detail: "expected value".to_owned(),
        }),
        EngineError::InvalidQuery(QueryError {
            detail: "missing field `seat`".to_owned(),
        }),
        EngineError::UnreadableBody {
            cause: UnreadableBodyCause::TooLarge,
        },
        EngineError::UnreadableBody {
            cause: UnreadableBodyCause::Interrupted,
        },
        EngineError::MachineNotOnTable {
            machine: "dialogue".to_owned(),
        },
        EngineError::UnknownReport {
            report: "everything".to_owned(),
        },
        EngineError::ZeroOperation,
        EngineError::ZeroStep,
        EngineError::InvalidName {
            what: "Bad Name".to_owned(),
        },
        EngineError::UnknownSeat { seat: 9 },
        EngineError::UnknownMachine {
            machine: "nobody".to_owned(),
        },
        EngineError::UnknownState {
            machine: "lobby".to_owned(),
            state: "elsewhere".to_owned(),
        },
        EngineError::UnknownPurpose {
            purpose: "plate".to_owned(),
        },
        EngineError::PayloadUnreadable {
            machine: "lobby".to_owned(),
            detail: "no phase".to_owned(),
        },
        EngineError::ForcedStateMismatch {
            machine: "lobby".to_owned(),
            named: "started".to_owned(),
            reached: "gathering".to_owned(),
        },
        EngineError::ForceRefused {
            cause: ForceCause::SeatLane,
            detail: None,
        },
        EngineError::Port {
            port: PortName::Broker,
            cause: FailureCause::Stale,
        },
        EngineError::NotAvailable {
            control: DevControl::Clock,
        },
        EngineError::UnknownSetting {
            name: "nope".to_owned(),
        },
        EngineError::SettingRefused {
            name: "held_nudge_s".to_owned(),
            cause: SettingCause::OutOfRange,
        },
        EngineError::SettingSpecInvalid {
            name: "held_nudge_s".to_owned(),
        },
        EngineError::SettingNeedsRestart {
            name: "seat_pending_limit".to_owned(),
        },
        EngineError::NotSupportedYet {
            what: "replay".to_owned(),
            needs: "C7".to_owned(),
        },
        EngineError::UnknownLog {
            log: "session".to_owned(),
        },
        EngineError::BlankLog,
        EngineError::TooFast {
            retry_after_ms: Some(5),
        },
        EngineError::Stalled,
        EngineError::NoMachineRunning,
        EngineError::ViewUnavailable {
            cause: ViewCause::WireCheck,
            detail: "blank readout".to_owned(),
        },
        EngineError::WireIntent {
            cause: WireIntentCause::MissingDeclineCode,
            detail: None,
        },
        EngineError::NotServedYet,
    ]
}

#[test]
fn a_refusal_cause_is_written_as_the_bare_key_of_its_code_or_cause_and_reads_back() {
    let causes = DeclineCode::ALL
        .iter()
        .copied()
        .map(RefusalCause::Declined)
        .chain(
            NotDeclinedCause::ALL
                .iter()
                .copied()
                .map(RefusalCause::NotDeclined),
        );
    for cause in causes {
        assert_eq!(cause.key(), serde_name(&cause));
        let read: RefusalCause =
            serde_json::from_value(serde_json::Value::from(cause.key())).expect("reads back");
        assert_eq!(read, cause);
    }
}

#[test]
fn a_failed_commit_is_written_as_its_cause_beside_the_tag_as_before() {
    let failed = CommitDisposition::Failed(CommitError::Unavailable);
    assert_eq!(
        serde_json::to_value(failed).expect("writes"),
        serde_json::json!({ "commit": "failed", "cause": "unavailable" })
    );
    let late = CommitDisposition::Failed(CommitError::DeadlineExceeded { deadline_ms: 50 });
    let written = serde_json::to_value(&late).expect("writes");
    assert_eq!(
        written,
        serde_json::json!({ "commit": "failed", "cause": "deadline_exceeded", "deadline_ms": 50 })
    );
    let read: CommitDisposition = serde_json::from_value(written).expect("reads back");
    assert_eq!(read, late);
}

#[test]
fn every_engine_error_key_is_its_serde_tag_and_it_reads_back_whole() {
    for error in every_error() {
        let written = serde_json::to_value(&error).expect("an error serializes");
        assert_eq!(written["error"], error.key(), "{written}");
        let read: EngineError = serde_json::from_value(written).expect("an error reads back");
        assert_eq!(read, error);
    }
}

#[test]
fn every_engine_error_says_a_sentence() {
    for error in every_error() {
        let sentence = error.to_string();
        assert!(!sentence.is_empty(), "{error:?}");
        assert!(
            sentence.starts_with(char::is_lowercase) || sentence.starts_with('\''),
            "{sentence}"
        );
    }
}

#[test]
fn a_panicked_owner_is_a_service_fault_not_a_mistake_of_the_caller() {
    // It was answered 400 Bad Request, which told the caller to change its request.
    assert_eq!(
        EngineError::OwnerPanicked.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        EngineError::Stopped.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[test]
fn a_forced_table_whose_commit_failed_is_a_service_fault_and_the_other_causes_are_the_callers() {
    // A failed commit (the store unavailable or refusing) was answered 400 Bad Request, which
    // blamed the caller for a fault of the host.
    for cause in ForceCause::ALL {
        let expected = match cause {
            ForceCause::CommitFailed => StatusCode::SERVICE_UNAVAILABLE,
            ForceCause::EntryRefused
            | ForceCause::NoActiveScene
            | ForceCause::SeatLane
            | ForceCause::StartRefused => StatusCode::BAD_REQUEST,
        };
        let refusal = EngineError::ForceRefused {
            cause,
            detail: None,
        };
        assert_eq!(refusal.status(), expected, "{}", cause.key());
    }
}

#[test]
fn a_refusal_status_names_whose_fault_it_is() {
    assert_eq!(EngineError::ZeroOperation.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        EngineError::TooFast {
            retry_after_ms: None
        }
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        EngineError::NotSupportedYet {
            what: "replay".to_owned(),
            needs: "C7".to_owned()
        }
        .status(),
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(
        EngineError::NoMachineRunning.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        EngineError::WireIntent {
            cause: WireIntentCause::RefusedAtTheEdge,
            detail: None
        }
        .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        EngineError::WireIntent {
            cause: WireIntentCause::ReceiptInvalid,
            detail: None
        }
        .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn a_body_over_the_limit_is_413_one_that_broke_off_400_and_a_report_name_the_callers_mistake() {
    let too_large = EngineError::UnreadableBody {
        cause: UnreadableBodyCause::TooLarge,
    };
    let broke_off = EngineError::UnreadableBody {
        cause: UnreadableBodyCause::Interrupted,
    };

    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(broke_off.status(), StatusCode::BAD_REQUEST);
    assert_eq!(too_large.key(), "unreadable_body");
    let written = serde_json::to_value(&too_large).expect("serializes");
    assert_eq!(
        written,
        serde_json::json!({"error": "unreadable_body", "cause": "too_large"})
    );
    for mistake in [
        EngineError::UnknownReport {
            report: "x".to_owned(),
        },
        EngineError::MachineNotOnTable {
            machine: "explore".to_owned(),
        },
    ] {
        assert_eq!(mistake.status(), StatusCode::BAD_REQUEST, "{mistake:?}");
    }
}

#[test]
fn an_owner_that_did_not_start_names_its_cause_in_its_sentence() {
    let error = EngineError::NotStarted {
        cause: StartCause::Spool,
        detail: "the spool cannot be opened".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "the owner did not start (spool): the spool cannot be opened"
    );
    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
}
