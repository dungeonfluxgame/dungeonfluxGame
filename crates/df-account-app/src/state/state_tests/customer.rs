use super::*;
use crate::state::customer_flow::{parse_date_ms, statements_csv};
use crate::{
    money::MICRO_DOLLARS_PER_CENT,
    wire::{
        AllocateRequest, CampaignFunding, CheckoutCreated, CheckoutStatus, CheckoutView,
        CloseAccountRequest, ClosedView, CreateCheckoutRequest, CreditOffers, Direction,
        FundingView, FundsState, LineStatus, PacePreset, PresetOffer, Quantity, Refusal,
        SessionFunding, SetPaceRequest, SettingsView, StatementCategory, StatementLine,
        StatementsView,
    },
    wire_campaign::{
        CampaignCard, CampaignRole, DashboardView, FundingNote, HostCreditView, LobbyView,
    },
};

fn offers() -> CreditOffers {
    CreditOffers {
        is_test_mode: true,
        preset_amounts_cents: vec![1000, 2000, 5000],
        custom_minimum_cents: 500,
        custom_maximum_cents: 50_000,
    }
}

fn credit_form() -> Model {
    let mut model = booted(Route::AddCredit, signed_in(true));
    deliver(&mut model, Reply::CreditOffers(Ok(offers())));
    issue(&mut model, OperationSlot::Primary, "op-credit");
    model
}

fn funding(can_choose_pace: bool, with_session: bool) -> FundingView {
    FundingView {
        presets: vec![
            PresetOffer {
                preset: PacePreset::Story,
                ceiling_cents_per_hour: Some(300),
            },
            PresetOffer {
                preset: PacePreset::Custom,
                ceiling_cents_per_hour: None,
            },
        ],
        is_proposed_pricing: true,
        custom_minimum_cents_per_hour: 100,
        custom_maximum_cents_per_hour: 10_000,
        campaigns: vec![CampaignFunding {
            campaign_id: "0c".to_owned(),
            name: "The Drowned Lantern".to_owned(),
            can_choose_pace,
            preset: PacePreset::Immersive,
            ceiling_cents_per_hour: 800,
            session: with_session.then(|| SessionFunding {
                session_id: "5e".to_owned(),
                pot_available_micro_dollars: 24_800_000,
                open_holds_micro_dollars: 700_000,
                spent_micro_dollars: 1_000_000,
                own_allocation_micro_dollars: 10_000_000,
                own_remaining_micro_dollars: 9_000_000,
                runway_at_ceiling_minutes: Some(180),
                runway_at_recent_rate_minutes: None,
                funds: FundsState::Funded,
                contributions_visible: true,
            }),
        }],
    }
}

fn funding_model(can_choose_pace: bool, with_session: bool) -> Model {
    let mut model = booted(Route::Funding, signed_in(true));
    deliver(
        &mut model,
        Reply::Funding(Ok(funding(can_choose_pace, with_session))),
    );
    model
}

#[test]
fn the_dashboard_asks_for_balance_campaigns_and_a_remembered_invitation_together() {
    let model_effects = {
        let mut model = Model::default();
        model.update(Msg::Started {
            route: Route::Dashboard,
            offset_east_minutes: 0,
        });
        deliver(&mut model, Reply::Session(Ok(signed_in(true))))
    };

    let wanted = [ApiCall::Overview, ApiCall::Dashboard, ApiCall::JoinPending];
    for call in &wanted {
        assert!(calls(&model_effects).contains(&call), "{call:?}");
    }
}

