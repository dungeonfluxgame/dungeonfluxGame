use df_protocol::common::{
    DisplaySpendView, ExpensiveWorkQuote, HostSpendView, NonPayingSpendView, PayerSpendView,
    SessionSpendStatus, SessionSpendView, expensive_work_quote::PayerScope,
    session_spend_view::Role,
};
use prost::Message;

#[test]
fn host_spend_view_round_trips_low_balance_controls_and_quoted_cost() {
    let view = SessionSpendView {
        role: Some(Role::Host(HostSpendView {
            ceiling_micro_dollars: 2_400_000_000,
            held_micro_dollars: 70_000_000,
            spent_micro_dollars: 800_000_000,
            remaining_micro_dollars: 1_530_000_000,
            pace_micro_dollars_per_hour: 800_000_000,
            consented_minimum_pace_micro_dollars_per_hour: 200_000_000,
            consented_maximum_pace_micro_dollars_per_hour: 1_200_000_000,
            status: SessionSpendStatus::LowBalance as i32,
            may_pause_expensive_media: true,
            may_change_pace: true,
            may_stop_session: true,
            paid_work_stopped: false,
            quote: Some(ExpensiveWorkQuote {
                output_description: "One cinematic scene".into(),
                quoted_cost_micro_dollars: 250_000_000,
                maximum_cost_micro_dollars: 300_000_000,
                payer_scope: PayerScope::ConsentedContributors as i32,
                expires_at_unix_millis: 1_800_000_000_000,
                host_confirmation_required: true,
                payer_approval_required: true,
            }),
        })),
    };

    assert_eq!(
        SessionSpendView::decode(view.encode_to_vec().as_slice()).unwrap(),
        view
    );
}

#[test]
fn exhausted_spend_view_stops_paid_work_without_representing_game_state() {
    let view = SessionSpendView {
        role: Some(Role::Host(HostSpendView {
            status: SessionSpendStatus::Exhausted as i32,
            paid_work_stopped: true,
            ..Default::default()
        })),
    };
    let Some(Role::Host(host)) = view.role else {
        panic!("host spend view must be retained");
    };
    assert_eq!(host.status, SessionSpendStatus::Exhausted as i32);
    assert!(host.paid_work_stopped);
}

#[test]
fn payer_projection_contains_only_its_own_authorization_and_share() {
    let view = SessionSpendView {
        role: Some(Role::Payer(PayerSpendView {
            own_remaining_authorization_micro_dollars: 450_000_000,
            own_share_micro_dollars: 125_000_000,
            status: SessionSpendStatus::Funded as i32,
        })),
    };
    let Some(Role::Payer(payer)) = SessionSpendView::decode(view.encode_to_vec().as_slice())
        .unwrap()
        .role
    else {
        panic!("payer spend view must be retained");
    };
    assert_eq!(payer.own_remaining_authorization_micro_dollars, 450_000_000);
    assert_eq!(payer.own_share_micro_dollars, 125_000_000);
}

#[test]
fn nonpaying_player_and_display_receive_no_payer_amounts() {
    let nonpaying = SessionSpendView {
        role: Some(Role::NonPayingPlayer(NonPayingSpendView {
            status: SessionSpendStatus::LowBalance as i32,
        })),
    };
    let display = SessionSpendView {
        role: Some(Role::Display(DisplaySpendView {
            status: SessionSpendStatus::LowBalance as i32,
            paid_work_stopped: false,
        })),
    };

    assert!(matches!(nonpaying.role, Some(Role::NonPayingPlayer(_))));
    assert!(matches!(display.role, Some(Role::Display(_))));
    assert_eq!(
        SessionSpendView::decode(nonpaying.encode_to_vec().as_slice())
            .unwrap()
            .encode_to_vec(),
        nonpaying.encode_to_vec()
    );
    assert_eq!(
        SessionSpendView::decode(display.encode_to_vec().as_slice())
            .unwrap()
            .encode_to_vec(),
        display.encode_to_vec()
    );
}
