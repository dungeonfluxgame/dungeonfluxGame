use super::*;
use crate::{
    api::ProviderSwitch,
    route::AdminRoute,
    wire::{
        ActionOutcome, AdminAccountDetail, AdminAccountRow, AdminActionView, DisputeRow,
        DisputeStatus, GoodwillCreditRequest, ProviderHealth, ProviderRow, ReasonRequest,
        RefundRequest, RefundableLot, Refusal, ReviewDecision, ReviewDecisionRequest, ReviewFlag,
        ReviewItem, ReviewKind, Standing,
    },
};

fn account_row() -> AdminAccountRow {
    AdminAccountRow {
        account_id: "00ff".to_owned(),
        email_masked: "a***@example.test".to_owned(),
        display_name: Some("Ana".to_owned()),
        is_verified: true,
        standing: Standing::Active,
        balance_micro_dollars: 20_000_000,
        last_activity_at_ms: Some(1),
        lifetime_topups_micro_dollars: 20_000_000,
        lifetime_spend_micro_dollars: 0,
        campaign_names: Vec::new(),
    }
}

fn account_model() -> Model {
    let mut model = booted(
        Route::Admin(AdminRoute::Account {
            account_id: "00ff".to_owned(),
        }),
        operator(true),
    );
    deliver(
        &mut model,
        Reply::AdminAccount(Ok(AdminAccountDetail {
            account: account_row(),
            ledger: Vec::new(),
            refundable_lots: vec![RefundableLot {
                lot_id: "lot1".to_owned(),
                purchased_at_ms: 1,
                refundable_cents: 1500,
                card_suffix: "4242".to_owned(),
            }],
            email: None,
            created_at_ms: 0,
            terms_version: None,
            terms_accepted_at_ms: None,
            adult_attested_at_ms: None,
            closure_state: crate::wire::ClosureState::Open,
            sign_in_methods: Vec::new(),
            sessions: Vec::new(),
            play_sessions: Vec::new(),
            assets: Vec::new(),
            balance: None,
            campaign_funding: Vec::new(),
            disputes: Vec::new(),
            audit: Vec::new(),
            statement_totals: crate::wire::StatementTotals::default(),
            monthly: Vec::new(),
        })),
    );
    issue(&mut model, OperationSlot::Primary, "op-refund");
    issue(&mut model, OperationSlot::Secondary, "op-credit");
    model
}

fn choose_lot(model: &mut Model) {
    click(
        model,
        Intent::ChooseLot {
            lot_id: "lot1".to_owned(),
        },
    );
}

#[test]
fn a_customer_who_opens_the_console_is_told_it_is_not_theirs_and_nothing_loads() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::Admin(AdminRoute::Summary),
        offset_east_minutes: 0,
    });

    let effects = deliver(&mut model, Reply::Session(Ok(signed_in(true))));

    assert_eq!(model.screen, Screen::NotPermitted);
    assert!(calls(&effects).is_empty());
}

#[test]
fn an_operator_without_the_second_step_gets_no_data_and_is_told_to_sign_in_again() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::Admin(AdminRoute::Summary),
        offset_east_minutes: 0,
    });

    let effects = deliver(&mut model, Reply::Session(Ok(operator(false))));

    assert_eq!(model.screen, Screen::NeedsSecondFactor);
    assert!(calls(&effects).is_empty());
}

#[test]
fn each_console_area_asks_for_its_own_data() {
    for (route, call) in [
        (AdminRoute::Summary, ApiCall::AdminSummary),
        (AdminRoute::Disputes, ApiCall::Disputes),
        (AdminRoute::Reconciliation, ApiCall::Reconciliation),
        (AdminRoute::Providers, ApiCall::Providers),
        (AdminRoute::Review, ApiCall::ReviewQueue),
        (AdminRoute::Audit, ApiCall::Audit),
    ] {
        let mut model = Model::default();
        model.update(Msg::Started {
            route: Route::Admin(route),
            offset_east_minutes: 0,
        });

        let effects = deliver(&mut model, Reply::Session(Ok(operator(true))));

        assert_eq!(first_call(&effects), Some(&call));
    }
}