#[test]
fn the_dashboard_loads_the_quote_for_its_live_game_before_play_now() {
    let mut model = booted(Route::Dashboard, signed_in(true));
    let effects = deliver(
        &mut model,
        Reply::Dashboard(Ok(DashboardView {
            campaigns: vec![CampaignCard {
                campaign_id: "0c".to_owned(),
                name: "The Drowned Lantern".to_owned(),
                role: CampaignRole::Host,
                host_name: "Ana".to_owned(),
                party_size: 4,
                last_played_at_ms: None,
                last_saved_at_ms: None,
                live_session_id: Some("5e".to_owned()),
                is_archived: false,
            }],
            can_create: true,
        })),
    );
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::Lobby {
            session_id: "5e".to_owned()
        })
    );

    deliver(
        &mut model,
        Reply::Lobby(Ok(LobbyView {
            session_id: "5e".to_owned(),
            campaign_id: "0c".to_owned(),
            campaign_name: "The Drowned Lantern".to_owned(),
            can_manage: true,
            start_blockers: Vec::new(),
            members: Vec::new(),
            funding_note: FundingNote::HostCredit(HostCreditView {
                price_version: [1; 16],
                hourly_micro_dollars: 8_000_000,
                available_micro_dollars: 50_000_000,
                cap_micro_dollars: 25_000_000,
                terms_version: 1,
                consented_at_ms: None,
                is_fundable: true,
            }),
            has_started: false,
            poll_seconds: 3,
        })),
    );
    let Screen::Dashboard(screen) = model.screen else {
        panic!("dashboard remains open while its game quote loads");
    };
    assert!(matches!(screen.resume_lobby, Remote::Ready(_)));
}

#[test]
fn a_custom_amount_outside_the_offered_range_is_refused_before_any_request() {
    let mut model = credit_form();
    click(&mut model, Intent::ChooseCustomAmount);
    type_into(&mut model, Field::CustomAmount, "4.99");

    let effects = submit(&mut model, Form::Credit);

    assert!(calls(&effects).is_empty());
    assert_eq!(
        crate::copy::refused_form_error(&model),
        Some(ApiError::refused(Refusal::AmountOutOfRange))
    );
}

#[test]
fn choosing_a_preset_then_continuing_creates_one_checkout_with_its_operation_id() {
    let mut model = credit_form();
    click(&mut model, Intent::ChooseAmount(2000));

    let effects = submit(&mut model, Form::Credit);
    let again = submit(&mut model, Form::Credit);

    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::CreateCheckout(CreateCheckoutRequest {
            amount_cents: 2000,
            operation: "op-credit".to_owned(),
        }))
    );
    assert!(calls(&again).is_empty());
}

#[test]
fn nothing_chosen_is_refused_without_a_request() {
    let mut model = credit_form();

    assert!(calls(&submit(&mut model, Form::Credit)).is_empty());
}

#[test]
fn the_browser_leaves_only_for_our_own_pages_or_the_processors_checkout() {
    for (url, leaves) in [
        ("https://checkout.stripe.com/c/pay/cs_test_1", true),
        ("/app/credit/return?checkout=0a", true),
        ("//evil.example/app", false),
        ("https://evil.example/", false),
        ("https://checkout.stripe.com.evil.example/", false),
        ("javascript:alert(1)", false),
    ] {
        let mut model = credit_form();
        click(&mut model, Intent::ChooseAmount(1000));
        submit(&mut model, Form::Credit);

        let effects = deliver(
            &mut model,
            Reply::CheckoutCreated(Ok(CheckoutCreated {
                checkout_id: "0a".to_owned(),
                redirect_url: url.to_owned(),
            })),
        );

        assert_eq!(
            has_effect(&effects, &Effect::Leave(url.to_owned())),
            leaves,
            "{url}"
        );
    }
}

#[test]
fn a_checkout_whose_answer_never_came_says_to_look_before_paying_again() {
    let mut model = credit_form();
    click(&mut model, Intent::ChooseAmount(1000));
    submit(&mut model, Form::Credit);

    deliver(
        &mut model,
        Reply::CheckoutCreated(Err(ApiError::of(ErrorCode::UnknownOutcome))),
    );

    let error = crate::copy::refused_form_error(&model);
    assert_eq!(error, Some(ApiError::of(ErrorCode::UnknownOutcome)));
    assert!(
        crate::copy::problem(ApiError::of(ErrorCode::UnknownOutcome))
            .fix
            .contains("so nothing happens twice")
    );
}

