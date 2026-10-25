use super::*;
use crate::wire_campaign::{
    DisplayChallenge, DisplayRedeemed, DisplayState, DisplayStatus, Reason,
};

fn showing_model() -> Model {
    let mut model = booted(Route::Display, anonymous());
    click(&mut model, Intent::ShowPairingCode);
    deliver(&mut model, Reply::Session(Ok(anonymous())));
    deliver(
        &mut model,
        Reply::DisplayChallenge(Ok(DisplayChallenge {
            user_code: "ABCD-EFGH".to_owned(),
            expires_at_ms: 600_000,
            poll_seconds: 2,
        })),
    );
    model
}

fn status(state: DisplayState) -> Reply {
    Reply::DisplayStatus(Ok(DisplayStatus { state }))
}

fn phase(model: &Model) -> DisplayPhase {
    let Screen::Display(screen) = &model.screen else {
        panic!("the display screen");
    };
    screen.phase.clone()
}

#[test]
fn the_display_page_is_open_to_everyone_and_asks_for_nothing_until_pressed() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::Display,
        offset_east_minutes: 0,
    });

    let effects = deliver(&mut model, Reply::Session(Ok(anonymous())));

    assert!(calls(&effects).is_empty());
    assert_eq!(phase(&model), DisplayPhase::Idle);
}

#[test]
fn a_signed_in_person_keeps_the_display_page_too() {
    let model = booted(Route::Display, signed_in(true));

    assert!(matches!(model.screen, Screen::Display(_)));
}

#[test]
fn showing_a_code_first_reads_a_fresh_session_token_then_asks_for_the_code() {
    let mut model = booted(Route::Display, anonymous());

    let pressed = click(&mut model, Intent::ShowPairingCode);
    let refreshed = deliver(&mut model, Reply::Session(Ok(anonymous())));

    assert_eq!(first_call(&pressed), Some(&ApiCall::Session));
    assert_eq!(phase(&model), DisplayPhase::Requesting);
    assert_eq!(first_call(&refreshed), Some(&ApiCall::DisplayChallenge));
}

#[test]
fn a_session_that_cannot_be_refreshed_ends_the_attempt_with_a_message() {
    let mut model = booted(Route::Display, anonymous());
    click(&mut model, Intent::ShowPairingCode);

    deliver(
        &mut model,
        Reply::Session(Err(ApiError::of(ErrorCode::Unavailable))),
    );

    assert!(matches!(
        phase(&model),
        DisplayPhase::Ended(DisplayEnd::Refused(_))
    ));
}

#[test]
fn the_code_is_polled_at_the_server_interval_until_the_host_approves_it() {
    let mut model = showing_model();

    let first = model.update(Msg::TimerFired {
        generation: model.generation,
    });
    let pending = deliver(&mut model, status(DisplayState::Pending));

    assert_eq!(first_call(&first), Some(&ApiCall::DisplayStatus));
    assert!(pending.iter().any(|effect| matches!(
        effect,
        Effect::After {
            delay_ms: 2_000,
            ..
        }
    )));
}

#[test]
fn an_approved_code_is_redeemed_and_the_browser_goes_to_the_game_display() {
    let mut model = showing_model();

    let redeem = deliver(&mut model, status(DisplayState::Approved));
    let open = deliver(
        &mut model,
        Reply::DisplayRedeemed(Ok(DisplayRedeemed {
            session_id: "5e".to_owned(),
            ticket_expires_at_ms: 1,
        })),
    );

    assert_eq!(first_call(&redeem), Some(&ApiCall::DisplayRedeem));
    assert_eq!(open, vec![Effect::Leave("/sessions/5e/display".to_owned())]);
    assert_eq!(phase(&model), DisplayPhase::Opening);
}

#[test]
fn a_session_id_that_is_not_ours_is_never_navigated_to() {
    let mut model = showing_model();
    deliver(&mut model, status(DisplayState::Approved));

    let effects = deliver(
        &mut model,
        Reply::DisplayRedeemed(Ok(DisplayRedeemed {
            session_id: "../../evil".to_owned(),
            ticket_expires_at_ms: 1,
        })),
    );

    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Leave(_)))
    );
    assert!(matches!(phase(&model), DisplayPhase::Ended(_)));
}

#[test]
fn a_redemption_whose_answer_was_lost_is_repeated_but_only_a_few_times() {
    let mut model = showing_model();
    deliver(&mut model, status(DisplayState::Approved));
    let lost = || Reply::DisplayRedeemed(Err(ApiError::of(ErrorCode::UnknownOutcome)));

    let mut repeats = 0;
    for _ in 0..DISPLAY_REDEEM_ATTEMPTS {
        if first_call(&deliver(&mut model, lost())) == Some(&ApiCall::DisplayRedeem) {
            repeats += 1;
        }
    }

    assert_eq!(repeats, usize::from(DISPLAY_REDEEM_ATTEMPTS) - 1);
    assert!(matches!(
        phase(&model),
        DisplayPhase::Ended(DisplayEnd::Refused(_))
    ));
}

#[test]
fn the_screen_limit_is_reported_as_a_reason_and_never_retried() {
    let mut model = showing_model();
    deliver(&mut model, status(DisplayState::Approved));
    let error = ApiError::because(ErrorCode::Forbidden, Reason::DisplayLimit);

    let effects = deliver(&mut model, Reply::DisplayRedeemed(Err(error)));

    assert!(calls(&effects).is_empty());
    assert_eq!(
        phase(&model),
        DisplayPhase::Ended(DisplayEnd::Refused(error))
    );
}

#[test]
fn an_expired_or_cancelled_code_ends_and_offers_a_new_one() {
    for (state, end) in [
        (DisplayState::Expired, DisplayEnd::Expired),
        (DisplayState::Cancelled, DisplayEnd::Cancelled),
    ] {
        let mut model = showing_model();

        deliver(&mut model, status(state));

        assert_eq!(phase(&model), DisplayPhase::Ended(end));
        let again = click(&mut model, Intent::ShowPairingCode);
        assert_eq!(first_call(&again), Some(&ApiCall::Session));
    }
}

#[test]
fn a_status_read_that_fails_briefly_keeps_polling() {
    let mut model = showing_model();

    let effects = deliver(
        &mut model,
        Reply::DisplayStatus(Err(ApiError::of(ErrorCode::Unavailable))),
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::After { .. }))
    );
    assert!(matches!(phase(&model), DisplayPhase::Showing { .. }));
}