#[test]
fn searching_accounts_needs_a_query_and_sends_it_trimmed() {
    let mut model = booted(Route::Admin(AdminRoute::Accounts), operator(true));

    let empty = submit(&mut model, Form::Search);
    type_into(&mut model, Field::SearchQuery, "  ana@example.test ");
    let sent = submit(&mut model, Form::Search);
    let again = submit(&mut model, Form::Search);

    assert!(calls(&empty).is_empty());
    assert_eq!(
        first_call(&sent),
        Some(&ApiCall::AdminAccounts {
            query: "ana@example.test".to_owned(),
            standing: None,
            verified: None,
            has_balance: None,
            sort: Some("last_activity".to_owned()),
        })
    );
    assert!(calls(&again).is_empty());
}

#[test]
fn a_refund_needs_a_lot_an_amount_within_it_and_a_reason() {
    let mut model = account_model();
    type_into(&mut model, Field::RefundAmount, "15.01");
    type_into(&mut model, Field::RefundReason, "Customer asked");

    let no_lot = submit(&mut model, Form::Refund);
    choose_lot(&mut model);
    let too_much = submit(&mut model, Form::Refund);
    type_into(&mut model, Field::RefundAmount, "15.00");
    type_into(&mut model, Field::RefundReason, "   ");
    let no_reason = submit(&mut model, Form::Refund);
    type_into(&mut model, Field::RefundReason, " Customer asked ");
    let sent = submit(&mut model, Form::Refund);

    assert!(calls(&no_lot).is_empty());
    assert!(calls(&too_much).is_empty());
    assert!(calls(&no_reason).is_empty());
    assert_eq!(
        first_call(&sent),
        Some(&ApiCall::Refund(RefundRequest {
            account_id: "00ff".to_owned(),
            lot_id: "lot1".to_owned(),
            amount_cents: 1500,
            reason: "Customer asked".to_owned(),
            operation: "op-refund".to_owned(),
        }))
    );
}

#[test]
fn a_reason_over_five_hundred_characters_is_refused() {
    let mut model = account_model();
    choose_lot(&mut model);
    type_into(&mut model, Field::RefundAmount, "1.00");
    type_into(&mut model, Field::RefundReason, &"x".repeat(501));

    submit(&mut model, Form::Refund);

    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::ReasonTooLong))
    );
}

#[test]
fn a_goodwill_credit_needs_an_amount_a_support_case_and_a_reason() {
    let mut model = account_model();
    type_into(&mut model, Field::CreditAmount, "5");
    type_into(&mut model, Field::CreditReason, "Outage");

    let no_case = submit(&mut model, Form::Goodwill);
    type_into(&mut model, Field::SupportCase, "CASE-12");
    let sent = submit(&mut model, Form::Goodwill);

    assert!(calls(&no_case).is_empty());
    assert_eq!(
        first_call(&sent),
        Some(&ApiCall::GoodwillCredit(GoodwillCreditRequest {
            account_id: "00ff".to_owned(),
            amount_cents: 500,
            reason: "Outage".to_owned(),
            support_case: "CASE-12".to_owned(),
            operation: "op-credit".to_owned(),
        }))
    );
}

#[test]
fn a_finished_refund_clears_the_form_reloads_the_account_and_asks_for_a_new_id() {
    let mut model = account_model();
    choose_lot(&mut model);
    type_into(&mut model, Field::RefundAmount, "1.00");
    type_into(&mut model, Field::RefundReason, "Customer asked");
    submit(&mut model, Form::Refund);

    let effects = deliver(
        &mut model,
        Reply::AdminAction(Ok(AdminActionView {
            audit_id: "au1".to_owned(),
            outcome: ActionOutcome::Pending,
        })),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::IssueOperation {
            slot: OperationSlot::Primary,
            ..
        }
    )));
    assert_eq!(
        calls(&effects),
        vec![&ApiCall::AdminAccount {
            account_id: "00ff".to_owned()
        }]
    );
    let Screen::Admin(AdminScreen::Account(screen)) = &model.screen else {
        panic!("account");
    };
    assert_eq!(screen.refund.reason, "");
    assert_eq!(
        screen.last.as_ref().map(|view| view.outcome),
        Some(ActionOutcome::Pending)
    );
}

fn dispute() -> DisputeRow {
    DisputeRow {
        dispute_id: "d1".to_owned(),
        account_id: "00ff".to_owned(),
        amount_cents: 2000,
        status: DisputeStatus::NeedsResponse,
        opened_at_ms: 1,
        evidence_due_at_ms: 2,
    }
}