#[test]
fn the_payment_return_waits_with_growing_delays_and_then_stops_asking() {
    let mut model = booted(
        Route::CreditReturn {
            checkout: Some("0a".to_owned()),
        },
        signed_in(true),
    );
    let pending = || {
        Reply::Checkout(Ok(CheckoutView {
            checkout_id: "0a".to_owned(),
            amount_cents: 2000,
            status: CheckoutStatus::Pending,
        }))
    };
    let mut delays = Vec::new();
    for _ in 0..CHECKOUT_POLL_DELAYS_MS.len() {
        for effect in deliver(&mut model, pending()) {
            if let Effect::After { delay_ms, .. } = effect {
                delays.push(delay_ms);
            }
        }
    }

    assert_eq!(delays, CHECKOUT_POLL_DELAYS_MS.to_vec());
    assert!(deliver(&mut model, pending()).is_empty());
    let Screen::CreditReturn(screen) = &model.screen else {
        panic!("payment return");
    };
    assert!(screen.has_stopped_waiting());
}

#[test]
fn a_paid_checkout_stops_the_polling_and_the_timer_asks_again_only_while_waiting() {
    let mut model = booted(
        Route::CreditReturn {
            checkout: Some("0a".to_owned()),
        },
        signed_in(true),
    );

    let effects = deliver(
        &mut model,
        Reply::Checkout(Ok(CheckoutView {
            checkout_id: "0a".to_owned(),
            amount_cents: 2000,
            status: CheckoutStatus::Paid,
        })),
    );
    let timer = model.update(Msg::TimerFired {
        generation: model.generation,
    });

    assert!(effects.is_empty());
    assert_eq!(
        first_call(&timer),
        Some(&ApiCall::Checkout {
            checkout_id: "0a".to_owned()
        })
    );
}

#[test]
fn only_the_host_can_open_the_pace_form() {
    let mut member = funding_model(false, false);
    let mut host = funding_model(true, false);

    click(
        &mut member,
        Intent::EditPace {
            campaign_id: "0c".to_owned(),
        },
    );
    click(
        &mut host,
        Intent::EditPace {
            campaign_id: "0c".to_owned(),
        },
    );

    let (Screen::Funding(member_screen), Screen::Funding(host_screen)) =
        (&member.screen, &host.screen)
    else {
        panic!("funding screens");
    };
    assert!(member_screen.edit.is_none());
    assert!(host_screen.edit.is_some());
}

#[test]
fn a_custom_pace_needs_a_ceiling_inside_the_offered_range() {
    let mut model = funding_model(true, false);
    click(
        &mut model,
        Intent::EditPace {
            campaign_id: "0c".to_owned(),
        },
    );
    issue(&mut model, OperationSlot::Primary, "op-pace");
    type_into(&mut model, Field::PacePreset, "custom");
    type_into(&mut model, Field::CustomCeiling, "250");

    let too_high = submit(&mut model, Form::Pace);
    type_into(&mut model, Field::CustomCeiling, "12.50");
    let fine = submit(&mut model, Form::Pace);

    assert!(calls(&too_high).is_empty());
    assert_eq!(
        first_call(&fine),
        Some(&ApiCall::SetPace(SetPaceRequest {
            campaign_id: "0c".to_owned(),
            preset: PacePreset::Custom,
            custom_ceiling_cents_per_hour: Some(1250),
            operation: "op-pace".to_owned(),
        }))
    );
}

#[test]
fn a_preset_pace_sends_no_custom_ceiling_and_the_saved_table_is_remembered() {
    let mut model = funding_model(true, false);
    click(
        &mut model,
        Intent::EditPace {
            campaign_id: "0c".to_owned(),
        },
    );
    issue(&mut model, OperationSlot::Primary, "op-pace");
    type_into(&mut model, Field::PacePreset, "story");

    let effects = submit(&mut model, Form::Pace);
    deliver(&mut model, Reply::PaceSaved(Ok(funding(true, false))));

    let Some(ApiCall::SetPace(request)) = first_call(&effects) else {
        panic!("a pace request");
    };
    assert_eq!(request.custom_ceiling_cents_per_hour, None);
    let Screen::Funding(screen) = &model.screen else {
        panic!("funding");
    };
    assert_eq!(screen.saved_campaign.as_deref(), Some("0c"));
    assert!(screen.edit.is_none());
}

