use axum::http::StatusCode;

use super::request::{
    IdentityOutcome, IoCause, LocateError, QuestionCause, ScanCause, ScanPrompt, ScanSendState,
};
use crate::dev_http::{BodyCause, BodyError, RouteError};
use crate::text::TextServiceCause;

fn every_error() -> Vec<LocateError> {
    let picture = || "plate.webp".to_owned();
    vec![
        LocateError::InvalidBody(BodyError {
            cause: BodyCause::Data,
            line: 1,
            column: 2,
            detail: "missing field `picture`".to_owned(),
        }),
        LocateError::InvalidQuery {
            detail: "missing field `picture`".to_owned(),
        },
        LocateError::InvalidPictureName {
            name: "a b.png".to_owned(),
        },
        LocateError::InvalidMaskCount { masks: 0 },
        LocateError::InvalidLabel {
            label: "not ok".to_owned(),
        },
        LocateError::InvalidMinimumConfidence {
            min_confidence_permille: 1001,
        },
        LocateError::NoCharacters,
        LocateError::UnknownRegion {
            region: "bard".to_owned(),
            region_count: 2,
            names: vec!["elf".to_owned()],
        },
        LocateError::PictureMissing { picture: picture() },
        LocateError::PictureUnreadable {
            picture: picture(),
            cause: IoCause::PermissionDenied,
        },
        LocateError::NoLayout { picture: picture() },
        LocateError::LayoutUnreadable {
            picture: picture(),
            cause: IoCause::Other,
        },
        LocateError::LayoutInvalid {
            picture: picture(),
            detail: "expected value".to_owned(),
        },
        LocateError::LayoutUnwritable {
            picture: picture(),
            cause: IoCause::StorageFull,
        },
        LocateError::StaleLayout { picture: picture() },
        LocateError::LayoutChanged { picture: picture() },
        LocateError::FalKeyMissing,
        LocateError::ScanFailed {
            cause: ScanCause::Expired,
            send_state: ScanSendState::PossiblySent,
        },
        LocateError::QuestionInvalid {
            cause: QuestionCause::Request,
        },
        LocateError::TextServiceMissing {
            missing: "set the text backend".to_owned(),
        },
        LocateError::TextService {
            cause: TextServiceCause::InFlight,
            detail: None,
        },
        LocateError::InvalidOperationKey,
        LocateError::Stopped,
    ]
}

#[test]
fn every_locate_error_key_is_its_serde_tag_and_it_round_trips() {
    for error in every_error() {
        let written = serde_json::to_value(&error).expect("writes");
        assert_eq!(written["error"], error.key(), "{written}");
        let read: LocateError = serde_json::from_value(written).expect("reads");
        assert_eq!(read, error);
        assert!(!error.to_string().is_empty());
    }
}

#[test]
fn the_closed_causes_keys_are_their_serde_names() {
    fn check<T: serde::Serialize + Copy>(all: &[T], key: impl Fn(T) -> &'static str) {
        for value in all {
            let written = serde_json::to_value(value).expect("writes");
            assert_eq!(written, key(*value));
        }
    }
    check(&ScanPrompt::ALL, ScanPrompt::key);
    check(&IoCause::ALL, IoCause::key);
    check(&ScanCause::ALL, ScanCause::key);
    check(&ScanSendState::ALL, ScanSendState::key);
    check(&QuestionCause::ALL, QuestionCause::key);
    check(&TextServiceCause::ALL, TextServiceCause::key);
    check(&IdentityOutcome::ALL, IdentityOutcome::key);
    for prompt in ScanPrompt::ALL {
        assert_eq!(ScanPrompt::from_key(prompt.key()), Some(prompt));
    }
}

#[test]
fn each_locate_error_answers_the_status_of_its_kind() {
    let status = |error: LocateError| error.status();
    assert_eq!(status(LocateError::NoCharacters), StatusCode::BAD_REQUEST);
    assert_eq!(
        status(LocateError::NoLayout {
            picture: "a.png".to_owned()
        }),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(LocateError::StaleLayout {
            picture: "a.png".to_owned()
        }),
        StatusCode::CONFLICT
    );
    assert_eq!(
        status(LocateError::FalKeyMissing),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        status(LocateError::ScanFailed {
            cause: ScanCause::RateLimited,
            send_state: ScanSendState::KnownSent,
        }),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        status(LocateError::Stopped),
        StatusCode::INTERNAL_SERVER_ERROR
    );
}

#[test]
fn every_segmentation_cause_keeps_its_own_key_on_the_wire() {
    use df_imagery::oneshot_segment::SegmentCause;
    for &cause in SegmentCause::ALL {
        assert_eq!(ScanCause::from(cause).key(), cause.key());
    }
    for state in df_providers::SendState::ALL {
        assert_eq!(ScanSendState::from(state).key(), state.key());
    }
    for state in ScanSendState::ALL {
        assert_eq!(ScanSendState::from_key(state.key()), Some(state));
    }
}
