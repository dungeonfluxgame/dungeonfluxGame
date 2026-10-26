use super::*;
use crate::{state::Submit, wire_campaign::HostCreditView};

fn screen(note: FundingNote) -> LobbyScreen {
    LobbyScreen {
        session_id: "session".into(),
        lobby: Remote::Ready(LobbyView {
            session_id: "session".into(),
            campaign_id: "campaign".into(),
            campaign_name: "Friday table".into(),
            can_manage: true,
            start_blockers: vec![StartBlocker::FundingNotReady],
            members: Vec::new(),
            displays: Vec::new(),
            funding_note: note,
            has_started: false,
            poll_seconds: 3,
        }),
        operation: Remote::Ready("operation".into()),
        action: Submit::Idle,
        pairing_code: String::new(),
        lookup: None,
        consent_checked: false,
    }
}

fn quote() -> HostCreditView {
    HostCreditView {
        price_version: [7; 16],
        hourly_micro_dollars: 3_000_000,
        available_micro_dollars: 10_000_000,
        cap_micro_dollars: 3_000_000,
        terms_version: 1,
        consented_at_ms: None,
        is_fundable: false,
    }
}

#[test]
fn fundinghook_host_sees_the_customer_price_credit_and_unchecked_consent() {
    let tree = lobby(&screen(FundingNote::HostCredit(quote())), 0);
    let text = tree.text_content();
    assert!(text.contains("Table price: $3.00 per table-hour, at most. Available credit: $10.00."));
    assert!(text.contains("Use my credit for this session, up to $3.00"));
    assert!(text.contains("Approve use of your credit before starting."));
    let elements = tree.elements();
    let checkbox = elements
        .iter()
        .find(|e| e.field == Some(Field::FundingChoice))
        .unwrap();
    assert!(
        !checkbox
            .attributes
            .iter()
            .any(|(name, _)| *name == "checked")
    );
    let start = elements
        .iter()
        .find(|e| e.on_click == Some(Intent::StartSession))
        .unwrap();
    assert!(start.attributes.iter().any(|(name, _)| *name == "disabled"));
    assert!(!text.contains("supplier"));
    assert!(!text.contains("markup"));
}

#[test]
fn fundinghook_players_have_no_credit_control_or_private_host_balance() {
    let tree = lobby(&screen(FundingNote::WaitingForSponsor), 0);
    assert!(
        !tree
            .elements()
            .iter()
            .any(|e| e.field == Some(Field::FundingChoice))
    );
    assert!(!tree.text_content().contains("Available credit"));
}

#[test]
fn fundinghook_insufficient_credit_has_a_plain_start_blocker() {
    let mut quote = quote();
    quote.available_micro_dollars = 2_999_999;
    let text = lobby(&screen(FundingNote::HostCredit(quote)), 0).text_content();
    assert!(text.contains("Add credit to cover the first hour before starting."));
}