#[test]
fn adding_credit_to_a_table_needs_an_amount_and_the_consent_box() {
    let mut model = funding_model(false, true);
    let effects = click(
        &mut model,
        Intent::EditAllocation {
            session_id: "5e".to_owned(),
        },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::IssueOperation {
            slot: OperationSlot::Secondary,
            ..
        }
    )));
    issue(&mut model, OperationSlot::Secondary, "op-allocate");

    type_into(&mut model, Field::AllocateAmount, "10");
    let without_consent = submit(&mut model, Form::Allocate);
    tick(&mut model, Field::AllocateConsent, true);
    let with_consent = submit(&mut model, Form::Allocate);

    assert!(calls(&without_consent).is_empty());
    assert_eq!(
        first_call(&with_consent),
        Some(&ApiCall::Allocate(AllocateRequest {
            session_id: "5e".to_owned(),
            amount_cents: 1000,
            consents_to_host_spending: true,
            operation: "op-allocate".to_owned(),
        }))
    );
}

#[test]
fn an_allocation_for_a_session_the_person_does_not_have_opens_nothing() {
    let mut model = funding_model(false, true);

    let effects = click(
        &mut model,
        Intent::EditAllocation {
            session_id: "ffff".to_owned(),
        },
    );

    assert!(effects.is_empty());
}

fn line(index: u64) -> StatementLine {
    StatementLine {
        line_id: format!("{index:x}"),
        at_ms: index,
        category: StatementCategory::TableTime,
        campaign_name: None,
        quantity: Quantity::None,
        direction: Direction::Charge,
        amount_micro_dollars: MICRO_DOLLARS_PER_CENT,
        status: LineStatus::Settled,
    }
}

#[test]
fn older_statement_pages_append_and_the_screen_holds_at_most_the_bound() {
    let mut model = booted(Route::Statements, signed_in(true));
    deliver(
        &mut model,
        Reply::Statements {
            is_continuation: false,
            result: Ok(StatementsView {
                lines: (0..300).map(line).collect(),
                next_cursor: Some("c1".to_owned()),
                totals: Default::default(),
                monthly: Vec::new(),
                per_campaign: Vec::new(),
            }),
        },
    );

    let effects = click(&mut model, Intent::LoadOlderStatements);
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::Statements {
            cursor: Some("c1".to_owned()),
            filter: crate::wire::StatementFilter::All,
            from_ms: None,
            to_ms: None,
        })
    );
    assert!(calls(&click(&mut model, Intent::LoadOlderStatements)).is_empty());
    deliver(
        &mut model,
        Reply::Statements {
            is_continuation: true,
            result: Ok(StatementsView {
                lines: (300..700).map(line).collect(),
                next_cursor: Some("c2".to_owned()),
                totals: Default::default(),
                monthly: Vec::new(),
                per_campaign: Vec::new(),
            }),
        },
    );

    let Screen::Statements(screen) = &model.screen else {
        panic!("statements");
    };
    assert_eq!(
        screen.lines.ready().map(Vec::len),
        Some(MAXIMUM_STATEMENT_LINES)
    );
    assert_eq!(screen.next_cursor, None, "no more pages once full");
}