fn open_evidence_form() -> Model {
    let mut model = booted(Route::Admin(AdminRoute::Disputes), operator(true));
    deliver(&mut model, Reply::Disputes(Ok(vec![dispute()])));
    click(
        &mut model,
        Intent::OpenAction(ActionTarget::DisputeEvidence {
            dispute_id: "d1".to_owned(),
        }),
    );
    issue(&mut model, OperationSlot::Primary, "op-evidence");
    model
}

#[test]
fn dispute_evidence_needs_a_reason_and_is_sent_for_that_dispute() {
    let mut model = open_evidence_form();

    let empty = submit(&mut model, Form::Action);
    type_into(
        &mut model,
        Field::ActionReason,
        "Receipt and usage log attached",
    );
    let sent = submit(&mut model, Form::Action);

    assert!(calls(&empty).is_empty());
    assert_eq!(
        first_call(&sent),
        Some(&ApiCall::DisputeEvidence {
            dispute_id: "d1".to_owned(),
            request: ReasonRequest {
                reason: "Receipt and usage log attached".to_owned(),
                operation: "op-evidence".to_owned(),
            },
        })
    );
}

#[test]
fn pausing_a_provider_sends_the_switch_and_reloads_the_list_afterwards() {
    let mut model = booted(Route::Admin(AdminRoute::Providers), operator(true));
    deliver(
        &mut model,
        Reply::Providers(Ok(vec![ProviderRow {
            provider_id: "p1".to_owned(),
            name: "Image provider".to_owned(),
            health: ProviderHealth::Degraded,
            error_rate_permille: 80,
            open_exposure_micro_dollars: 1_000,
        }])),
    );
    click(
        &mut model,
        Intent::OpenAction(ActionTarget::Provider {
            provider_id: "p1".to_owned(),
            switch: ProviderSwitch::Pause,
        }),
    );
    issue(&mut model, OperationSlot::Primary, "op-pause");
    type_into(&mut model, Field::ActionReason, "Error rate");

    let sent = submit(&mut model, Form::Action);
    let done = deliver(
        &mut model,
        Reply::AdminAction(Ok(AdminActionView {
            audit_id: "au2".to_owned(),
            outcome: ActionOutcome::Done,
        })),
    );

    assert!(matches!(
        first_call(&sent),
        Some(ApiCall::SwitchProvider {
            switch: ProviderSwitch::Pause,
            ..
        })
    ));
    assert_eq!(first_call(&done), Some(&ApiCall::Providers));
}

#[test]
fn a_review_decision_carries_the_decision_and_the_reason() {
    let mut model = booted(Route::Admin(AdminRoute::Review), operator(true));
    deliver(
        &mut model,
        Reply::ReviewQueue(Ok(vec![ReviewItem {
            item_id: "i1".to_owned(),
            kind: ReviewKind::UploadedPortrait,
            campaign_name: "The Drowned Lantern".to_owned(),
            submitted_at_ms: 1,
            review_deadline_ms: 72_000_000,
            flag: ReviewFlag::PossibleRealPerson,
        }])),
    );
    click(
        &mut model,
        Intent::OpenAction(ActionTarget::Review {
            item_id: "i1".to_owned(),
            decision: ReviewDecision::Reject,
        }),
    );
    issue(&mut model, OperationSlot::Primary, "op-review");
    type_into(&mut model, Field::ActionReason, "Looks like a real person");

    let sent = submit(&mut model, Form::Action);

    assert_eq!(
        first_call(&sent),
        Some(&ApiCall::DecideReview {
            item_id: "i1".to_owned(),
            request: ReviewDecisionRequest {
                decision: ReviewDecision::Reject,
                reason: "Looks like a real person".to_owned(),
                operation: "op-review".to_owned(),
            },
        })
    );
}

#[test]
fn a_refused_console_action_keeps_the_form_open_with_the_server_answer() {
    let mut model = open_evidence_form();
    type_into(&mut model, Field::ActionReason, "Evidence");
    submit(&mut model, Form::Action);

    deliver(
        &mut model,
        Reply::AdminAction(Err(ApiError::of(ErrorCode::Forbidden))),
    );

    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::of(ErrorCode::Forbidden))
    );
}
