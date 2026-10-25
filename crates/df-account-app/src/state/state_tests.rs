//! Flow tests: each drives `Model::update` with messages and replies and checks the effects. No
//! DOM, no clock, no network: the runtime's job is only to perform the effects.

use super::*;
use crate::{
    api::{ApiCall, Reply},
    wire::{AccountView, OperationView, OperatorCapability, OperatorView, SessionView},
};

mod admin;
mod auth;
mod campaign;
mod customer;
mod display;

pub(super) fn anonymous() -> SessionView {
    SessionView {
        csrf: "csrf-anonymous".to_owned(),
        account: None,
        dev_sign_in: None,
    }
}

pub(super) fn signed_in(is_verified: bool) -> SessionView {
    SessionView {
        csrf: "csrf-member".to_owned(),
        account: Some(AccountView {
            email: "ana@example.test".to_owned(),
            display_name: Some("Ana".to_owned()),
            is_verified,
            operator: None,
        }),
        dev_sign_in: None,
    }
}

pub(super) fn operator(has_second_factor: bool) -> SessionView {
    let mut session = signed_in(true);
    if let Some(account) = session.account.as_mut() {
        account.operator = Some(OperatorView {
            capabilities: vec![
                OperatorCapability::SupportInspect,
                OperatorCapability::Refund,
                OperatorCapability::GoodwillCredit,
                OperatorCapability::DisputeResponse,
                OperatorCapability::ProviderControl,
                OperatorCapability::ContentReview,
            ],
            has_second_factor,
        });
    }
    session
}

/// A page that finished loading its session and entered `route`.
pub(super) fn booted(route: Route, session: SessionView) -> Model {
    let mut model = Model::default();
    model.update(Msg::Started {
        route,
        offset_east_minutes: 0,
    });
    deliver(&mut model, Reply::Session(Ok(session)));
    model
}

/// Delivers a reply under the model's current generation, as the runtime does.
pub(super) fn deliver(model: &mut Model, reply: Reply) -> Vec<Effect> {
    let generation = model.generation;
    model.update(Msg::Replied { generation, reply })
}

pub(super) fn issue(model: &mut Model, slot: OperationSlot, operation: &str) {
    let generation = model.generation;
    model.update(Msg::OperationIssued {
        generation,
        slot,
        result: Ok(OperationView {
            operation: operation.to_owned(),
        }),
    });
}

pub(super) fn click(model: &mut Model, intent: Intent) -> Vec<Effect> {
    model.update(Msg::Intent(intent))
}

pub(super) fn submit(model: &mut Model, form: Form) -> Vec<Effect> {
    click(model, Intent::Submit(form))
}

pub(super) fn type_into(model: &mut Model, field: Field, text: &str) {
    model.update(Msg::Edit(field, text.to_owned()));
}

pub(super) fn tick(model: &mut Model, field: Field, is_on: bool) {
    model.update(Msg::Toggle(field, is_on));
}

pub(super) fn calls(effects: &[Effect]) -> Vec<&ApiCall> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Api { call, .. } => Some(call),
            _ => None,
        })
        .collect()
}

pub(super) fn first_call(effects: &[Effect]) -> Option<&ApiCall> {
    calls(effects).into_iter().next()
}

pub(super) fn has_effect(effects: &[Effect], wanted: &Effect) -> bool {
    effects.contains(wanted)
}

#[test]
fn a_page_starts_by_asking_for_the_session_and_nothing_else() {
    let mut model = Model::default();

    let effects = model.update(Msg::Started {
        route: Route::Dashboard,
        offset_east_minutes: -240,
    });

    assert_eq!(
        effects,
        vec![Effect::Api {
            generation: 0,
            call: ApiCall::Session
        }]
    );
    assert_eq!(model.screen, Screen::Booting);
    assert_eq!(model.offset_east_minutes, -240);
}

#[test]
fn a_session_that_cannot_be_read_shows_a_retryable_failure_not_a_blank_page() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::Landing,
        offset_east_minutes: 0,
    });

    deliver(
        &mut model,
        Reply::Session(Err(ApiError::of(ErrorCode::Unavailable))),
    );

    assert!(matches!(model.screen, Screen::BootFailed(_)));
    let effects = click(&mut model, Intent::Retry);
    assert_eq!(first_call(&effects), Some(&ApiCall::Session));
    assert_eq!(model.screen, Screen::Booting);
}

#[test]
fn a_reply_for_a_screen_the_person_already_left_changes_nothing() {
    let mut model = booted(Route::Statements, signed_in(true));
    let stale_generation = model.generation;
    click(&mut model, Intent::Go(Route::Settings));
    let before = model.clone();

    model.update(Msg::Replied {
        generation: stale_generation,
        reply: Reply::Statements {
            is_continuation: false,
            result: Err(ApiError::of(ErrorCode::Unavailable)),
        },
    });

    assert_eq!(model, before);
}

#[test]
fn a_timer_from_an_earlier_screen_is_ignored() {
    let mut model = booted(Route::Dashboard, signed_in(true));
    let stale_generation = model.generation;
    click(&mut model, Intent::Go(Route::Statements));

    let effects = model.update(Msg::TimerFired {
        generation: stale_generation,
    });

    assert!(effects.is_empty());
}

#[test]
fn navigating_aborts_requests_in_flight_and_moves_the_address() {
    let mut model = booted(Route::Dashboard, signed_in(true));

    let effects = click(&mut model, Intent::Go(Route::Settings));

    assert_eq!(effects.first(), Some(&Effect::CancelPending));
    assert!(has_effect(
        &effects,
        &Effect::PushUrl("/app/settings".to_owned())
    ));
    assert!(has_effect(&effects, &Effect::Focus(FocusTarget::Heading)));
}

#[test]
fn the_browsers_clock_is_kept_for_the_views() {
    let mut model = Model::default();

    let effects = model.update(Msg::Clock { now_ms: 1_800_000 });

    assert!(effects.is_empty());
    assert_eq!(model.now_ms, 1_800_000);
}

#[test]
fn every_field_name_parses_back_to_its_field() {
    for field in FIELDS {
        assert_eq!(Field::parse(field.name()), Some(field), "{}", field.name());
    }
    assert_eq!(Field::parse("not-a-field"), None);
}

#[test]
fn field_names_are_unique_so_the_dom_can_identify_each_field() {
    let mut names: Vec<_> = FIELDS.iter().map(|field| field.name()).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count);
}