#[test]
fn a_failed_older_page_keeps_the_lines_already_shown() {
    let mut model = booted(Route::Statements, signed_in(true));
    deliver(
        &mut model,
        Reply::Statements {
            is_continuation: false,
            result: Ok(StatementsView {
                lines: vec![line(1)],
                next_cursor: Some("c1".to_owned()),
                totals: Default::default(),
                monthly: Vec::new(),
                per_campaign: Vec::new(),
            }),
        },
    );
    click(&mut model, Intent::LoadOlderStatements);

    deliver(
        &mut model,
        Reply::Statements {
            is_continuation: true,
            result: Err(ApiError::of(ErrorCode::Unavailable)),
        },
    );

    let Screen::Statements(screen) = &model.screen else {
        panic!("statements");
    };
    assert_eq!(screen.lines.ready().map(Vec::len), Some(1));
    assert!(matches!(screen.more, Submit::Refused(_)));
}

fn settings_model() -> Model {
    let mut model = booted(Route::Settings, signed_in(true));
    deliver(
        &mut model,
        Reply::Settings(Ok(SettingsView {
            email: "ana@example.test".to_owned(),
            display_name: Some("Ana".to_owned()),
            sessions: Vec::new(),
            closure_blockers: Vec::new(),
            created_at_ms: 0,
            is_verified: true,
            terms_version: None,
            terms_accepted_at_ms: None,
            adult_attested_at_ms: None,
            closure_state: crate::wire::ClosureState::Open,
            sign_in_methods: Vec::new(),
            email_preferences: Default::default(),
            export_status: crate::wire::ExportStatus::NotRequested,
        })),
    );
    issue(&mut model, OperationSlot::Primary, "op-profile");
    issue(&mut model, OperationSlot::Secondary, "op-close");
    model
}

#[test]
fn the_profile_form_starts_from_the_saved_name_and_refreshes_the_session_after_saving() {
    let mut model = settings_model();
    let Screen::Settings(screen) = &model.screen else {
        panic!("settings");
    };
    assert_eq!(screen.display_name, "Ana");
    type_into(&mut model, Field::DisplayName, "Ana B");

    let effects = submit(&mut model, Form::Profile);
    let saved = deliver(
        &mut model,
        Reply::ProfileSaved(Ok(crate::wire::Accepted { accepted: true })),
    );

    assert!(matches!(
        first_call(&effects),
        Some(ApiCall::SaveProfile(_))
    ));
    assert!(calls(&saved).contains(&&ApiCall::Session));
}

#[test]
fn closing_the_account_needs_the_confirmation_box_and_then_the_password() {
    let mut model = settings_model();
    type_into(&mut model, Field::ClosePassword, "secret");

    let unconfirmed = submit(&mut model, Form::Close);
    tick(&mut model, Field::CloseUnderstood, true);
    let confirmed = submit(&mut model, Form::Close);

    assert!(calls(&unconfirmed).is_empty());
    assert_eq!(
        first_call(&confirmed),
        Some(&ApiCall::CloseAccount(CloseAccountRequest {
            password: "secret".to_owned(),
            operation: "op-close".to_owned(),
        }))
    );
}

#[test]
fn a_closed_account_ends_signed_out_with_the_deletion_date_in_a_notice() {
    let mut model = settings_model();
    type_into(&mut model, Field::ClosePassword, "secret");
    tick(&mut model, Field::CloseUnderstood, true);
    submit(&mut model, Form::Close);

    let effects = deliver(
        &mut model,
        Reply::Closed(Ok(ClosedView {
            delete_after_ms: 1_900_000_000_000,
        })),
    );

    assert_eq!(
        model.notice,
        Some(Notice::AccountClosed {
            delete_after_ms: 1_900_000_000_000
        })
    );
    assert_eq!(effects.first(), Some(&Effect::CancelPending));
    assert_eq!(first_call(&effects), Some(&ApiCall::Session));
}

#[test]
fn a_refused_close_clears_the_password_so_it_is_not_kept_on_screen() {
    let mut model = settings_model();
    type_into(&mut model, Field::ClosePassword, "wrong");
    tick(&mut model, Field::CloseUnderstood, true);
    submit(&mut model, Form::Close);

    deliver(
        &mut model,
        Reply::Closed(Err(ApiError::refused(Refusal::WrongPassword))),
    );

    let Screen::Settings(screen) = &model.screen else {
        panic!("settings");
    };
    assert_eq!(screen.close_password, "");
}

#[test]
fn signing_out_the_other_devices_is_one_request_at_a_time() {
    let mut model = settings_model();

    let first = click(&mut model, Intent::RevokeOtherSessions);
    let second = click(&mut model, Intent::RevokeOtherSessions);

    assert_eq!(first_call(&first), Some(&ApiCall::RevokeOtherSessions));
    assert!(calls(&second).is_empty());
}

#[test]
fn the_dashboard_loads_stats_assets_and_latest_topups() {
    let mut model = Model::default();
    model.update(Msg::Started {
        route: Route::Dashboard,
        offset_east_minutes: 0,
    });
    let effects = deliver(&mut model, Reply::Session(Ok(signed_in(true))));
    let sent = calls(&effects);
    assert!(sent.iter().any(|call| **call == ApiCall::AccountStats));
    assert!(sent.iter().any(|call| **call
        == ApiCall::Assets {
            kind: None,
            campaign: None,
            cursor: None,
        }));
}

#[test]
fn play_sessions_and_assets_pages_load_and_page_older() {
    let mut model = booted(Route::PlaySessions, signed_in(true));
    deliver(
        &mut model,
        Reply::PlaySessions {
            is_continuation: false,
            result: Ok(crate::wire::PlaySessionsView {
                sessions: vec![crate::wire::PlaySessionRow {
                    session_id: "s1".to_owned(),
                    campaign_id: "c1".to_owned(),
                    campaign_name: "Keep".to_owned(),
                    started_at_ms: 1,
                    ended_at_ms: Some(2),
                    duration_seconds: 60,
                    players: 3,
                    spent_micro_dollars: 1,
                }],
                next_cursor: Some("c1".to_owned()),
            }),
        },
    );
    let effects = click(&mut model, Intent::LoadOlderPlaySessions);
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::PlaySessions {
            cursor: Some("c1".to_owned())
        })
    );

    let mut model = booted(Route::Assets, signed_in(true));
    let effects = click(
        &mut model,
        Intent::SetAssetKind(Some(crate::wire::AssetKind::Music)),
    );
    assert_eq!(
        first_call(&effects),
        Some(&ApiCall::Assets {
            kind: Some(crate::wire::AssetKind::Music),
            campaign: None,
            cursor: None,
        })
    );
}

#[test]
fn the_change_email_and_password_forms_validate_before_sending() {
    let mut model = settings_model();
    // Password mismatch is refused locally.
    type_into(&mut model, Field::CurrentPassword, "the current password");
    type_into(&mut model, Field::Password, "a new long passphrase");
    type_into(
        &mut model,
        Field::Confirmation,
        "a different long passphrase",
    );
    assert!(calls(&submit(&mut model, Form::ChangePassword)).is_empty());
    // Matching passwords send the change.
    type_into(&mut model, Field::Confirmation, "a new long passphrase");
    let sent = submit(&mut model, Form::ChangePassword);
    assert!(matches!(
        first_call(&sent),
        Some(ApiCall::ChangePassword(_))
    ));
    // An empty new email is refused locally.
    let mut model = settings_model();
    let sent = submit(&mut model, Form::ChangeEmail);
    assert!(calls(&sent).is_empty());
    type_into(&mut model, Field::NewEmail, "new@example.test");
    type_into(&mut model, Field::EmailPassword, "the current password");
    let sent = submit(&mut model, Form::ChangeEmail);
    assert!(matches!(first_call(&sent), Some(ApiCall::ChangeEmail(_))));
}

#[test]
fn statements_csv_and_date_parsing_are_plain_and_bounded() {
    assert_eq!(parse_date_ms("2026-01-31", false), Some(1_769_817_600_000));
    assert_eq!(parse_date_ms("not a date", false), None);
    assert_eq!(parse_date_ms("", false), None);
    let csv = statements_csv(&[line(1)]);
    assert!(csv.starts_with("when,what,campaign,direction,amount_micro_dollars,status\n"));
    assert!(csv.contains(",charge,"));
}
